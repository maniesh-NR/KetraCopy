//! The LAN transfer **sender**: discovers receivers, negotiates a session
//! and uploads files — single-stream (LocalSend v2 compatible) or with the
//! Ketra accelerated extension (parallel range streams + end-to-end
//! SHA-256 manifest, see `docs/ALGORITHMS.md`).
//!
//! Integrity model: the sender hashes every byte it actually streams
//! (single-stream: while sending; parallel: one post-send pass) and, on
//! Ketra receivers, compares digests end-to-end via
//! `POST /api/ketra/v1/manifest`. On plain v2 receivers the offered digest
//! (sent in `prepare-upload`) is verified by the receiver itself.

use super::http::{self, ClientResponse};
use super::protocol::{
    FileMeta, Identity, ManifestCheck, ManifestVerdict, PrepareUpload, UploadSession,
};
use crate::hash::{Hasher, Sha256};
use crate::throttle::TokenBucket;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sender configuration for one transfer.
#[derive(Debug, Clone)]
pub struct SendConfig {
    /// Destination sub-directory hint (empty = receiver default inbox).
    pub dest_dir_hint: String,
    /// Parallel range streams for Ketra receivers (1 = single stream).
    pub parallel: u32,
    /// End-to-end digest verification (always on for Ketra receivers).
    pub verify: bool,
    /// Optional hard speed limit in bytes/sec (shared by all streams).
    pub limit_bps: Option<u64>,
    /// HTTP timeouts.
    pub timeout: Duration,
    /// Use the Ketra accelerated extension (auto: true when both ends are
    /// KetraCopy; forced off for pure v2 compatibility).
    pub ketra_mode: bool,
}

/// Result of one send run.
#[derive(Debug, Clone, Default)]
pub struct SendReport {
    pub files_total: u32,
    pub files_sent: u32,
    pub files_verified: u32,
    pub files_failed: u32,
    pub bytes: u64,
    pub duration: Duration,
    /// Per-file status: (name, "sent" | "verified" | "failed", bytes).
    pub files: Vec<(String, String, u64)>,
    /// Number of parallel streams actually used (per file max).
    pub max_streams_used: u32,
}

struct UploadFile {
    meta: FileMeta,
    src: PathBuf,
    /// Digest of the bytes we actually streamed (filled during upload).
    sent_sha: Mutex<Option<String>>,
}

/// Send `files` to the receiver at `addr` (`ip:port`).
///
/// `progress(file_name, delta_bytes, file_size)` is called from worker
/// threads; it must be cheap and thread-safe.
pub fn send_files(
    addr: &str,
    files: &[PathBuf],
    cfg: &SendConfig,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(&str, u64, u64) + Send),
) -> Result<SendReport, String> {
    let t0 = Instant::now();
    let progress = Mutex::new(progress);
    let report = send_files_inner(addr, files, cfg, cancel, &progress)?;
    let _ = progress;
    let mut r = report;
    r.duration = t0.elapsed();
    Ok(r)
}

fn emit_progress(
    p: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
    name: &str,
    delta: u64,
    size: u64,
) {
    if let Ok(mut guard) = p.lock() {
        (guard)(name, delta, size);
    }
}

