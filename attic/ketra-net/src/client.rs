//! The send client: resolves a peer (via the `PeerRegistry` or a direct
//! `ip:port`), runs the LocalSend v2 handshake (`/register` →
//! `/prepare-upload`), and uploads each file. When both peers support the
//! Ketra accelerated extension, uploads use parallel ranged streams with
//! per-chunk SHA-256 / XXH64 / CRC32C checksums.

use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::ServerName;
use rustls::ClientConfig;

use crate::identity::{DeviceFingerprint, DeviceIdentity};
use crate::protocol::{
    DeviceInfo, FileMetadata, FileType, KetraExtension, PrepareUploadRequest, PrepareUploadResponse,
};
use crate::transfer::{ProgressEvent, ProgressSink, TransferOutcome};

/// Options for a send operation.
pub struct SendOptions {
    /// Chunk size for accelerated uploads (default 4 MiB).
    pub chunk_size: u64,
    /// Max parallel streams (default 4; receiver may negotiate down).
    pub parallel_streams: u8,
    /// Preferred checksum algorithm.
    pub checksum: String,
    /// Whether to compute SHA-256 of each whole file and include in metadata.
    pub include_file_sha256: bool,
    /// Whether to send the manifest as the final "file" of the session.
    pub send_manifest: bool,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            parallel_streams: 4,
            checksum: "sha256".into(),
            include_file_sha256: true,
            send_manifest: true,
        }
    }
}

/// A send client bound to a specific peer.
pub struct SendClient {
    id: DeviceIdentity,
    peer_alias: String,
    peer_addr: SocketAddr,
    peer_fingerprint: DeviceFingerprint,
    tls_cfg: Arc<ClientConfig>,
    sink: Arc<dyn ProgressSink>,
    ketra: KetraExtension,
}

impl SendClient {
    /// Construct a send client targeting a specific peer.
    pub fn new(
        id: DeviceIdentity,
        peer_alias: String,
        peer_addr: SocketAddr,
        peer_fingerprint: DeviceFingerprint,
        ketra: KetraExtension,
        sink: Arc<dyn ProgressSink>,
    ) -> std::io::Result<Self> {
        let tls_cfg = crate::tls::client_config_pinned(peer_fingerprint.clone())?;
        Ok(Self { id, peer_alias, peer_addr, peer_fingerprint, tls_cfg, sink, ketra })
    }

    /// Send a list of files to the peer. Returns the outcome.
    pub fn send(&self, files: &[PathBuf], opts: &SendOptions) -> std::io::Result<TransferOutcome> {
        // Build file metadata: id = "1", "2", ...; compute SHA-256 if requested.
        let mut metas: Vec<FileMetadata> = Vec::with_capacity(files.len());
        for (i, path) in files.iter().enumerate() {
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("file{i}"));
            let size = std::fs::metadata(path)?.len();
            let sha256 = if opts.include_file_sha256 {
                Some(compute_file_sha256(path)?)
            } else {
                None
            };
            metas.push(FileMetadata {
                id: (i + 1).to_string(),
                filename,
                size,
                file_type: FileType::from_ext(
                    &path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(),
                ),
                sha256,
                preview: None,
                ketra_chunk_size: if self.ketra.accelerated { opts.chunk_size } else { 0 },
            });
        }

        // 1. /register — announce ourselves.
        let our_info = DeviceInfo::from_identity(&self.id, self.ketra.clone());
        let reg_body = format!("{{\"info\":{}}}", our_info.to_json());
        let reg_resp = self.post("/api/localsend/v2/register", &reg_body, &[])?;
        // reg_resp.body is the peer's DeviceInfo. We could verify it matches
        // our pinned fingerprint; the TLS layer already does that.

        // 2. /prepare-upload — describe files.
        let prep = PrepareUploadRequest { info: our_info.clone(), files: metas.clone() };
        let prep_resp_body = self.post("/api/localsend/v2/prepare-upload", &prep.to_json(), &[])?;
        let prep_resp = PrepareUploadResponse::parse(&String::from_utf8_lossy(&prep_resp_body.body))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad prepare-upload response"))?;
        let accelerated = prep_resp.ketra.as_ref().map(|k| k.accelerated).unwrap_or(false);
        let parallel = prep_resp.ketra.as_ref().map(|k| k.parallel_streams).unwrap_or(1).max(1).min(opts.parallel_streams);
        let checksum_algo = prep_resp.ketra.as_ref().map(|k| k.checksum.clone()).unwrap_or_else(|| opts.checksum.clone());

        self.sink.emit(ProgressEvent::Started {
            peer_alias: self.peer_alias.clone(),
            session_id: prep_resp.session_id.clone(),
            file_count: metas.len(),
            total_bytes: metas.iter().map(|m| m.size).sum(),
        });

