//! Minimal HTTP/1.1 helpers: a header-only `Request` struct, response
//! writers, and a query-string parser. Body streaming is handled by the
//! caller (TLS server reads body bytes via the rustls connection; the send
//! client writes body bytes via its rustls client connection).

/// Parsed HTTP request headers. The body is read separately by the caller
/// (who knows whether the transport is plain TCP or rustls TLS).
pub struct Request {
    pub method: String,
    /// Path + query string (no scheme/host).
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub content_length: Option<u64>,
}

impl Request {
    /// Get a header value by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    /// The `Content-Length` body size, if known.
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }
}

/// Write a simple HTTP response with a body on a plain `TcpStream`. The
/// connection is closed after the response (no keep-alive).
pub fn write_response(stream: &mut std::net::TcpStream, status: u16, reason: &str, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, reason, content_type, body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

/// Write a JSON response with the given status on a plain `TcpStream`.
pub fn write_json(stream: &mut std::net::TcpStream, status: u16, reason: &str, body: &str) -> std::io::Result<()> {
    write_response(stream, status, reason, "application/json; charset=utf-8", body.as_bytes())
}

/// Parse the query string of a path into key/value pairs.
pub fn parse_query(path: &str) -> Vec<(String, String)> {
    let q = match path.find('?') {
        Some(i) => &path[i + 1..],
        None => return Vec::new(),
    };
    q.split('&')
        .filter_map(|kv| {
            let mut it = kv.splitn(2, '=');
            let k = it.next()?.to_string();
            let v = it.next().unwrap_or("").to_string();
            Some((k, v))
        })
        .collect()
}
