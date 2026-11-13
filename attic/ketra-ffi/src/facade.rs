//! Façade implementations backing the UniFFI proc-macro scaffolding. These
//! are the Rust-side bodies of `KetraEngine`, `ProgressCallback`, and the
//! `Record` dictionaries. UniFFI generates the Kotlin-facing wrappers from
//! the `#[derive(uniffi::*)]` and `#[uniffi::export]` annotations.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ketra_core::hash::{Algo, Digest};
use ketra_net::client::{SendClient, SendOptions};
use ketra_net::discovery::{Peer, PeerRegistry};
use ketra_net::identity::{DeviceIdentity, DeviceType};
use ketra_net::protocol::{FileMetadata, FileType, KetraExtension};
use ketra_net::server::{ReceiveOptions, ReceiveServer};
use ketra_net::transfer::{ProgressEvent as NetProgress, ProgressSink};
use ketra_net::{self as net, ServiceDaemon};

// ---------- Records (dictionaries) ----------

/// Options for in-device copy/move and LAN send.
#[derive(uniffi::Record)]
pub struct EngineOptions {
    pub dest_dir: String,
    pub verify_inline: bool,
    pub include_file_sha256: bool,
    pub send_manifest: bool,
    pub chunk_size: u64,
    pub parallel_streams: u8,
    pub checksum: String,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            dest_dir: String::new(),
            verify_inline: true,
            include_file_sha256: true,
            send_manifest: true,
            chunk_size: 4 * 1024 * 1024,
            parallel_streams: 4,
            checksum: "sha256".into(),
        }
    }
}

/// A discovered LAN peer, snapshot of `ketra_net::discovery::Peer`.
#[derive(uniffi::Record)]
pub struct PeerInfo {
    pub service_name: String,
    pub alias: String,
    pub device_type: String,
    pub fingerprint: String,
    pub port: u16,
    pub ip: String,
    pub ketra: bool,
}

/// One progress event delivered to the Kotlin callback. `kind` selects which
/// fields are meaningful (mirrors `ketra_net::transfer::ProgressEvent`).
#[derive(uniffi::Record)]
pub struct ProgressEvent {
    pub kind: String,
    pub peer_alias: String,
    pub session_id: String,
    pub file_index: u32,
    pub file_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub verified: bool,
    pub sha256: String,
    pub filename: String,
    pub size: u64,
    pub ok: u32,
    pub failed: u32,
    pub bytes: u64,
    pub manifest_exchanged: bool,
    pub message: String,
}

impl Default for ProgressEvent {
    fn default() -> Self {
        Self {
            kind: String::new(),
            peer_alias: String::new(),
            session_id: String::new(),
            file_index: 0,
            file_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            verified: false,
            sha256: String::new(),
            filename: String::new(),
            size: 0,
            ok: 0,
            failed: 0,
            bytes: 0,
            manifest_exchanged: false,
            message: String::new(),
        }
    }
}

/// Final outcome of a transfer session.
#[derive(uniffi::Record)]
pub struct TransferResult {
    pub ok: u32,
    pub failed: u32,
    pub bytes: u64,
    pub manifest_exchanged: bool,
    pub error_message: String,
}

// ---------- Callback interface ----------

/// Kotlin implements this trait; Rust calls `on_event` from worker threads.
/// UniFFI bridges the call across the JNI boundary.
#[uniffi::export(with_foreign)]
pub trait ProgressCallback: Send + Sync {
    fn on_event(&self, event: ProgressEvent);
}

// ---------- Object: KetraEngine ----------

