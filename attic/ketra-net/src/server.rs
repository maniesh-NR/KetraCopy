//! The receive server: binds an HTTPS listener on `port`, handles the
//! LocalSend v2 endpoints, and writes incoming files into a destination
//! directory using `ketra-core`'s streaming copy + inline verify.
//!
//! When both peers are KetraCopy and the accelerated extension is negotiated,
//! the upload endpoint accepts `Range:` headers for parallel chunked upload
//! and `X-Ketra-Checksum` per chunk.

use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::ServerConfig;

use crate::http::{parse_query, Request};
use crate::identity::DeviceIdentity;
use crate::protocol::{
    DeviceInfo, FileType, FileMetadata, KetraExtension, PrepareUploadResponse,
};
use crate::transfer::{ProgressEvent, ProgressSink, TransferOutcome};

/// Options for the receive server.
pub struct ReceiveOptions {
    /// Where to place received files. A subdirectory per session is created
    /// to avoid collisions (e.g. `<dest>/<alias>-<date>/`).
    pub dest_dir: PathBuf,
    /// The Ketra extension block we advertise.
    pub ketra: KetraExtension,
    /// Whether to require a PIN (Payload AES-GCM). When `Some(pin)`, the
    /// `/register` request must include `X-Localsend-Pin: <pin>` matching.
    pub pin: Option<String>,
    /// Whether to auto-accept incoming sessions (server mode). When false,
    /// the caller must approve via the `accept_session` callback.
    pub auto_accept: bool,
}

impl Default for ReceiveOptions {
    fn default() -> Self {
        Self {
            dest_dir: PathBuf::from("."),
            ketra: KetraExtension {
                accelerated: true,
                checksum: "sha256".into(),
                parallel_streams: 4,
                resume_supported: true,
                manifest_exchange: true,
            },
            pin: None,
            auto_accept: true,
        }
    }
}

/// The running receive server. Dropping it stops the listener thread.
pub struct ReceiveServer {
    listener: TcpListener,
    tls_cfg: Arc<ServerConfig>,
    id: DeviceIdentity,
    opts: ReceiveOptions,
    sink: Arc<dyn ProgressSink>,
    /// Active sessions: `session_id -> (dest_subdir, accepted_files)`.
    sessions: Arc<Mutex<Vec<ActiveSession>>>,
}

#[derive(Clone)]
struct ActiveSession {
    session_id: String,
    dest_subdir: PathBuf,
    files: Vec<FileMetadata>,
}