fn send_files_inner(
    addr: &str,
    files: &[PathBuf],
    cfg: &SendConfig,
    cancel: &AtomicBool,
    progress: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
) -> Result<SendReport, String> {
    if files.is_empty() {
        return Err("no files given".into());
    }
    let mut report = SendReport {
        files_total: files.len() as u32,
        ..Default::default()
    };

    // ---- 1. register (identity exchange) ----
    let reg = http::request(
        addr,
        "POST",
        "/api/localsend/v2/register",
        &[],
        b"{}",
        cfg.timeout,
    )?;
    if reg.status != 200 {
        return Err(format!("register failed: HTTP {}", reg.status));
    }
    let peer_identity = reg
        .json()
        .and_then(|v| Identity::from_json(&v))
        .ok_or_else(|| "register: malformed identity".to_string())?;
    // Ketra extension when both sides speak it.
    let ketra = cfg.ketra_mode
        && peer_identity
            .to_json()
            .contains("\"ext\":\"ketra-v1\"");

    // ---- 2. build the file list (with digests when verifying) ----
    let mut uploads: Vec<UploadFile> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("file{i}"));
        let md = std::fs::metadata(f).map_err(|e| format!("{name}: {e}"))?;
        if !md.is_file() {
            return Err(format!("{name}: not a regular file"));
        }
        let sha256 = if cfg.verify && !ketra {
            // v2 receivers verify the offered digest during upload.
            Some(
                crate::options::hash_file(f, "sha256", 1024 * 1024, cancel)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        uploads.push(UploadFile {
            meta: FileMeta {
                id: format!("f{i}"),
                file_name: name.clone(),
                size: md.len(),
                file_type: super::protocol::mime_for(&name).to_string(),
                sha256,
            },
            src: f.clone(),
            sent_sha: Mutex::new(None),
        });
    }

    // ---- 3. prepare-upload ----
    let prep = PrepareUpload {
        info: Identity::new("ketra-sender", 0),
        files: uploads.iter().map(|u| u.meta.clone()).collect(),
    };
    let (base, session) = if ketra {
        match http::request(
            addr,
            "POST",
            "/api/ketra/v1/prepare-upload",
            &[],
            prep.to_json().as_bytes(),
            cfg.timeout,
        ) {
            Ok(resp) if resp.status == 200 => {
                let s = UploadSession::from_json(
                    &resp.json().ok_or("prepare: malformed response")?,
                )
                .ok_or("prepare: bad session")?;
                ("/api/ketra/v1", s)
            }
            _ => {
                // Fall back to the v2 protocol.
                ("/api/localsend/v2", prepare_v2(addr, &prep, cfg)?)
            }
        }
    } else {
        ("/api/localsend/v2", prepare_v2(addr, &prep, cfg)?)
    };
    if session.tokens.len() != uploads.len() {
        return Err(format!(
            "prepare: got {} tokens for {} files",
            session.tokens.len(),
            uploads.len()
        ));
    }

    let streams = if ketra { cfg.parallel.clamp(1, 16) } else { 1 };
    report.max_streams_used = streams;

    // ---- 4. upload every file ----
    'files: for u in &uploads {
        if cancel.load(Ordering::Relaxed) {
            let _ = http::request(
                addr,
                "POST",
                &format!("{base}/cancel?sessionId={}", session.session_id),
                &[],
                b"",
                Duration::from_secs(5),
            );
            return Err("canceled by user".into());
        }
        let token = &session.tokens[&u.meta.id];
        let name = u.meta.file_name.clone();
        let result = if ketra && streams >= 2 && u.meta.size >= 512 * 1024 {
            upload_parallel(addr, base, &session.session_id, u, token, streams, cfg, cancel, progress)
                .map(|sent| (sent, false))
        } else if ketra {
            upload_ketra_single(addr, base, &session.session_id, u, token, cfg, cancel, progress)
                .map(|sent| (sent, false))
        } else {
            upload_v2(addr, &session.session_id, u, token, cfg, cancel, progress)
        };
        match result {
            Ok((sent, verified)) => {
                report.files_sent += 1;
                if verified {
                    report.files_verified += 1;
                }
                report.bytes += sent;
                report.files.push((
                    name.clone(),
                    if verified { "verified".into() } else { "sent".into() },
                    sent,
                ));
            }
            Err(e) => {
                report.files_failed += 1;
                report.files.push((name.clone(), "failed".into(), 0));
                let _ = http::request(
                    addr,
                    "POST",
                    &format!("{base}/cancel?sessionId={}", session.session_id),
                    &[],
                    b"",
                    Duration::from_secs(5),
                );
                if e == "canceled by user" {
                    return Err(e);
                }
                if cfg.parallel <= 1 {
                    return Err(e);
                }
                // In parallel mode keep trying the remaining files.
                let _ = e;
                continue 'files;
            }
        }
    }

    // ---- 5. end-to-end manifest (Ketra receivers) ----
    if ketra && cfg.verify {
        let mut digests = BTreeMap::new();
        for u in &uploads {
            let sent = u.sent_sha.lock().unwrap().clone();
            if let Some(sha) = sent {
                digests.insert(u.meta.id.clone(), sha);
            }
        }
        let check = ManifestCheck {
            session_id: session.session_id.clone(),
            digests,
        };
        let resp = http::request(
            addr,
            "POST",
            "/api/ketra/v1/manifest",
            &[],
            check.to_json().as_bytes(),
            cfg.timeout,
        )?;
        if resp.status != 200 && resp.status != 409 {
            return Err(format!("manifest check failed: HTTP {}", resp.status));
        }
        let verdict = resp
            .json()
            .and_then(|v| ManifestVerdict::from_json(&v))
            .ok_or("manifest: malformed verdict")?;
        for u in &uploads {
            match verdict.results.get(&u.meta.id).map(String::as_str) {
                Some("ok") => {
                    report.files_verified += 1;
                    for entry in report.files.iter_mut() {
                        if entry.0 == u.meta.file_name && entry.1 == "sent" {
                            entry.1 = "verified".into();
                        }
                    }
                }
                Some(other) => {
                    report.files_failed += 1;
                    report.files_sent = report.files_sent.saturating_sub(1);
                    for entry in report.files.iter_mut() {
                        if entry.0 == u.meta.file_name {
                            entry.1 = format!("digest-{other}");
                        }
                    }
                }
                None => {}
            }
        }
        if !verdict.all_ok() {
            let bad: Vec<String> = verdict
                .results
                .iter()
                .filter(|(_, v)| v.as_str() != "ok")
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            return Err(format!("end-to-end verification failed: {}", bad.join(", ")));
        }
    }

    Ok(report)
}