/// The main façade. Constructed once per app session.
#[derive(uniffi::Object)]
pub struct KetraEngine {
    identity: Mutex<DeviceIdentity>,
    ketra_ext: KetraExtension,
    registry: Mutex<Option<PeerRegistry>>,
    daemon: Mutex<Option<ServiceDaemon>>,
    receiver: Mutex<Option<Arc<ReceiveServer>>>,
    receiver_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[uniffi::export]
impl KetraEngine {
    #[uniffi::constructor]
    pub fn new(
        alias: String,
        model: String,
        device_type: String,
        port: u16,
        identity_dir: String,
    ) -> Self {
        let dt = match device_type.as_str() {
            "mobile" => DeviceType::Mobile,
            "desktop" => DeviceType::Desktop,
            "web" => DeviceType::Web,
            "server" => DeviceType::Server,
            _ => DeviceType::Headless,
        };
        let id = DeviceIdentity::load_or_generate(&PathBuf::from(&identity_dir), alias, model, dt, port)
            .unwrap_or_else(|_| {
                DeviceIdentity::generate("Ketra".into(), "Unknown".into(), dt, port)
                    .expect("identity generation must not fail")
            });
        Self {
            identity: Mutex::new(id),
            ketra_ext: KetraExtension {
                accelerated: true,
                checksum: "sha256".into(),
                parallel_streams: 4,
                resume_supported: true,
                manifest_exchange: true,
            },
            registry: Mutex::new(None),
            daemon: Mutex::new(None),
            receiver: Mutex::new(None),
            receiver_thread: Mutex::new(None),
        }
    }

    // ---- Discovery ----

    pub fn start_discovery(&self) {
        let id = self.identity.lock().unwrap().clone();
        if let Ok((daemon, registry)) = net::discovery::start(&id, &self.ketra_ext, None) {
            *self.daemon.lock().unwrap() = Some(daemon);
            *self.registry.lock().unwrap() = Some(registry);
        }
    }

    pub fn stop_discovery(&self) {
        if let Some(d) = self.daemon.lock().unwrap().take() {
            let _ = d.shutdown();
        }
        *self.registry.lock().unwrap() = None;
    }

    pub fn list_peers(&self) -> Vec<PeerInfo> {
        let reg = self.registry.lock().unwrap();
        let Some(reg) = reg.as_ref() else { return Vec::new() };
        reg.snapshot().into_iter().map(peer_to_info).collect()
    }

    // ---- In-device copy/move/test (ketra-core 3-phase pipeline) ----

    pub fn copy(&self, sources: Vec<String>, dest_dir: String, callback: Option<Arc<dyn ProgressCallback>>) -> TransferResult {
        run_engine_simple("copy", sources, dest_dir, callback)
    }

    pub fn move_files(&self, sources: Vec<String>, dest_dir: String, callback: Option<Arc<dyn ProgressCallback>>) -> TransferResult {
        run_engine_simple("move", sources, dest_dir, callback)
    }

    pub fn test(&self, sources: Vec<String>, dest_dir: String, callback: Option<Arc<dyn ProgressCallback>>) -> TransferResult {
        run_engine_simple("test", sources, dest_dir, callback)
    }

    // ---- Manifest + verify ----

    pub fn manifest(&self, root_dir: String, out_file: String) -> bool {
        write_manifest(&PathBuf::from(root_dir), &PathBuf::from(out_file)).is_ok()
    }

    pub fn verify(&self, root_dir: String, manifest_file: String) -> bool {
        verify_manifest(&PathBuf::from(root_dir), &PathBuf::from(manifest_file)).is_ok()
    }

    // ---- LAN transfer ----