        let mut ok = 0usize;
        let mut failed = 0usize;
        let mut bytes_sent: u64 = 0;

        for (i, meta) in metas.iter().enumerate() {
            // Did the receiver accept this file?
            let accepted = prep_resp.files.iter().find(|(id, _)| id == &meta.id).map(|(_, a)| *a).unwrap_or(false);
            if !accepted {
                self.sink.emit(ProgressEvent::FileDone { index: i, verified: false, sha256: None });
                failed += 1;
                continue;
            }
            self.sink.emit(ProgressEvent::FileStart { index: i, total: metas.len(), meta: meta.clone() });

            let path = &files[i];
            let r = if accelerated && parallel > 1 && meta.size > opts.chunk_size {
                self.upload_parallel(path, meta, &prep_resp.session_id, opts, parallel, &checksum_algo)
            } else {
                self.upload_sequential(path, meta, &prep_resp.session_id, opts, accelerated, &checksum_algo)
            };
            match r {
                Ok(n) => {
                    bytes_sent += n;
                    ok += 1;
                    self.sink.emit(ProgressEvent::FileDone { index: i, verified: true, sha256: meta.sha256.clone() });
                }
                Err(e) => {
                    failed += 1;
                    self.sink.emit(ProgressEvent::FileDone { index: i, verified: false, sha256: None });
                    self.sink.emit(ProgressEvent::Failed { message: format!("{}: {e}", meta.filename) });
                }
            }
        }

        // 3. Manifest exchange: if negotiated, send our manifest as the last file.
        let manifest_exchanged = accelerated && prep_resp.ketra.as_ref().map(|k| k.manifest_exchange).unwrap_or(false) && opts.send_manifest;
        if manifest_exchanged {
            // Best-effort: synthesize a minimal manifest from the files we just sent.
            let manifest = synthesize_manifest(&metas);
            let manifest_meta = FileMetadata {
                id: "manifest".to_string(),
                filename: "KetraCopy-manifest.jsonl".into(),
                size: manifest.len() as u64,
                file_type: FileType::Text,
                sha256: Some(ketra_core::hash::sha256(&manifest).iter().map(|b| format!("{b:02x}")).collect()),
                preview: None,
                ketra_chunk_size: 0,
            };
            let _ = self.upload_bytes(&manifest, &manifest_meta, &prep_resp.session_id, accelerated, &checksum_algo);
        }