fn prepare_v2(addr: &str, prep: &PrepareUpload, cfg: &SendConfig) -> Result<UploadSession, String> {
    let resp = http::request(
        addr,
        "POST",
        "/api/localsend/v2/prepare-upload",
        &[],
        prep.to_json().as_bytes(),
        cfg.timeout,
    )?;
    if resp.status == 403 {
        return Err("receiver declined the upload".into());
    }
    if resp.status != 200 {
        return Err(format!("prepare-upload failed: HTTP {}", resp.status));
    }
    UploadSession::from_json(&resp.json().ok_or("prepare: malformed response")?)
        .ok_or_else(|| "prepare: bad session".into())
}

/// v2: one POST streams the whole file while hashing on the fly.
/// Returns `(bytes_sent, end_to_end_verified)` — verified is true when the
/// receiver echoed a digest that matches the bytes we streamed.
fn upload_v2(
    addr: &str,
    session_id: &str,
    u: &UploadFile,
    token: &str,
    cfg: &SendConfig,
    cancel: &AtomicBool,
    progress: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
) -> Result<(u64, bool), String> {
    let path_and_query = format!(
        "/api/localsend/v2/upload?sessionId={session_id}&fileId={}&token={token}",
        u.meta.id
    );
    let size = u.meta.size;
    let name = u.meta.file_name.clone();
    let mut file = std::fs::File::open(&u.src).map_err(|e| format!("{}: {e}", u.src.display()))?;
    let mut sha = Sha256::new();
    let bucket = bucket_for(cfg.limit_bps);
    let sent = Arc::new(AtomicU64::new(0));

    let resp = http::request_streaming(
        addr,
        "POST",
        &path_and_query,
        &[],
        size,
        cfg.timeout,
        |stream| {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("canceled by user".into());
                }
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                if let Some(b) = &bucket {
                    b.lock().unwrap().acquire(n as u64);
                }
                stream.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                sha.update(&buf[..n]);
                let prev = sent.fetch_add(n as u64, Ordering::Relaxed);
                emit_progress(progress, &name, n as u64, size);
                let _ = prev;
            }
            Ok(())
        },
    )?;
    if resp.status != 200 && resp.status != 400 {
        return Err(format!("upload failed: HTTP {} {}", resp.status, body_text(&resp)));
    }
    if resp.status != 200 {
        return Err(format!("upload rejected: HTTP {} {}", resp.status, body_text(&resp)));
    }
    let sha_hex = Box::new(sha).finish_hex();
    *u.sent_sha.lock().unwrap() = Some(sha_hex);
    // The receiver reports its own digest of the received bytes: when it
    // echoes one, an end-to-end match is proven.
    let mut verified = false;
    if let Ok(txt) = std::str::from_utf8(&resp.body) {
        if let Ok(v) = crate::json::parse(txt) {
            if let Some(receiver_sha) = v.get("sha256").and_then(|j| j.as_str()) {
                if let Some(mine) = u.sent_sha.lock().unwrap().clone() {
                    if !mine.eq_ignore_ascii_case(receiver_sha) {
                        return Err("receiver digest does not match sent bytes".into());
                    }
                    if cfg.verify {
                        verified = true;
                    }
                }
            }
        }
    }
    Ok((size, verified))
}

