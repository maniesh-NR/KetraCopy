//! LAN transfer subcommands: `ketra peers`, `ketra send`, `ketra receive`.
//!
//! Everything rides on `ketra_core::lan` — the std-only LocalSend v2 +
//! Ketra accelerated extension implementation (mDNS discovery, HTTP/1.1,
//! end-to-end SHA-256). `peers` is a one-shot mDNS browse; `send` and
//! `receive` block until the transfer completes / the user interrupts.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ketra_core::lan::client::{send_files, SendConfig, SendReport};
use ketra_core::lan::server::{discover, ReceiverConfig, ReceiverHooks, Receiver};
use ketra_core::lan::{Mdns};

use crate::sig;

fn cancel_flag() -> Arc<AtomicBool> {
    sig::install();
    Arc::new(AtomicBool::new(false))
}

// ---------- `ketra peers` ----------

pub fn peers_cmd(wait_secs: u64) -> i32 {
    let mdns = match Mdns::spawn() {
        Some(m) => m,
        None => {
            eprintln!("ketra: mDNS: cannot bind :5353 (another mDNS daemon owns it, or no network)");
            return 1;
        }
    };
    eprintln!("ketra: browsing _localsend._tcp.local. / _ketra._tcp.local. for {wait_secs}s…");
    let peers = discover(Some(&mdns), Duration::from_secs(wait_secs.max(1)));
    // The same device announces on both `_localsend._tcp` and `_ketra._tcp`;
    // list it once (dedupe by fingerprint + address).
    let mut seen: Vec<String> = Vec::new();
    let peers: Vec<_> = peers
        .into_iter()
        .filter(|p| {
            let key = format!("{}|{}|{}", p.fingerprint().unwrap_or_default(), p.ip, p.port);
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        })
        .collect();
    if peers.is_empty() {
        println!("no LocalSend/Ketra peers found on the LAN (waited {wait_secs}s)");
        return 0;
    }
    println!("{:<22} {:<22} {:<9} {:<6} {}", "ALIAS", "ADDRESS", "SERVICE", "KETRA", "FINGERPRINT");
    for p in &peers {
        println!(
            "{:<22} {:<22} {:<9} {:<6} {}",
            truncate(&p.alias(), 22),
            format!("{}:{}", p.ip, p.port),
            if p.service.starts_with("_ketra") { "ketra" } else { "v2" },
            if p.is_ketra() { "yes" } else { "no" },
            p.fingerprint().unwrap_or_default(),
        );
    }
    0
}

// ---------- `ketra send` ----------

pub fn send_cmd(
    files: Vec<PathBuf>,
    peer_spec: String,
    dest_dir: Option<String>,
    parallel: u8,
    chunk_size: u64,
    checksum: String,
    no_manifest: bool,
) -> i32 {
    let _ = chunk_size;
    let _ = checksum;
    let cancel = cancel_flag();
    let mdns = match Mdns::spawn() {
        Some(m) => m,
        None => {
            eprintln!("ketra: mDNS: cannot bind :5353");
            return 1;
        }
    };
    eprintln!("ketra: discovering peers for 3s…");
    let peers = discover(Some(&mdns), Duration::from_secs(3));
    let wanted = peer_spec.to_ascii_lowercase();
    let peer = peers.iter().find(|p| {
        p.alias().to_ascii_lowercase() == wanted
            || p.instance.to_ascii_lowercase() == wanted
            || p.fingerprint().map(|f| f.to_ascii_lowercase() == wanted).unwrap_or(false)
            || format!("{}:{}", p.ip, p.port) == peer_spec
            || p.ip == peer_spec
    });
    let Some(peer) = peer else {
        eprintln!("ketra: peer '{peer_spec}' not found on the LAN (run `ketra peers`)");
        return 1;
    };
    let addr = format!("{}:{}", peer.ip, peer.port);
    eprintln!("ketra: sending to '{}' ({}){}", peer.alias(), addr,
        if peer.is_ketra() { " [ketra accelerated]" } else { " [localsend v2]" });

    let cfg = SendConfig {
        dest_dir_hint: dest_dir.unwrap_or_default(),
        parallel: (parallel.max(1)) as u32,
        verify: true,
        limit_bps: None,
        timeout: Duration::from_secs(30),
        ketra_mode: true,
    };

    let last = Mutex::new(Instant::now());
    let mut report: Option<SendReport> = None;
    let mut send_err: Option<String> = None;
    {
        let cancel_ref: &AtomicBool = &cancel;
        let result = send_files(&addr, &files, &cfg, cancel_ref, &mut |name: &str, delta: u64, size: u64| {
            if sig::was_interrupted() {
                // The engine's cancel flag is shared; just let progress flow.
            }
            let mut l = last.lock().unwrap();
            if l.elapsed() >= Duration::from_millis(200) {
                *l = Instant::now();
                eprint!("\r    {} {}  ", name, pct(delta, size));
                let _ = std::io::Write::flush(&mut std::io::stderr());
            }
            let _ = delta;
        });
        match result {
            Ok(r) => report = Some(r),
            Err(e) => send_err = Some(e),
        }
    }

    if let Some(e) = send_err {
        eprintln!("\nketra: send: {e}");
        return 1;
    }
    let r = report.unwrap();
    eprintln!();
    println!("  {} file(s) sent, {} verified, {} failed, {} in {:.1}s (streams used: {})",
        r.files_sent, r.files_verified, r.files_failed, human(r.bytes), r.duration.as_secs_f64(), r.max_streams_used);
    for (name, status, size) in &r.files {
        println!("    {:<7} {} ({})", status, name, human(*size));
    }
    if r.files_failed > 0 { 1 } else { 0 }
}