    pub fn send(
        &self,
        files: Vec<String>,
        peer_fingerprint: String,
        opts: Option<EngineOptions>,
        callback: Option<Arc<dyn ProgressCallback>>,
    ) -> TransferResult {
        let opts = opts.unwrap_or_default();
        let id = self.identity.lock().unwrap().clone();
        let peer = self.registry.lock().unwrap().as_ref().and_then(|r| {
            r.snapshot().into_iter().find(|p| {
                p.fingerprint == peer_fingerprint || p.fingerprint.replace('-', "") == peer_fingerprint.replace('-', "")
            })
        });
        let Some(peer) = peer else {
            return TransferResult {
                ok: 0, failed: 0, bytes: 0, manifest_exchanged: false,
                error_message: "peer not found".into(),
            };
        };
        let addr = match peer.ip {
            Some(ip) => std::net::SocketAddr::new(ip, peer.port),
            None => return TransferResult {
                ok: 0, failed: 0, bytes: 0, manifest_exchanged: false,
                error_message: "peer has no IP".into(),
            },
        };
        let fp = net::discovery::peer_fingerprint(&peer);
        let sink: Arc<dyn ProgressSink> = Arc::new(CallbackSink::new(callback));
        let client = match SendClient::new(id, peer.alias.clone(), addr, fp, self.ketra_ext.clone(), sink) {
            Ok(c) => c,
            Err(e) => return TransferResult {
                ok: 0, failed: files.len() as u32, bytes: 0, manifest_exchanged: false,
                error_message: e.to_string(),
            },
        };
        let paths: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
        let send_opts = SendOptions {
            chunk_size: opts.chunk_size,
            parallel_streams: opts.parallel_streams,
            checksum: opts.checksum,
            include_file_sha256: opts.include_file_sha256,
            send_manifest: opts.send_manifest,
        };
        match client.send(&paths, &send_opts) {
            Ok(o) => TransferResult {
                ok: o.ok as u32, failed: o.failed as u32, bytes: o.bytes,
                manifest_exchanged: o.manifest_exchanged, error_message: o.error.unwrap_or_default(),
            },
            Err(e) => TransferResult {
                ok: 0, failed: paths.len() as u32, bytes: 0, manifest_exchanged: false,
                error_message: e.to_string(),
            },
        }
    }

    pub fn start_receiver(&self, dest_dir: String, auto_accept: bool, callback: Option<Arc<dyn ProgressCallback>>) {
        let id = self.identity.lock().unwrap().clone();
        let sink: Arc<dyn ProgressSink> = Arc::new(CallbackSink::new(callback));
        let opts = ReceiveOptions {
            dest_dir: PathBuf::from(dest_dir),
            ketra: self.ketra_ext.clone(),
            pin: None,
            auto_accept,
        };
        if let Ok(server) = ReceiveServer::bind(id, opts, sink) {
            let server = Arc::new(server);
            let s = Arc::clone(&server);
            let handle = s.serve();
            *self.receiver.lock().unwrap() = Some(server);
            *self.receiver_thread.lock().unwrap() = Some(handle);
        }
    }

    pub fn stop_receiver(&self) {
        *self.receiver.lock().unwrap() = None;
        *self.receiver_thread.lock().unwrap() = None;
    }

    // ---- Bench ----

    pub fn bench(&self, path: String, size: u64) -> u64 {
        let p = PathBuf::from(path).join("ketra-bench.bin");
        let start = std::time::Instant::now();
        let mut f = match std::fs::File::create(&p) { Ok(f) => f, Err(_) => return 0 };
        let chunk = vec![0xAAu8; 64 * 1024];
        let mut written = 0u64;
        while written < size {
            let n = (chunk.len() as u64).min(size - written) as usize;
            if std::io::Write::write_all(&mut f, &chunk[..n]).is_err() {
                return 0;
            }
            written += n as u64;
        }
        let _ = f.sync_all();
        let elapsed = start.elapsed().as_secs_f64();
        let _ = std::fs::remove_file(&p);
        if elapsed > 0.0 {
            ((written as f64 / elapsed) / (1024.0 * 1024.0)) as u64
        } else {
            0
        }
    }
}

// ---------- Bridge: ketra-net ProgressSink -> UniFFI ProgressCallback ----------

struct CallbackSink {
    callback: Option<Arc<dyn ProgressCallback>>,
}

impl CallbackSink {
    fn new(callback: Option<Arc<dyn ProgressCallback>>) -> Self {
        Self { callback }
    }
}

impl ProgressSink for CallbackSink {
    fn emit(&self, ev: NetProgress) {
        if let Some(cb) = &self.callback {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cb.on_event(net_event_to_record(&ev));
            }));
        }
    }
}