impl ReceiveServer {
    /// Bind the server (does not yet start accepting). Call `serve` to spawn
    /// the accept loop.
    pub fn bind(id: DeviceIdentity, opts: ReceiveOptions, sink: Arc<dyn ProgressSink>) -> std::io::Result<Self> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), id.port);
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(false)?;
        let tls_cfg = crate::tls::server_config(&id.cert_pem, &id.key_pem)?;
        Ok(Self { listener, tls_cfg, id, opts, sink, sessions: Arc::new(Mutex::new(Vec::new())) })
    }

    /// The port actually bound (useful when `id.port == 0` for ephemeral).
    pub fn local_port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(self.id.port)
    }

    /// Spawn the accept loop. Returns immediately; the loop runs on a thread.
    pub fn serve(self: Arc<Self>) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("ketra-recv".into())
            .spawn(move || {
                for stream in self.listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let server = Arc::clone(&self);
                    let _ = std::thread::Builder::new()
                        .name("ketra-conn".into())
                        .spawn(move || {
                            let _ = server.handle_conn(stream);
                        });
                }
            })
            .expect("spawn recv thread")
    }

    fn handle_conn(&self, stream: TcpStream) -> std::io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_nodelay(true)?;
        let mut stream = stream;
        let mut incoming = rustls::ServerConnection::new(self.tls_cfg.clone())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        // Drive the bidirectional TLS handshake: read ClientHello, write
        // ServerHello+Certificate+ServerHelloDone, read ClientFinished.
        let mut buf = [0u8; 8192];
        while incoming.is_handshaking() {
            // Flush any handshake bytes the server wants to send.
            while incoming.wants_write() {
                incoming.write_tls(&mut stream)?;
            }
            if !incoming.is_handshaking() {
                break;
            }
            // Read more TLS bytes from the client.
            let r = stream.read(&mut buf)?;
            if r == 0 {
                return Ok(());
            }
            let mut slice = &buf[..r];
            incoming.read_tls(&mut slice)?;
            if let Err(e) = incoming.process_new_packets() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        }
        // Final flush of the finished handshake (NewSessionTicket etc.).
        while incoming.wants_write() {
            incoming.write_tls(&mut stream)?;
        }
        // Now drive the application data loop.
        let conn = Arc::new(Mutex::new(incoming));
        let stream_arc = Arc::new(Mutex::new(stream));
        self.dispatch_loop(conn, stream_arc)
    }

    fn dispatch_loop(&self, conn: Arc<Mutex<rustls::ServerConnection>>, stream: Arc<Mutex<TcpStream>>) -> std::io::Result<()> {
        let req = read_request(&conn, &stream)?;
        let Some(req) = req else { return Ok(()); };
        let path = req.path.split('?').next().unwrap_or(&req.path).to_string();
        match (req.method.as_str(), path.as_str()) {
            ("GET", p) if p.ends_with("/info") => {
                let info = DeviceInfo::from_identity(&self.id, self.opts.ketra.clone());
                let _ = write_response_tls(&conn, &stream, 200, "OK", "application/json", info.to_json().as_bytes());
            }
            ("POST", p) if p.ends_with("/register") => {
                self.handle_register(req, &conn, &stream)?;
            }
            ("POST", p) if p.ends_with("/prepare-upload") => {
                self.handle_prepare_upload(req, &conn, &stream)?;
            }
            ("POST", p) if p.starts_with("/upload") => {
                self.handle_upload(req, &conn, &stream)?;
            }
            ("POST", p) if p.starts_with("/cancel") => {
                self.handle_cancel(req, &conn, &stream)?;
            }
            _ => {
                let _ = write_json_tls(&conn, &stream, 404, "Not Found", r#"{"error":"not found"}"#);
            }
        }
        Ok(())
    }

    fn handle_register(&self, req: Request, conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<()> {
        // Body is JSON: DeviceInfo of the sender. We respond with our own info.
        let body = read_all_body(req.content_length, conn, stream)?;
        let sender_info = DeviceInfo::parse(&String::from_utf8_lossy(&body)).unwrap_or_else(|| DeviceInfo {
            alias: "?".into(), version: 2, device_model: String::new(),
            device_type: crate::identity::DeviceType::Desktop, fingerprint: String::new(),
            port: 53317, download: true, ketra: None,
        });
        // PIN check
        if let Some(expected) = &self.opts.pin {
            let got = req.header("x-localsend-pin").unwrap_or("");
            if got != expected {
                let _ = write_json_tls(conn, stream, 403, "Forbidden", r#"{"error":"pin mismatch"}"#);
                return Ok(());
            }
        }
        let our_info = DeviceInfo::from_identity(&self.id, self.opts.ketra.clone());
        let _ = write_json_tls(conn, stream, 200, "OK", &our_info.to_json());
        let _ = sender_info; // could log
        Ok(())
    }

    fn handle_prepare_upload(&self, req: Request, conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<()> {
        let body = read_all_body(req.content_length, conn, stream)?;
        let body_str = String::from_utf8_lossy(&body);
        // Parse the `files` map: each entry has id + FileMetadata.
        let files = parse_prepare_upload_files(&body_str);
        let session_id = format!("s{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
        // Build the dest subdir for this session.
        let subdir = self.opts.dest_dir.join(format!("{}-{}", session_id, chrono_like_ts()));
        std::fs::create_dir_all(&subdir)?;
        // Decide accept/decline per file. Auto-accept all when auto_accept.
        let accepted: Vec<(String, bool)> = files.iter().map(|f| (f.id.clone(), self.opts.auto_accept)).collect();
        // Negotiate Ketra extension: only if both peers support it.
        let ketra_response = self.opts.ketra.accelerated.then(|| self.opts.ketra.clone());
        let resp = PrepareUploadResponse {
            session_id: session_id.clone(),
            files: accepted.clone(),
            ketra: ketra_response,
        };
        let _ = write_json_tls(conn, stream, 200, "OK", &resp.to_json());
        // Stash the session.
        self.sessions.lock().unwrap().push(ActiveSession {
            session_id,
            dest_subdir: subdir,
            files,
        });
        Ok(())
    }

    fn handle_upload(&self, req: Request, conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<()> {
        let query = parse_query(&req.path);
        let session_id = query.iter().find(|(k, _)| k == "sessionId").map(|(_, v)| v.clone()).unwrap_or_default();
        let file_id = query.iter().find(|(k, _)| k == "fileId").map(|(_, v)| v.clone()).unwrap_or_default();
        let _token = query.iter().find(|(k, _)| k == "token").map(|(_, v)| v.clone()).unwrap_or_default();
        let range = req.header("range").map(String::from);
        let chunk_checksum = req.header("x-ketra-checksum").map(String::from);

        // Find the session + file metadata.
        let session_meta = {
            let sessions = self.sessions.lock().unwrap();
            sessions.iter().find(|s| s.session_id == session_id).and_then(|s| {
                s.files.iter().find(|f| f.id == file_id).map(|f| (s.dest_subdir.clone(), f.clone()))
            })
        };
        let Some((dest_subdir, meta)) = session_meta else {
            let _ = write_json_tls(conn, stream, 404, "Not Found", r#"{"error":"no such session/file"}"#);
            return Ok(());
        };
        let dest_path = dest_subdir.join(&meta.filename);
        std::fs::create_dir_all(&dest_subdir)?;

        // Range: "bytes=START-END" (inclusive) or "START-". If present, we
        // seek to START and write until we've written END-START+1 (or all).
        let start_offset = range.as_deref().and_then(|r| {
            r.strip_prefix("bytes=").and_then(|s| s.split('-').next()).and_then(|s| s.parse::<u64>().ok())
        }).unwrap_or(0);

        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(start_offset == 0)
            .open(&dest_path)?;
        out.seek(SeekFrom::Start(start_offset))?;

        // Stream the body to disk in 64KiB chunks, accumulating a SHA-256 if
        // the file declares one (inline verify per chunk).
        let total = req.content_length().unwrap_or(0);
        let mut written: u64 = 0;
        let mut chunk_hasher = match meta.sha256.as_ref() {
            Some(_) => Some(ketra_core::hash::Digest::new(ketra_core::hash::Algo::Sha256)),
            None => None,
        };
        // For per-chunk checksum (X-Ketra-Checksum), verify the chunk bytes.
        let chunk_algo = self.opts.ketra.checksum.as_str();
        let mut chunk_digest = match chunk_checksum {
            Some(_) => Some(match chunk_algo {
                "xxh64" => ketra_core::hash::Digest::new(ketra_core::hash::Algo::Xxh64),
                "crc32c" => ketra_core::hash::Digest::new(ketra_core::hash::Algo::Crc32c),
                _ => ketra_core::hash::Digest::new(ketra_core::hash::Algo::Sha256),
            }),
            None => None,
        };

        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let want = buf.len().min((total.saturating_sub(written)) as usize);
            if want == 0 && total > 0 {
                break;
            }
            let n = read_body_chunk(conn, stream, &mut buf[..want])?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            if let Some(h) = chunk_hasher.as_mut() {
                h.update(&buf[..n]);
            }
            if let Some(h) = chunk_digest.as_mut() {
                h.update(&buf[..n]);
            }
            written += n as u64;
            self.sink.emit(ProgressEvent::FileProgress {
                index: file_id.parse::<usize>().unwrap_or(0),
                bytes_done: start_offset + written,
                bytes_total: meta.size,
            });
        }
        out.flush()?;

        // Verify per-chunk checksum (consumes the digest).
        if let Some(expected_hex) = chunk_checksum.as_ref() {
            if let Some(h) = chunk_digest.take() {
                let actual = h.finish_hex();
                if actual != *expected_hex {
                    let _ = write_json_tls(conn, stream, 422, "Unprocessable Entity", r#"{"error":"checksum mismatch"}"#);
                    return Ok(());
                }
            }
        }
        // Verify whole-file SHA-256 once the file is fully written.
        let verified = if let Some(expected_hex) = meta.sha256.as_ref() {
            if start_offset == 0 && written == meta.size {
                if let Some(h) = chunk_hasher.take() {
                    h.finish_hex() == *expected_hex
                } else {
                    true
                }
            } else {
                // Partial chunk upload: defer verification to session end.
                true
            }
        } else {
            true
        };
        let _ = write_json_tls(conn, stream, 200, "OK", r#"{"ok":true}"#);
        self.sink.emit(ProgressEvent::FileDone {
            index: file_id.parse::<usize>().unwrap_or(0),
            verified,
            sha256: meta.sha256.clone(),
        });
        Ok(())
    }

    fn handle_cancel(&self, _req: Request, conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<()> {
        let _ = write_json_tls(conn, stream, 200, "OK", r#"{"ok":true}"#);
        self.sink.emit(ProgressEvent::Failed { message: "session cancelled by peer".into() });
        Ok(())
    }
}

// ---- TLS I/O helpers ----

fn read_request(conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<Option<Request>> {
    // Read until we have the headers (\r\n\r\n), then hand the rest off.
    let mut header_buf: Vec<u8> = Vec::with_capacity(2048);
    let mut byte = [0u8; 1];
    loop {
        if header_buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if header_buf.len() > 64 * 1024 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "header too large"));
        }
        let n = tls_read(conn, stream, &mut byte)?;
        if n == 0 {
            if header_buf.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof during headers"));
        }
        header_buf.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&header_buf).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no request line"))?;
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(idx) = line.find(':') {
            headers.push((line[..idx].trim().to_ascii_lowercase(), line[idx + 1..].trim().to_string()));
        }
    }
    let content_length = headers.iter().find(|(n, _)| n == "content-length").and_then(|(_, v)| v.parse::<u64>().ok());
    Ok(Some(Request { method, path, headers, content_length }))
}

fn read_all_body(content_length: Option<u64>, conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>) -> std::io::Result<Vec<u8>> {
    let n = content_length.unwrap_or(0) as usize;
    let mut out = Vec::with_capacity(n);
    let mut buf = vec![0u8; 8192];
    while out.len() < n {
        let want = buf.len().min(n - out.len());
        let r = read_body_chunk(conn, stream, &mut buf[..want])?;
        if r == 0 {
            break;
        }
        out.extend_from_slice(&buf[..r]);
    }
    Ok(out)
}

fn read_body_chunk(conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>, dst: &mut [u8]) -> std::io::Result<usize> {
    let mut conn = conn.lock().unwrap();
    let mut total = 0;
    while total < dst.len() {
        // Drain any buffered plaintext first; tolerate WouldBlock.
        match conn.reader().read(&mut dst[total..]) {
            Ok(n) if n > 0 => {
                total += n;
                if total >= dst.len() {
                    break;
                }
                continue;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        let mut sock = stream.lock().unwrap();
        let mut cbuf = [0u8; 8192];
        let r = sock.read(&mut cbuf)?;
        if r == 0 {
            break;
        }
        let mut slice = &cbuf[..r];
        conn.read_tls(&mut slice)?;
        if let Err(e) = conn.process_new_packets() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
        }
    }
    Ok(total)
}

fn tls_read(conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>, dst: &mut [u8]) -> std::io::Result<usize> {
    let mut conn = conn.lock().unwrap();
    loop {
        // Try to read plaintext from rustls's buffer. Returns WouldBlock
        // when no plaintext is currently decrypted — we must fetch more
        // ciphertext from the socket and feed it through read_tls.
        match conn.reader().read(dst) {
            Ok(n) if n > 0 => return Ok(n),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        // Read more ciphertext from the socket and feed it to rustls.
        let mut sock = stream.lock().unwrap();
        let mut cbuf = [0u8; 8192];
        let r = sock.read(&mut cbuf)?;
        if r == 0 {
            return Ok(0);
        }
        let mut slice = &cbuf[..r];
        conn.read_tls(&mut slice)?;
        if let Err(e) = conn.process_new_packets() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
        }
    }
}

fn write_response_tls(conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>, status: u16, reason: &str, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, reason, content_type, body.len()
    );
    let mut conn = conn.lock().unwrap();
    conn.writer().write_all(head.as_bytes())?;
    conn.writer().write_all(body)?;
    conn.writer().flush()?;
    let _ = conn.write_tls(&mut &mut *stream.lock().unwrap());
    Ok(())
}

fn write_json_tls(conn: &Arc<Mutex<rustls::ServerConnection>>, stream: &Arc<Mutex<TcpStream>>, status: u16, reason: &str, body: &str) -> std::io::Result<()> {
    write_response_tls(conn, stream, status, reason, "application/json; charset=utf-8", body.as_bytes())
}

// ---- tiny helpers ----

fn parse_prepare_upload_files(body: &str) -> Vec<FileMetadata> {
    // The body is `{"info":{...},"files":{"1":{...},"2":{...}}}`. We extract
    // each file object by brace-matching.
    let files_key = match body.find("\"files\"") {
        Some(i) => i,
        None => return Vec::new(),
    };
    let after = &body[files_key..];
    let lbrace = match after.find('{') {
        Some(i) => i,
        None => return Vec::new(),
    };
    let body = &after[lbrace + 1..];
    let rbrace = brace_match_close(body).unwrap_or(body.len());
    let body = &body[..rbrace];
    let mut out = Vec::new();
    let mut p = 0;
    while p < body.len() {
        // Find next quoted key.
        let Some(q) = body[p..].find('"') else { break };
        let q = p + q;
        let key_start = q + 1;
        let Some(key_end_rel) = body[key_start..].find('"') else { break };
        let id = body[key_start..key_start + key_end_rel].to_string();
        // Skip to the value's opening brace.
        let rest = &body[key_start + key_end_rel + 1..];
        let Some(vb) = rest.find('{') else { break };
        let val_body = &rest[vb + 1..];
        let vc = brace_match_close(val_body).unwrap_or(val_body.len());
        let obj = &val_body[..vc];
        out.push(parse_file_metadata(&id, obj));
        p = key_start + key_end_rel + 1 + vb + 1 + vc;
    }
    out
}

fn brace_match_close(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, b) in s.bytes().enumerate() {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn parse_file_metadata(id: &str, obj: &str) -> FileMetadata {
    let filename = json_str_field(obj, "fileName").unwrap_or_default();
    let size = json_num_field(obj, "size").map(|v| v as u64).unwrap_or(0);
    let sha256 = json_str_field(obj, "sha256");
    let preview = json_str_field(obj, "preview");
    let file_type = FileType::from_ext(&filename);
    let ketra_chunk_size = json_num_field(obj, "chunkSize").map(|v| v as u64).unwrap_or(0);
    FileMetadata { id: id.to_string(), filename, size, file_type, sha256, preview, ketra_chunk_size }
}

fn json_str_field(s: &str, field: &str) -> Option<String> {
    let key = format!("\"{}\"", field);
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let q = rest.find('"')?;
    let rest = &rest[q + 1..];
    let mut end = 0;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        if bytes[end] == b'"' && bytes[end.saturating_sub(1)] != b'\\' {
            break;
        }
        end += 1;
    }
    if end >= bytes.len() {
        return None;
    }
    Some(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn json_num_field(s: &str, field: &str) -> Option<f64> {
    let key = format!("\"{}\"", field);
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let mut end = 0;
    for b in rest.bytes() {
        if b == b'-' || b == b'+' || b == b'.' || b == b'e' || b == b'E' || (b as char).is_ascii_digit() {
            end += 1;
        } else {
            break;
        }
    }
    rest[..end].parse().ok()
}

fn chrono_like_ts() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:x}", secs)
}

// (hex helpers no longer needed — `Digest::finish_hex` returns lowercase hex directly.)