// ---------- `ketra receive` ----------

pub fn receive_cmd(dest_dir: PathBuf, port: u16, auto_accept: bool, alias: String, _model: String) -> i32 {
    let cancel = cancel_flag();
    let cfg = ReceiverConfig {
        alias: alias.clone(),
        dest_dir: dest_dir.clone(),
        port,
        auto_accept,
        verify_sha256: true,
        announce: true,
        max_streams: 4,
    };
    let hooks = ReceiverHooks {
        on_request: Box::new(|req| {
            let total: u64 = req.files.iter().map(|f| f.size).sum();
            if std::env::var("KETRA_AUTO_ACCEPT").map(|v| v == "1").unwrap_or(false) {
                return true;
            }
            if std::io::stdin().is_terminal() {
                print!(
                    "accept {} file(s) ({}) from {}? [Y/n] > ",
                    req.files.len(),
                    human(total),
                    req.info.alias,
                );
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                    return true;
                }
                let t = line.trim().to_ascii_lowercase();
                !(t == "n" || t == "no")
            } else {
                true
            }
        }),
        on_file_done: Box::new(|f| {
            eprintln!("\r  [{}] {} ({}){}", if f.verified { "✓" } else { "?" }, f.file_name, human(f.size),
                if f.verified { "" } else { " unverified" });
        }),
        on_progress: Box::new(|name, delta, size| {
            let _ = delta;
            eprint!("\r    {} {}  ", name, pct_from(size, name.len()));
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }),
        on_message: Box::new(|m| eprintln!("ketra: {m}")),
    };
    let server = match Receiver::start(cfg, hooks) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ketra: receive: {e}");
            return 1;
        }
    };
    eprintln!("ketra: receiving on :{}, dest = {}", server.port(), dest_dir.display());
    eprintln!("  fingerprint: {}", server.identity().fingerprint);
    eprintln!("  (Ctrl-C to stop)");
    loop {
        if sig::was_interrupted() {
            cancel.store(true, Ordering::SeqCst);
            server.stop();
            eprintln!("\nketra: receiver stopped");
            return 0;
        }
        if !server.is_running() {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn pct(delta: u64, size: u64) -> String {
    if size == 0 {
        return format!("{} B", human(delta));
    }
    format!("{:.1}%", 100.0 * delta as f64 / size as f64)
}

/// The receive-side progress hook gets (name, delta, size) but the server
/// tracks cumulative bytes internally; we render the percentage from size
/// only as a textual placeholder that the name progress already implies.
fn pct_from(size: u64, _name_len: usize) -> String {
    if size == 0 { "0.0%".into() } else { format!("{:.0} B total", size) }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n - 1).collect();
        format!("{t}…")
    }
}

fn human(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB { format!("{:.2} GiB", v / GB) }
    else if v >= MB { format!("{:.1} MiB", v / MB) }
    else if v >= KB { format!("{:.1} KiB", v / KB) }
    else { format!("{n} B") }
}