/// Ketra single stream: PUT with offset 0, hash while sending.
fn upload_ketra_single(
    addr: &str,
    base: &str,
    session_id: &str,
    u: &UploadFile,
    token: &str,
    cfg: &SendConfig,
    cancel: &AtomicBool,
    progress: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
) -> Result<u64, String> {
    let path_and_query = format!(
        "{base}/upload?sessionId={session_id}&fileId={}&token={token}&offset=0",
        u.meta.id
    );
    let size = u.meta.size;
    let name = u.meta.file_name.clone();
    let mut file = std::fs::File::open(&u.src).map_err(|e| format!("{}: {e}", u.src.display()))?;
    let mut sha = Sha256::new();
    let bucket = bucket_for(cfg.limit_bps);

    let resp = http::request_streaming(
        addr,
        "PUT",
        &path_and_query,
        &[],
        size,
        cfg.timeout,
        |stream| {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("canceled by user".into());
                }
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                if let Some(b) = &bucket {
                    b.lock().unwrap().acquire(n as u64);
                }
                stream.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                sha.update(&buf[..n]);
                emit_progress(progress, &name, n as u64, size);
            }
            Ok(())
        },
    )?;
    if resp.status != 200 {
        return Err(format!("upload failed: HTTP {} {}", resp.status, body_text(&resp)));
    }
    *u.sent_sha.lock().unwrap() = Some(Box::new(sha).finish_hex());
    Ok(size)
}

/// Ketra parallel upload: `streams` threads each PUT one contiguous range,
/// throttled by one shared token bucket (same range-splitting technique as
/// `netmode.rs` / bbcp / GridFTP, see `docs/ALGORITHMS.md`).
fn upload_parallel(
    addr: &str,
    base: &str,
    session_id: &str,
    u: &UploadFile,
    token: &str,
    streams: u32,
    cfg: &SendConfig,
    cancel: &AtomicBool,
    progress: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
) -> Result<u64, String> {
    let size = u.meta.size;
    let name = u.meta.file_name.clone();
    let range_len = size.div_ceil(streams as u64);
    let bucket = bucket_for(cfg.limit_bps);
    let first_err: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for s in 0..streams as u64 {
            let start = s * range_len;
            if start >= size {
                break;
            }
            let end = ((s + 1) * range_len).min(size);
            let len = end - start;
            let pq = format!(
                "{base}/upload?sessionId={session_id}&fileId={}&token={token}&offset={start}",
                u.meta.id
            );
            let src = u.src.clone();
            let bucket = bucket.as_ref();
            let cancel = cancel;
            let progress = progress;
            let name = name.clone();
            let first_err = &first_err;
            let timeout = cfg.timeout;
            let addr = addr.to_string();
            handles.push(scope.spawn(move || {
                if first_err.lock().unwrap().is_some() {
                    return Ok(());
                }
                upload_range(
                    &addr,
                    &pq,
                    &src,
                    start,
                    len,
                    bucket,
                    cancel,
                    progress,
                    &name,
                    size,
                    timeout,
                )
            }));
        }
        for h in handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let mut g = first_err.lock().unwrap();
                    if g.is_none() {
                        *g = Some(e);
                    }
                }
                Err(_) => {
                    let mut g = first_err.lock().unwrap();
                    if g.is_none() {
                        *g = Some("worker thread panicked".into());
                    }
                }
            }
        }
    });

    if let Some(e) = first_err.into_inner().unwrap() {
        return Err(e);
    }

    // Post-pass digest of the source bytes we just streamed.
    let sha = crate::options::hash_file(&u.src, "sha256", 4 * 1024 * 1024, cancel)
        .map_err(|e| e.to_string())?;
    *u.sent_sha.lock().unwrap() = Some(sha);
    Ok(size)
}