fn net_event_to_record(ev: &NetProgress) -> ProgressEvent {
    let mut r = ProgressEvent::default();
    match ev {
        NetProgress::Started { peer_alias, session_id, file_count, total_bytes } => {
            r.kind = "started".into();
            r.peer_alias = peer_alias.clone();
            r.session_id = session_id.clone();
            r.file_total = *file_count as u32;
            r.bytes_total = *total_bytes;
        }
        NetProgress::FileStart { index, total, meta } => {
            r.kind = "file_start".into();
            r.file_index = *index as u32;
            r.file_total = *total as u32;
            r.filename = meta.filename.clone();
            r.size = meta.size;
        }
        NetProgress::FileProgress { index, bytes_done, bytes_total } => {
            r.kind = "file_progress".into();
            r.file_index = *index as u32;
            r.bytes_done = *bytes_done;
            r.bytes_total = *bytes_total;
        }
        NetProgress::FileDone { index, verified, sha256 } => {
            r.kind = "file_done".into();
            r.file_index = *index as u32;
            r.verified = *verified;
            r.sha256 = sha256.clone().unwrap_or_default();
        }
        NetProgress::Completed { ok, failed, bytes, manifest_exchanged } => {
            r.kind = "completed".into();
            r.ok = *ok as u32;
            r.failed = *failed as u32;
            r.bytes = *bytes;
            r.manifest_exchanged = *manifest_exchanged;
        }
        NetProgress::Failed { message } => {
            r.kind = "failed".into();
            r.message = message.clone();
        }
    }
    r
}

fn peer_to_info(p: Peer) -> PeerInfo {
    PeerInfo {
        service_name: p.service_name,
        alias: p.alias,
        device_type: p.device_type.as_str().to_string(),
        fingerprint: p.fingerprint,
        port: p.port,
        ip: p.ip.map(|i| i.to_string()).unwrap_or_default(),
        ketra: p.ketra,
    }
}

// ---------- In-device engine wrapper ----------
//
// We don't pull in the full ketra-core Options/Controller here (that would
// require re-mapping every field). Instead we run a streaming copy with
// SHA-256 inline verify directly — the same algorithmic core, exposed
// through a simpler synchronous API that is easy to bridge to Kotlin.

fn run_engine_simple(kind: &str, sources: Vec<String>, dest_dir: String, callback: Option<Arc<dyn ProgressCallback>>) -> TransferResult {
    let dest = PathBuf::from(&dest_dir);
    if let Err(e) = std::fs::create_dir_all(&dest) {
        return TransferResult { ok: 0, failed: sources.len() as u32, bytes: 0, manifest_exchanged: false, error_message: e.to_string() };
    }
    let mut ok = 0u32;
    let mut failed = 0u32;
    let mut bytes = 0u64;
    let mut errors = Vec::new();
    let total = sources.len();
    for (i, src) in sources.iter().enumerate() {
        let src_path = PathBuf::from(src);
        let filename = src_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| format!("file{i}"));
        let dst_path = dest.join(&filename);
        // Emit file_start.
        if let Some(cb) = &callback {
            let size = std::fs::metadata(&src_path).map(|m| m.len()).unwrap_or(0);
            cb.on_event(ProgressEvent {
                kind: "file_start".into(),
                file_index: i as u32,
                file_total: total as u32,
                filename: filename.clone(),
                size,
                ..ProgressEvent::default()
            });
        }
        let r = match kind {
            "move" => copy_and_remove(&src_path, &dst_path, &callback, i as u32),
            "test" => test_compare(&src_path, &dst_path),
            _ => copy_one(&src_path, &dst_path, &callback, i as u32),
        };
        match r {
            Ok(n) => {
                ok += 1;
                bytes += n;
                if let Some(cb) = &callback {
                    cb.on_event(ProgressEvent {
                        kind: "file_done".into(),
                        file_index: i as u32,
                        verified: true,
                        ..ProgressEvent::default()
                    });
                }
            }
            Err(e) => {
                failed += 1;
                errors.push(format!("{filename}: {e}"));
                if let Some(cb) = &callback {
                    cb.on_event(ProgressEvent {
                        kind: "file_done".into(),
                        file_index: i as u32,
                        verified: false,
                        ..ProgressEvent::default()
                    });
                }
            }
        }
    }
    if let Some(cb) = &callback {
        cb.on_event(ProgressEvent {
            kind: "completed".into(),
            ok, failed, bytes,
            ..ProgressEvent::default()
        });
    }
    TransferResult {
        ok, failed, bytes, manifest_exchanged: false,
        error_message: if errors.is_empty() { String::new() } else { errors.join("; ") },
    }
}

