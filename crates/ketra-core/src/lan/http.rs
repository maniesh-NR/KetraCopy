//! Minimal HTTP/1.1 client and server (std-only, no external crates).
//!
//! Scope: exactly what the LAN transfer protocol needs —
//!
//! * `POST` / `GET` / `PUT` requests with `Content-Length` bodies
//!   (chunked transfer-encoding is rejected with 411),
//! * **streaming** bodies in both directions: an upload body is exposed to
//!   handlers as a [`Body`] reader (files stream straight to disk); a
//!   response body can be a `(len, Read)` pair (files stream straight to the
//!   socket),
//! * one thread per connection, `Connection: close` semantics,
//! * query-string parsing (`?sessionId=..&fileId=..`).
//!
//! Not a general-purpose web server: no keep-alive, no TLS, no chunked
//! responses. The LAN protocol runs on a trusted private network and gets
//! its integrity from SHA-256 manifests (see `docs/ALGORITHMS.md`).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_HEADER_LINE: usize = 8 * 1024;

/// A parsed HTTP request handed to a route handler.
pub struct Request {
    pub method: String,
    /// Path without the query string, percent-decoded per segment.
    pub path: String,
    /// Decoded query parameters.
    pub query: Vec<(String, String)>,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub peer: SocketAddr,
    body: Body,
}

impl Request {
    /// First value of a query parameter.
    pub fn q(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// First value of a header (case-insensitive name).
    pub fn h(&self, key: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Body length declared by the sender (from Content-Length).
    pub fn body_len(&self) -> u64 {
        self.body.remaining
    }

    /// Consume the request and get its body reader (streams from the socket).
    pub fn into_body(self) -> Body {
        self.body
    }
}

/// Streaming request body: reads exactly `remaining` bytes from the socket.
pub struct Body {
    stream: TcpStream,
    remaining: u64,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let max = buf.len().min(self.remaining as usize);
        let n = self.stream.read(&mut buf[..max])?;
        self.remaining = self.remaining.saturating_sub(n as u64);
        Ok(n)
    }
}

/// An HTTP response. Either fully buffered (`body`) or streamed
/// (`body_stream` = exact length + reader).
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub extra_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_stream: Option<(u64, Box<dyn Read + Send>)>,
}

impl Response {
    pub fn json(status: u16, json: &str) -> Response {
        Response {
            status,
            content_type: "application/json".into(),
            extra_headers: Vec::new(),
            body: json.as_bytes().to_vec(),
            body_stream: None,
        }
    }

    pub fn text(status: u16, text: &str) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            extra_headers: Vec::new(),
            body: text.as_bytes().to_vec(),
            body_stream: None,
        }
    }

    pub fn empty(status: u16) -> Response {
        Response::text(status, "")
    }

    /// Stream `len` bytes from `reader` as the response body.
    pub fn stream(len: u64, content_type: &str, reader: Box<dyn Read + Send>) -> Response {
        Response {
            status: 200,
            content_type: content_type.into(),
            extra_headers: Vec::new(),
            body: Vec::new(),
            body_stream: Some((len, reader)),
        }
    }
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Percent-decode a URL component (also '+' as space in query strings).
pub fn url_decode(s: &str, plus_as_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() + 1 && i + 2 <= b.len() - 1 + 1 => {
                if i + 2 < b.len() {
                    let hex = &s[i + 1..i + 3];
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                }
                out.push(b'%');
                i += 1;
            }
            b'+' if plus_as_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (url_decode(k, true), url_decode(v, true)),
            None => (url_decode(p, true), String::new()),
        })
        .collect()
}

/// Read one header line (CRLF-terminated) from the stream.
fn read_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte)?;
        if n == 0 {
            break;
        }
        if byte[0] == b'\n' {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            break;
        }
        line.push(byte[0]);
        if line.len() > MAX_HEADER_LINE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header line too long",
            ));
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// Parse an incoming request off the socket. Returns `None` on a clean EOF.
fn read_request(stream: &mut TcpStream, peer: SocketAddr) -> std::io::Result<Option<Request>> {
    let start = match read_line(stream) {
        Ok(l) if l.is_empty() => return Ok(None),
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let _version = parts.next().unwrap_or_default();

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut total = 0usize;
    loop {
        let line = read_line(stream)?;
        if line.is_empty() {
            break;
        }
        total += line.len();
        if total > MAX_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    let (path_raw, query_raw) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target.as_str(), ""),
    };
    let path = path_raw
        .split('/')
        .map(|seg| url_decode(seg, false))
        .collect::<Vec<_>>()
        .join("/");
    let query = parse_query(query_raw);

    let content_length: u64 = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "chunked bodies are not supported",
        ));
    }
    // Must be set BEFORE moving `stream` into Body: read_request consumed
    // nothing of the body yet, the reader continues from the same socket.
    let body = Body {
        stream: stream.try_clone()?,
        remaining: content_length,
    };
    Ok(Some(Request { method, path, query, headers, peer, body }))
}