        let outcome = TransferOutcome { ok, failed, bytes: bytes_sent, manifest_exchanged, error: None };
        self.sink.emit(ProgressEvent::Completed { ok, failed, bytes: bytes_sent, manifest_exchanged });
        let _ = reg_resp;
        Ok(outcome)
    }

    /// Sequential single-stream upload (LocalSend v2 baseline).
    fn upload_sequential(
        &self,
        path: &Path,
        meta: &FileMetadata,
        session_id: &str,
        _opts: &SendOptions,
        accelerated: bool,
        checksum_algo: &str,
    ) -> std::io::Result<u64> {
        let mut file = std::fs::File::open(path)?;
        let total = meta.size;
        // Build the request URL.
        let url = format!(
            "/api/localsend/v2/upload?sessionId={}&fileId={}&token=ketra",
            session_id, meta.id
        );
        // Headers we want: Content-Length, and (if accelerated) X-Ketra-Checksum.
        // For sequential we send the whole file in one request; the checksum
        // covers the whole body.
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        if accelerated {
            // Compute checksum of the whole file first.
            let csum = compute_file_checksum(path, checksum_algo)?;
            extra_headers.push(("X-Ketra-Checksum".into(), csum));
        }
        // We need to stream the body. Since our HTTP helper doesn't support
        // streaming bodies yet, we read the whole file into memory. For very
        // large files this is a limitation; in practice the parallel path
        // handles large files.
        let mut buf = Vec::with_capacity(total as usize);
        file.read_to_end(&mut buf)?;
        let resp = self.post_with_headers(&url, "", "application/octet-stream", &buf, &extra_headers)?;
        if resp.status >= 200 && resp.status < 300 {
            Ok(buf.len() as u64)
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::Other, format!("upload failed: HTTP {}", resp.status)))
        }
    }

    /// Parallel ranged upload: split the file into chunks, upload each chunk
    /// concurrently with `parallel` workers, each with its own checksum.
    fn upload_parallel(
        &self,
        path: &Path,
        meta: &FileMetadata,
        session_id: &str,
        opts: &SendOptions,
        parallel: u8,
        checksum_algo: &str,
    ) -> std::io::Result<u64> {
        let chunk_size = opts.chunk_size;
        let total = meta.size;
        let chunks: Vec<(u64, u64)> = chunk_ranges(total, chunk_size);
        let path = Arc::new(path.to_path_buf());
        let session_id = Arc::new(session_id.to_string());
        let errors = Arc::new(Mutex::new(Vec::<String>::new()));
        let bytes_done = Arc::new(AtomicU64Counter::new(0));

        // Scoped threads: each worker pops a (start, end) range and uploads it.
        let ranges = Arc::new(Mutex::new(chunks.clone().into_iter().collect::<std::collections::VecDeque<_>>()));
        let mut handles = Vec::new();
        for _ in 0..parallel {
            let ranges = Arc::clone(&ranges);
            let path = Arc::clone(&path);
            let session_id = Arc::clone(&session_id);
            let meta_id = meta.id.clone();
            let tls_cfg = self.tls_cfg.clone();
            let peer_addr = self.peer_addr;
            let checksum_algo = checksum_algo.to_string();
            let errors = Arc::clone(&errors);
            let bytes_done = Arc::clone(&bytes_done);
            let sink = Arc::clone(&self.sink);
            let total_bytes = total;
            let index: usize = meta_id.parse().unwrap_or(0);
            handles.push(std::thread::spawn(move || -> std::io::Result<()> {
                loop {
                    let range = ranges.lock().unwrap().pop_front();
                    let Some((start, end)) = range else { break };
                    // Read the chunk from the file.
                    let mut f = std::fs::File::open(path.as_ref())?;
                    f.seek(SeekFrom::Start(start))?;
                    let len = (end - start + 1) as usize;
                    let mut buf = vec![0u8; len];
                    f.read_exact(&mut buf)?;
                    // Compute chunk checksum.
                    let csum = compute_bytes_checksum(&buf, &checksum_algo);
                    let url = format!(
                        "/api/localsend/v2/upload?sessionId={}&fileId={}&token=ketra",
                        session_id, meta_id
                    );
                    let extra = vec![
                        ("Range".into(), format!("bytes={start}-{end}")),
                        ("X-Ketra-Checksum".into(), csum),
                    ];
                    let resp = tls_post(&tls_cfg, peer_addr, &url, "application/octet-stream", &buf, &extra)?;
                    if !(200..300).contains(&resp.status) {
                        errors.lock().unwrap().push(format!("chunk {start}-{end}: HTTP {}", resp.status));
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("HTTP {}", resp.status)));
                    }
                    let done = bytes_done.fetch_add(len as u64);
                    sink.emit(ProgressEvent::FileProgress { index, bytes_done: done, bytes_total: total_bytes });
                }
                Ok(())
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        let errs = errors.lock().unwrap().clone();
        if !errs.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, errs.join("; ")));
        }
        Ok(total)
    }

    /// Upload a raw byte buffer (used for the manifest pseudo-file).
    fn upload_bytes(&self, body: &[u8], meta: &FileMetadata, session_id: &str, accelerated: bool, checksum_algo: &str) -> std::io::Result<()> {
        let url = format!("/api/localsend/v2/upload?sessionId={}&fileId={}&token=ketra", session_id, meta.id);
        let mut extra = Vec::new();
        if accelerated {
            extra.push(("X-Ketra-Checksum".into(), compute_bytes_checksum(body, checksum_algo)));
        }
        let resp = self.post_with_headers(&url, "", "application/octet-stream", body, &extra)?;
        if resp.status >= 200 && resp.status < 300 {
            Ok(())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::Other, format!("HTTP {}", resp.status)))
        }
    }

    // --- low-level HTTPS POST helpers ---

    fn post(&self, path: &str, body: &str, _extra: &[(&str, &str)]) -> std::io::Result<HttpResponse> {
        self.post_with_headers(path, "", "application/json; charset=utf-8", body.as_bytes(), &[])
    }

    fn post_with_headers(&self, path: &str, _query: &str, content_type: &str, body: &[u8], extra: &[(String, String)]) -> std::io::Result<HttpResponse> {
        tls_post(&self.tls_cfg, self.peer_addr, path, content_type, body, extra)
    }
}