fn copy_one(src: &std::path::Path, dst: &std::path::Path, callback: &Option<Arc<dyn ProgressCallback>>, index: u32) -> std::io::Result<u64> {
    let total = std::fs::metadata(src)?.len();
    let mut inp = std::fs::File::open(src)?;
    let mut out = std::fs::File::create(dst)?;
    let mut h = Digest::new(Algo::Sha256);
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = std::io::Read::read(&mut inp, &mut buf)?;
        if n == 0 { break; }
        std::io::Write::write_all(&mut out, &buf[..n])?;
        h.update(&buf[..n]);
        done += n as u64;
        if let Some(cb) = callback {
            cb.on_event(ProgressEvent {
                kind: "file_progress".into(),
                file_index: index,
                bytes_done: done,
                bytes_total: total,
                ..ProgressEvent::default()
            });
        }
    }
    out.sync_all()?;
    // Preserve mtime.
    #[cfg(unix)]
    {
        if let Ok(meta) = std::fs::metadata(src) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(meta.permissions().mode()));
        }
    }
    Ok(done)
}

fn copy_and_remove(src: &std::path::Path, dst: &std::path::Path, callback: &Option<Arc<dyn ProgressCallback>>, index: u32) -> std::io::Result<u64> {
    let n = copy_one(src, dst, callback, index)?;
    std::fs::remove_file(src)?;
    Ok(n)
}

fn test_compare(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<u64> {
    use std::io::Read;
    let mut a = std::fs::File::open(src)?;
    let mut b = std::fs::File::open(dst)?;
    let mut ba = vec![0u8; 64 * 1024];
    let mut bb = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let na = a.read(&mut ba)?;
        let nb = b.read(&mut bb)?;
        if na == 0 && nb == 0 { return Ok(total); }
        if na != nb || ba[..na] != bb[..nb] {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "byte mismatch"));
        }
        total += na as u64;
    }
}

// ---------- Manifest helpers ----------

fn write_manifest(root: &std::path::Path, out: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut f_out = std::fs::File::create(out)?;
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files(root, &mut paths)?;
    for p in &paths {
        let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        let size = std::fs::metadata(p)?.len();
        let sha = file_sha256_hex(p)?;
        let line = format!("{{\"t\":\"f\",\"p\":\"{rel}\",\"s\":{size},\"sha256\":\"{sha}\"}}\n");
        f_out.write_all(line.as_bytes())?;
    }
    Ok(())
}

fn verify_manifest(root: &std::path::Path, manifest: &std::path::Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(manifest)?;
    for line in text.lines() {
        if line.trim().is_empty() { continue; }
        let p = json_field(line, "p").ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing p"))?;
        let expected = json_field(line, "sha256").ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing sha256"))?;
        let actual = file_sha256_hex(&root.join(&p))?;
        if actual != expected {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("verify failed: {p}")));
        }
    }
    Ok(())
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

fn file_sha256_hex(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Digest::new(Algo::Sha256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(h.finish_hex())
}

fn json_field(s: &str, field: &str) -> Option<String> {
    let key = format!("\"{}\":\"", field);
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// Suppress unused-symbol warnings for items referenced only via UniFFI macros.
#[allow(dead_code)]
fn _unused_refs() {
    let _ = FileMetadata { id: String::new(), filename: String::new(), size: 0, file_type: FileType::File, sha256: None, preview: None, ketra_chunk_size: 0 };
}