fn write_response(stream: &mut TcpStream, resp: Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nConnection: close\r\n",
        resp.status,
        status_text(resp.status),
        resp.content_type
    );
    for (k, v) in &resp.extra_headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    match &resp.body_stream {
        Some((len, _)) => head.push_str(&format!("Content-Length: {len}\r\n")),
        None => head.push_str(&format!("Content-Length: {}\r\n", resp.body.len())),
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if let Some((len, mut reader)) = resp.body_stream {
        let mut remaining = len;
        let mut buf = [0u8; 64 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = reader.read(&mut buf[..want])?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n])?;
            remaining -= n as u64;
        }
    } else {
        stream.write_all(&resp.body)?;
    }
    stream.flush()
}

/// Route handler: receives the request (with streaming body).
pub type Handler = Box<dyn Fn(Request) -> Response + Send + Sync>;

/// A small threaded HTTP server.
pub struct HttpServer {
    listener: TcpListener,
    routes: Vec<(String, String, Arc<Handler>)>, // (method, path, handler)
    running: Arc<AtomicBool>,
    threads: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl HttpServer {
    /// Bind `addr` (`127.0.0.1:0` for an ephemeral port) with no routes yet.
    pub fn bind(addr: &str) -> std::io::Result<HttpServer> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(HttpServer {
            listener,
            routes: Vec::new(),
            running: Arc::new(AtomicBool::new(true)),
            threads: std::sync::Mutex::new(Vec::new()),
        })
    }