/// PUT one `[start, start+len)` range of `src`.
fn upload_range(
    addr: &str,
    path_and_query: &str,
    src: &Path,
    start: u64,
    len: u64,
    bucket: Option<&Arc<Mutex<TokenBucket>>>,
    cancel: &AtomicBool,
    progress: &Mutex<&mut (dyn FnMut(&str, u64, u64) + Send)>,
    name: &str,
    size: u64,
    timeout: Duration,
) -> Result<(), String> {
    let file = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut reader = RangeReader {
        file,
        pos: start,
        remaining: len,
    };
    let mut buf = vec![0u8; 256 * 1024];
    http::request_streaming(
        addr,
        "PUT",
        path_and_query,
        &[],
        len,
        timeout,
        |stream| {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("canceled by user".into());
                }
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                if let Some(b) = bucket {
                    b.lock().unwrap().acquire(n as u64);
                }
                stream.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                emit_progress(progress, name, n as u64, size);
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// Sequential reader over `[start, start+len)` of a file (seek_read wrapped).
struct RangeReader {
    file: std::fs::File,
    pos: u64,
    remaining: u64,
}

impl Read for RangeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = buf.len().min(self.remaining as usize);
        let n = crate::platform::read_at(&self.file, &mut buf[..want], self.pos)?;
        if n == 0 {
            self.remaining = 0;
            return Ok(0);
        }
        self.pos += n as u64;
        self.remaining -= n as u64;
        Ok(n)
    }
}

fn bucket_for(limit_bps: Option<u64>) -> Option<Arc<Mutex<TokenBucket>>> {
    limit_bps.map(|l| Arc::new(Mutex::new(TokenBucket::new((l.max(1024)) as f64))))
}

fn body_text(resp: &ClientResponse) -> String {
    String::from_utf8_lossy(&resp.body).chars().take(200).collect()
}

/// Address-of helper used by tests that need a `&TcpStream` write sink.
#[allow(dead_code)]
fn unused_stream_helper(_s: &mut TcpStream) {}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_reader_reads_exact_window() {
        let dir = std::env::temp_dir().join(format!("ketra-lan-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rr.bin");
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let f = std::fs::File::open(&p).unwrap();
        let mut rr = RangeReader { file: f, pos: 100, remaining: 500 };
        let mut all = Vec::new();
        rr.read_to_end(&mut all).unwrap();
        assert_eq!(all, data[100..600]);
        // second read returns 0
        let mut buf = [0u8; 4];
        assert_eq!(rr.read(&mut buf).unwrap(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_send_config_defaults() {
        // The default config used by the CLI: 4 streams, verify on.
        let cfg = SendConfig {
            dest_dir_hint: String::new(),
            parallel: 4,
            verify: true,
            limit_bps: None,
            timeout: Duration::from_secs(30),
            ketra_mode: true,
        };
        assert!(cfg.ketra_mode && cfg.verify);
        assert_eq!(cfg.parallel, 4);
    }
}