/// An HTTP response from the peer.
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Perform an HTTPS POST via rustls to `addr` (TCP), `path` (URL path+query),
/// with `body` bytes. `extra` headers are sent as-is.
fn tls_post(cfg: &Arc<ClientConfig>, addr: SocketAddr, path: &str, content_type: &str, body: &[u8], extra: &[(String, String)]) -> std::io::Result<HttpResponse> {
    let mut sock = TcpStream::connect(addr)?;
    sock.set_nodelay(true)?;
    let server_name: ServerName<'static> = ServerName::try_from("localsend")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let mut conn = rustls::ClientConnection::new(cfg.clone(), server_name)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    // Complete TLS handshake.
    let mut buf = [0u8; 8192];
    while conn.is_handshaking() {
        let n = sock.read(&mut buf)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "tls handshake eof"));
        }
        conn.read_tls(&mut &buf[..n])?;
        conn.process_new_packets()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    // Send the request.
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: localsend\r\nContent-Type: {ct}\r\nContent-Length: {cl}\r\nConnection: close\r\n",
        ct = content_type,
        cl = body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    use std::io::Write;
    conn.writer().write_all(head.as_bytes())?;
    conn.writer().write_all(body)?;
    conn.writer().flush()?;
    while conn.wants_write() {
        conn.write_tls(&mut &sock)?;
    }
    // Read the response.
    let mut resp_buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if conn.wants_read() {
            let n = conn.read_tls(&mut &sock)?;
            if n == 0 {
                break;
            }
            conn.process_new_packets()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            loop {
                match conn.reader().read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => resp_buf.extend_from_slice(&tmp[..n]),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
        } else {
            // Try to read more TLS bytes.
            let n = sock.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            let mut slice = &tmp[..n];
            conn.read_tls(&mut slice)?;
            conn.process_new_packets()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }
    }
    // Parse the response: status line + headers + body.
    let head_end = resp_buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(resp_buf.len());
    let head_str = String::from_utf8_lossy(&resp_buf[..head_end]);
    let status = head_str
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body = resp_buf.get(head_end + 4..).unwrap_or(&[]).to_vec();
    Ok(HttpResponse { status, body })
}

/// Split `[0, total)` into inclusive `(start, end)` ranges of `chunk_size`.
fn chunk_ranges(total: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut start = 0u64;
    while start < total {
        let end = (start + chunk_size - 1).min(total - 1);
        out.push((start, end));
        start = end + 1;
    }
    out
}

/// Compute SHA-256 of a whole file (hex string).
fn compute_file_sha256(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = ketra_core::hash::Digest::new(ketra_core::hash::Algo::Sha256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish_hex())
}

/// Compute a whole-file checksum in the given algorithm (hex string).
fn compute_file_checksum(path: &Path, algo: &str) -> std::io::Result<String> {
    use std::io::Read;
    let a = match algo {
        "xxh64" => ketra_core::hash::Algo::Xxh64,
        "crc32c" => ketra_core::hash::Algo::Crc32c,
        _ => ketra_core::hash::Algo::Sha256,
    };
    let mut f = std::fs::File::open(path)?;
    let mut h = ketra_core::hash::Digest::new(a);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish_hex())
}

/// Compute the checksum of an in-memory buffer.
fn compute_bytes_checksum(buf: &[u8], algo: &str) -> String {
    let a = match algo {
        "xxh64" => ketra_core::hash::Algo::Xxh64,
        "crc32c" => ketra_core::hash::Algo::Crc32c,
        _ => ketra_core::hash::Algo::Sha256,
    };
    let mut h = ketra_core::hash::Digest::new(a);
    h.update(buf);
    h.finish_hex()
}

/// Build a minimal `manifest.jsonl` from the sent files (one record per file).
fn synthesize_manifest(files: &[FileMetadata]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in files {
        let sha = f.sha256.clone().unwrap_or_default();
        let line = format!(
            "{{\"t\":\"f\",\"p\":\"{}\",\"s\":{},\"sha256\":\"{}\"}}\n",
            f.filename, f.size, sha
        );
        out.extend_from_slice(line.as_bytes());
    }
    out
}

use std::sync::Mutex;

/// Tiny atomic u64 counter (std-only).
struct AtomicU64Counter {
    v: Mutex<u64>,
}
impl AtomicU64Counter {
    fn new(start: u64) -> Self {
        Self { v: Mutex::new(start) }
    }
    fn fetch_add(&self, n: u64) -> u64 {
        let mut g = self.v.lock().unwrap();
        let prev = *g;
        *g += n;
        prev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_ranges_split_correctly() {
        let r = chunk_ranges(10, 4);
        assert_eq!(r, vec![(0, 3), (4, 7), (8, 9)]);
    }

    #[test]
    fn chunk_ranges_single_chunk() {
        let r = chunk_ranges(3, 4);
        assert_eq!(r, vec![(0, 2)]);
    }

    #[test]
    fn synthesize_manifest_has_one_line_per_file() {
        let metas = vec![
            FileMetadata { id: "1".into(), filename: "a.txt".into(), size: 5, file_type: FileType::Text, sha256: Some("ab".into()), preview: None, ketra_chunk_size: 0 },
            FileMetadata { id: "2".into(), filename: "b.bin".into(), size: 100, file_type: FileType::File, sha256: Some("cd".into()), preview: None, ketra_chunk_size: 0 },
        ];
        let m = synthesize_manifest(&metas);
        let s = String::from_utf8(m).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains("\"p\":\"a.txt\""));
        assert!(s.contains("\"p\":\"b.bin\""));
    }

    #[test]
    fn compute_bytes_checksum_sha256_matches_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let s = compute_bytes_checksum(b"abc", "sha256");
        assert_eq!(s, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