    pub fn port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Register a handler for `method` + path (exact match, e.g.
    /// `/api/localsend/v2/upload`).
    pub fn route(&mut self, method: &str, path: &str, handler: Handler) {
        self.routes.push((method.to_string(), path.to_string(), Arc::new(handler)));
    }

    /// Start the accept loop on a background thread.
    pub fn start(self) -> std::io::Result<Arc<HttpServer>> {
        let arc = Arc::new(self);
        let a = Arc::clone(&arc);
        let t = std::thread::Builder::new()
            .name("ketra-http".into())
            .spawn(move || accept_loop(a))?;
        arc.threads.lock().unwrap().push(t);
        Ok(arc)
    }

    /// Stop accepting and finish in-flight connections soon.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn accept_loop(server: Arc<HttpServer>) {
    while server.running.load(Ordering::Relaxed) {
        match server.listener.accept() {
            Ok((mut stream, peer)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(600)))
                    .ok();
                stream
                    .set_write_timeout(Some(Duration::from_secs(600)))
                    .ok();
                stream.set_nonblocking(false).ok();
                let routes: Vec<(String, String, Arc<Handler>)> = server.routes.clone();
                let t = std::thread::Builder::new()
                    .name("ketra-http-conn".into())
                    .spawn(move || {
                        let _ = handle_conn(&mut stream, peer, &routes);
                    });
                if let Ok(t) = t {
                    server.threads.lock().unwrap().push(t);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                if !server.running.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn handle_conn(
    stream: &mut TcpStream,
    peer: SocketAddr,
    routes: &[(String, String, Arc<Handler>)],
) -> std::io::Result<()> {
    let req = match read_request(stream, peer) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => {
            let code = if e.kind() == std::io::ErrorKind::Unsupported { 411 } else { 400 };
            let resp = Response::text(code, &e.to_string());
            return write_response(stream, resp);
        }
    };
    // Method + path match.
    let handler = routes
        .iter()
        .find(|(m, p, _)| {
            (m == "*" || m.eq_ignore_ascii_case(&req.method)) && req.path == *p
        })
        .map(|(_, _, h)| h);
    let resp = match handler {
        Some(h) => h(req),
        None => Response::text(404, "not found"),
    };
    write_response(stream, resp)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// One HTTP response from the client side.
pub struct ClientResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ClientResponse {
    /// Parsed JSON body (when the content type says so).
    pub fn json(&self) -> Option<crate::json::Json> {
        crate::json::parse(std::str::from_utf8(&self.body).ok()?).ok()
    }
}

fn read_client_response(stream: &mut TcpStream) -> std::io::Result<ClientResponse> {
    let mut status = 0u16;
    let mut headers = Vec::new();
    // status line
    let line = read_line(stream)?;
    if let Some(code) = line.split_whitespace().nth(1) {
        status = code.parse().unwrap_or(0);
    }
    loop {
        let line = read_line(stream)?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let len: u64 = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len as usize];
    let mut read = 0usize;
    while read < body.len() {
        let n = stream.read(&mut body[read..])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    body.truncate(read);
    Ok(ClientResponse { status, headers, body })
}

/// Perform a buffered request: `addr`, method, `path?query`, extra headers
/// and a fully in-memory body.
pub fn request(
    addr: &str,
    method: &str,
    path_and_query: &str,
    headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<ClientResponse, String> {
    let mut stream = connect(addr, timeout)?;
    send_request_head(&mut stream, method, path_and_query, headers, body.len() as u64)?;
    stream.write_all(body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    read_client_response(&mut stream).map_err(|e| e.to_string())
}

/// Perform a streaming request: `body_len` bytes will be written by `writer`
/// (used to upload files while hashing them on the fly).
pub fn request_streaming<F: FnOnce(&mut TcpStream) -> Result<(), String>>(
    addr: &str,
    method: &str,
    path_and_query: &str,
    headers: &[(String, String)],
    body_len: u64,
    timeout: Duration,
    writer: F,
) -> Result<ClientResponse, String> {
    let mut stream = connect(addr, timeout)?;
    send_request_head(&mut stream, method, path_and_query, headers, body_len)?;
    writer(&mut stream)?;
    stream.flush().map_err(|e| e.to_string())?;
    read_client_response(&mut stream).map_err(|e| e.to_string())
}

fn connect(addr: &str, timeout: Duration) -> Result<TcpStream, String> {
    let stream = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    Ok(stream)
}

fn send_request_head(
    stream: &mut TcpStream,
    method: &str,
    path_and_query: &str,
    headers: &[(String, String)],
    body_len: u64,
) -> Result<(), String> {
    let mut head = format!(
        "{method} {path_and_query} HTTP/1.1\r\nHost: ketra\r\nContent-Length: {body_len}\r\nConnection: close\r\n"
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("a%20b", false), "a b");
        assert_eq!(url_decode("a+b", true), "a b");
        assert_eq!(url_decode("a+b", false), "a+b");
        assert_eq!(url_decode("caf%C3%A9", false), "café");
        assert_eq!(url_decode("100%", false), "100%");
        assert_eq!(url_decode("d%2", false), "d%2");
    }

    #[test]
    fn test_server_roundtrip_buffered() {
        let mut srv = HttpServer::bind("127.0.0.1:0").unwrap();
        let port = srv.port();
        srv.route("POST", "/api/test", Box::new(|req| {
            let len = req.body_len();
            let body = req.into_body();
            let mut all = Vec::new();
            body.take(1024).read_to_end(&mut all).ok();
            Response::json(200, &format!("{{\"len\":{len},\"body\":\"{}\"}}", String::from_utf8_lossy(&all)))
        }));
        srv.route("GET", "/api/missing", Box::new(|_| Response::empty(204)));
        let s = srv.start().unwrap();

        let resp = request(
            &format!("127.0.0.1:{port}"),
            "POST",
            "/api/test?x=1",
            &[],
            b"hello world",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, br#"{"len":11,"body":"hello world"}"#.to_vec());

        let resp = request(
            &format!("127.0.0.1:{port}"),
            "GET",
            "/api/missing",
            &[],
            b"",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 204);

        let resp = request(
            &format!("127.0.0.1:{port}"),
            "GET",
            "/api/nowhere",
            &[],
            b"",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 404);
        s.stop();
    }

    #[test]
    fn test_server_streaming_response_and_upload() {
        let mut srv = HttpServer::bind("127.0.0.1:0").unwrap();
        let port = srv.port();
        srv.route("PUT", "/up", Box::new(|req| {
            // stream the body to /dev/null-ish: count bytes
            let mut body = req.into_body();
            let mut buf = [0u8; 4096];
            let mut total = 0u64;
            loop {
                match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => total += n as u64,
                    Err(_) => break,
                }
            }
            Response::json(201, &format!("{{\"bytes\":{total}}}"))
        }));
        srv.route("GET", "/down", Box::new(move |_| {
            let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
            Response::stream(data.len() as u64, "application/octet-stream", Box::new(std::io::Cursor::new(data)))
        }));
        let s = srv.start().unwrap();

        // streaming upload (write in 3 chunks)
        let data = vec![7u8; 30_000];
        let d2 = data.clone();
        let resp = request_streaming(
            &format!("127.0.0.1:{port}"),
            "PUT",
            "/up",
            &[],
            d2.len() as u64,
            Duration::from_secs(5),
            |stream| {
                for chunk in d2.chunks(10_000) {
                    stream.write_all(chunk).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.body, br#"{"bytes":30000}"#.to_vec());

        // streaming download
        let resp = request(
            &format!("127.0.0.1:{port}"),
            "GET",
            "/down",
            &[],
            b"",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body.len(), 100_000);
        assert_eq!(resp.body[99999], (99_999u32 % 251) as u8);
        s.stop();
    }

    #[test]
    fn test_query_parsing() {
        let mut srv = HttpServer::bind("127.0.0.1:0").unwrap();
        let port = srv.port();
        srv.route("GET", "/q", Box::new(|req| {
            let session = req.q("sessionId").unwrap_or("").to_string();
            let fid = req.q("fileId").unwrap_or("").to_string();
            Response::json(200, &format!("{{\"s\":\"{session}\",\"f\":\"{fid}\"}}"))
        }));
        let s = srv.start().unwrap();
        let resp = request(
            &format!("127.0.0.1:{port}"),
            "GET",
            "/q?sessionId=abc%20def&fileId=x%2Fy",
            &[],
            b"",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.body, br#"{"s":"abc def","f":"x/y"}"#.to_vec());
        s.stop();
    }
}
