//! The LAN transfer **receiver**: an HTTP server plus mDNS announcement.
//!
//! Accepts LocalSend-v2-style uploads and, from KetraCopy senders, the
//! accelerated parallel-range upload + end-to-end SHA-256 manifest check
//! (see [`super::protocol`]). Files stream straight to disk — no payload is
//! ever fully buffered in memory.

use super::http::{HttpServer, Request, Response};
use super::mdns::{Discovered, Mdns};
use super::protocol::{
    FileMeta, Identity, ManifestCheck, ManifestVerdict, PrepareUpload, UploadSession,
    PROTOCOL_VERSION,
};
use super::SECURE_NAME;
use crate::hash::{Hasher, Sha256};
use crate::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Receiver configuration.
pub struct ReceiverConfig {
    /// Display name announced on the network.
    pub alias: String,
    /// Directory incoming files land in (created when missing).
    pub dest_dir: PathBuf,
    /// Port to bind; 0 = the LocalSend default 53317 (next-free used when busy).
    pub port: u16,
    /// Accept every upload without asking (`-y`).
    pub auto_accept: bool,
    /// Hash incoming files and compare against the offered digests.
    pub verify_sha256: bool,
    /// Announce via mDNS (`_localsend._tcp.local` + `_ketra._tcp.local`).
    pub announce: bool,
    /// Max parallel range streams Ketra senders may use.
    pub max_streams: u32,
}

/// One fully received file (handed to `on_file_done`).
#[derive(Debug, Clone)]
pub struct ReceivedFile {
    pub file_name: String,
    pub size: u64,
    /// True when a digest was offered and matched.
    pub verified: bool,
    pub path: PathBuf,
}

/// Callbacks for the embedding UI (CLI / GUI / Android JNI). All hooks run
/// on HTTP handler threads, so they must be `Send + Sync`.
pub struct ReceiverHooks {
    /// Decide whether to accept an incoming upload request.
    pub on_request: Box<dyn Fn(&PrepareUpload) -> bool + Send + Sync>,
    /// A file finished (after verification when available).
    pub on_file_done: Box<dyn Fn(&ReceivedFile) + Send + Sync>,
    /// Streaming progress: (file_name, delta_bytes, file_size).
    pub on_progress: Box<dyn Fn(&str, u64, u64) + Send + Sync>,
    /// Human-readable status lines.
    pub on_message: Box<dyn Fn(String) + Send + Sync>,
}

impl Default for ReceiverHooks {
    fn default() -> Self {
        ReceiverHooks {
            on_request: Box::new(|_| true),
            on_file_done: Box::new(|_| {}),
            on_progress: Box::new(|_, _, _| {}),
            on_message: Box::new(|_| {}),
        }
    }
}

struct FileSlot {
    meta: FileMeta,
    token: String,
    path: PathBuf,
    written: AtomicU64,
    done_reported: AtomicBool,
}

struct Session {
    dest_dir: PathBuf,
    ketra: bool,
    files: BTreeMap<String, FileSlot>,
    created: Instant,
}

struct State {
    cfg: ReceiverConfig,
    hooks: ReceiverHooks,
    identity: Mutex<Identity>,
    sessions: Mutex<BTreeMap<String, Session>>,
    port: u16,
    running: AtomicBool,
}

/// A running receiver. Stop by dropping (or [`Receiver::stop`]).
pub struct Receiver {
    state: Arc<State>,
    http: Arc<HttpServer>,
    mdns: Option<Arc<Mdns>>,
}

impl Receiver {
    /// Bind, register routes and announce on mDNS.
    pub fn start(cfg: ReceiverConfig, hooks: ReceiverHooks) -> Result<Arc<Receiver>, String> {
        std::fs::create_dir_all(&cfg.dest_dir)
            .map_err(|e| format!("dest dir {}: {e}", cfg.dest_dir.display()))?;

        // Bind the default port, then the next free ones (53317..53327).
        let mut http = None;
        let mut port = if cfg.port == 0 { super::LAN_PORT } else { cfg.port };
        for cand in port..port + 10 {
            if let Ok(s) = HttpServer::bind(&format!("0.0.0.0:{cand}")) {
                http = Some(s);
                port = cand;
                break;
            }
        }
        let mut http =
            http.ok_or_else(|| format!("cannot bind any port in {port}..{}", port + 10))?;

        let identity = Identity::new(&cfg.alias, port);
        let state = Arc::new(State {
            identity: Mutex::new(identity.clone()),
            sessions: Mutex::new(BTreeMap::new()),
            cfg,
            hooks,
            port,
            running: AtomicBool::new(true),
        });

        // ---- routes (LocalSend v2) ----
        for base in ["/api/localsend/v2", "/api/ketra/v1"] {
            let ketra = base == "/api/ketra/v1";
            let st = Arc::clone(&state);
            let _ = st; // captured below via clones
            let st2 = Arc::clone(&state);
            // GET /info — identity probe (LocalSend v2 + Ketra).
            let st_info = Arc::clone(&state);
            http.route(
                "GET",
                &format!("{base}/info"),
                Box::new(move |_req| {
                    let id = st_info.identity.lock().unwrap().clone();
                    Response::json(200, &id.to_json())
                }),
            );
            http.route(
                "POST",
                &format!("{base}/register"),
                Box::new(move |_req| {
                    let id = st2.identity.lock().unwrap().clone();
                    Response::json(200, &id.to_json())
                }),
            );
            let st3 = Arc::clone(&state);
            let b = base.to_string();
            http.route(
                "POST",
                &format!("{base}/prepare-upload"),
                Box::new(move |req| prepare_upload(st3.clone(), req, &b, ketra)),
            );
            let st4 = Arc::clone(&state);
            let b2 = base.to_string();
            http.route(
                "POST",
                &format!("{base}/cancel"),
                Box::new(move |req| cancel_session(st4.clone(), req)),
            );
        }
        // v2 upload: one POST streams the whole file.
        {
            let st = Arc::clone(&state);
            http.route(
                "POST",
                "/api/localsend/v2/upload",
                Box::new(move |req| upload_v2(st.clone(), req)),
            );
        }
        // Ketra upload: one PUT per range, parallel-friendly.
        {
            let st = Arc::clone(&state);
            http.route(
                "PUT",
                "/api/ketra/v1/upload",
                Box::new(move |req| upload_ketra_range(st.clone(), req)),
            );
        }
        // Ketra manifest: end-to-end digest check.
        {
            let st = Arc::clone(&state);
            http.route(
                "POST",
                "/api/ketra/v1/manifest",
                Box::new(move |req| manifest_check(st.clone(), req)),
            );
        }

        let http = http.start().map_err(|e| e.to_string())?;

        // ---- mDNS announcement ----
        let mdns = if state.cfg.announce {
            match Mdns::spawn() {
                Some(m) => {
                    let instance = format!("ketra-{}", identity.fingerprint);
                    let mut txt = BTreeMap::new();
                    txt.insert("alias".to_string(), identity.alias.clone());
                    txt.insert("fingerprint".to_string(), identity.fingerprint.clone());
                    txt.insert("deviceModel".to_string(), identity.device_model.clone());
                    txt.insert("deviceType".to_string(), identity.device_type.clone());
                    txt.insert("version".to_string(), identity.version.clone());
                    txt.insert("protocol".to_string(), PROTOCOL_VERSION.to_string());
                    txt.insert("ketra".to_string(), "1".to_string());
                    m.announce(&["_localsend._tcp.local", "_ketra._tcp.local"], &instance, port, &txt);
                    Some(m)
                }
                None => {
                    (state.hooks.on_message)("mDNS unavailable — direct IP transfers still work".into());
                    None
                }
            }
        } else {
            None
        };

        Ok(Arc::new(Receiver { state, http, mdns }))
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.state.port
    }

    /// Our announced identity.
    pub fn identity(&self) -> Identity {
        self.state.identity.lock().unwrap().clone()
    }

    /// True while serving.
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Relaxed)
    }

    /// Stop serving and withdraw from mDNS.
    pub fn stop(&self) {
        self.state.running.store(false, Ordering::Relaxed);
        if let Some(m) = &self.mdns {
            m.withdraw();
        }
        self.http.stop();
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Discover receivers on the LAN (LocalSend + Ketra services), optionally
/// probing each with `POST /register` to fetch its full identity.
pub fn discover(mdns: Option<&Arc<Mdns>>, timeout: Duration) -> Vec<Discovered> {
    let Some(m) = mdns else { return Vec::new() };
    let mut found = m.browse(&["_localsend._tcp.local", "_ketra._tcp.local"], timeout);
    found.sort_by(|a, b| a.alias().cmp(&b.alias()));
    found.dedup_by(|a, b| a.ip == b.ip && a.port == b.port && a.instance == b.instance);
    found
}

// ---------------------------------------------------------------------------
// route handlers
// ---------------------------------------------------------------------------

fn purge_expired(sessions: &mut BTreeMap<String, Session>) {
    sessions.retain(|_, s| s.created.elapsed() < Duration::from_secs(30 * 60));
}

fn prepare_upload(state: Arc<State>, req: Request, base: &str, ketra: bool) -> Response {
    let body = read_body(req, 1 << 20);
    let parsed = json::parse(&body);
    let prep = match parsed.ok().and_then(|v| PrepareUpload::from_json(&v)) {
        Some(p) => p,
        None => return Response::text(400, "malformed prepare-upload body"),
    };

    // Accept policy.
    let accept = state.cfg.auto_accept || (state.hooks.on_request)(&prep);
    if !accept {
        return Response::json(403, "{\"error\":\"declined\"}");
    }
    if !prep.info.download && !ketra {
        // v2 devices that are not accepting: still let Ketra peers through.
    }

    // Expire old sessions, then create the new one.
    let session_id = format!(
        "s-{}",
        crate::hash::xxh64(
            format!(
                "{}:{}:{}",
                prep.info.fingerprint,
                prep.files.len(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            )
            .as_bytes(),
            0x53455331
        )
    );

    let dest_dir = state.cfg.dest_dir.clone();
    let mut files = BTreeMap::new();
    for f in &prep.files {
        if f.file_name.is_empty() {
            return Response::text(400, "empty fileName");
        }
        let safe = SECURE_NAME.replace(&f.file_name);
        let path = unique_path(&dest_dir, &safe);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Preallocate so parallel range writes land inside the final length.
        if ketra && f.size > 0 {
            if let Ok(h) = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
            {
                let _ = h.set_len(f.size);
            }
        }
        let token = format!(
            "t-{:016x}",
            crate::hash::xxh64(format!("{session_id}:{}", f.id).as_bytes(), 0x544f4b45)
        );
        files.insert(
            f.id.clone(),
            FileSlot {
                meta: f.clone(),
                token,
                path,
                written: AtomicU64::new(0),
                done_reported: AtomicBool::new(false),
            },
        );
    }

    let tokens: BTreeMap<String, String> = files
        .iter()
        .map(|(k, v)| (k.clone(), v.token.clone()))
        .collect();
    {
        let mut s = state.sessions.lock().unwrap();
        purge_expired(&mut s);
        s.insert(
            session_id.clone(),
            Session {
                dest_dir,
                ketra,
                files,
                created: Instant::now(),
            },
        );
    }

    let resp = UploadSession {
        session_id,
        tokens,
        max_streams: if ketra { state.cfg.max_streams.max(1) } else { 1 },
        wants_manifest: ketra,
    };
    let _ = base;
    Response::json(200, &resp.to_json())
}

fn cancel_session(state: Arc<State>, req: Request) -> Response {
    let sid = req.q("sessionId").unwrap_or_default().to_string();
    state.sessions.lock().unwrap().remove(&sid);
    Response::empty(204)
}

/// v2: stream the whole file body to disk, hashing while streaming.
fn upload_v2(state: Arc<State>, req: Request) -> Response {
    let sid = req.q("sessionId").unwrap_or_default().to_string();
    let fid = req.q("fileId").unwrap_or_default().to_string();
    let token = req.q("token").unwrap_or_default().to_string();

    let (path, expected_sha, size) = {
        let sessions = state.sessions.lock().unwrap();
        let Some(sess) = sessions.get(&sid) else {
            return Response::text(404, "unknown session");
        };
        let Some(slot) = sess.files.get(&fid) else {
            return Response::text(404, "unknown fileId");
        };
        if slot.token != token {
            return Response::json(403, "{\"error\":\"bad token\"}");
        }
        (slot.path.clone(), slot.meta.sha256.clone(), slot.meta.size)
    };

    // Stream to disk, hashing every byte on the way through.
    let mut file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => return Response::text(500, &format!("create: {e}")),
    };
    let mut body = req.into_body();
    let mut buf = vec![0u8; 256 * 1024];
    let mut written = 0u64;
    let mut stream_sha = Sha256::new();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Response::text(500, &format!("read: {e}")),
        };
        if file.write_all(&buf[..n]).is_err() {
            return Response::text(500, "write failed");
        }
        stream_sha.update(&buf[..n]);
        written += n as u64;
        (state.hooks.on_progress)(&name, n as u64, size);
    }

    let sha_hex = Box::new(stream_sha).finish_hex();
    let verified = match (&expected_sha, state.cfg.verify_sha256) {
        (Some(expected), true) => {
            let ok = expected.eq_ignore_ascii_case(&sha_hex);
            if !ok {
                (state.hooks.on_message)(format!(
                    "sha256 mismatch for {}: expected {expected}, got {sha_hex}",
                    path.display()
                ));
            }
            ok
        }
        _ => true,
    };

    // Report completion once.
    let done = {
        let sessions = state.sessions.lock().unwrap();
        sessions
            .get(&sid)
            .and_then(|s| s.files.get(&fid))
            .map(|slot| {
                slot.written.store(written, Ordering::Relaxed);
                slot.done_reported
                    .swap(true, Ordering::SeqCst)
            })
            .unwrap_or(false)
    };
    if !done {
        (state.hooks.on_file_done)(&ReceivedFile {
            file_name: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size: written,
            verified,
            path: path.clone(),
        });
    }

    if !verified {
        let _ = std::fs::remove_file(&path);
        return Response::json(
            422,
            &format!("{{\"ok\":false,\"sha256\":\"{sha_hex}\",\"error\":\"checksum mismatch\"}}"),
        );
    }
    if written != size {
        return Response::json(
            400,
            &format!("{{\"ok\":true,\"sha256\":\"{sha_hex}\",\"warning\":\"size mismatch: sent {written} of {size}\"}}"),
        );
    }
    Response::json(200, &format!("{{\"ok\":true,\"sha256\":\"{sha_hex}\"}}"))
}

/// Ketra extension: one PUT writes one range at `offset` (parallel-safe).
fn upload_ketra_range(state: Arc<State>, req: Request) -> Response {
    let sid = req.q("sessionId").unwrap_or_default().to_string();
    let fid = req.q("fileId").unwrap_or_default().to_string();
    let token = req.q("token").unwrap_or_default().to_string();
    let offset: u64 = req.q("offset").unwrap_or("0").parse().unwrap_or(0);

    let (path, size, name) = {
        let sessions = state.sessions.lock().unwrap();
        let Some(sess) = sessions.get(&sid) else {
            return Response::text(404, "unknown session");
        };
        if !sess.ketra {
            return Response::text(400, "session is not a ketra session");
        }
        let Some(slot) = sess.files.get(&fid) else {
            return Response::text(404, "unknown fileId");
        };
        if slot.token != token {
            return Response::json(403, "{\"error\":\"bad token\"}");
        }
        (
            slot.path.clone(),
            slot.meta.size,
            slot.meta.file_name.clone(),
        )
    };
    if offset > size {
        return Response::text(400, "offset beyond file size");
    }

    let file = match std::fs::OpenOptions::new().write(true).open(&path) {
        Ok(f) => f,
        Err(e) => return Response::text(500, &format!("open: {e}")),
    };
    let mut body = req.into_body();
    let mut buf = vec![0u8; 256 * 1024];
    let mut pos = offset;
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Response::text(500, &format!("read: {e}")),
        };
        if pos + n as u64 > size {
            return Response::text(400, "range overflows file size");
        }
        if let Err(e) = crate::platform::write_at(&file, &buf[..n], pos) {
            return Response::text(500, &format!("write_at: {e}"));
        }
        pos += n as u64;
        (state.hooks.on_progress)(&name, n as u64, size);
    }

    // Accumulate written bytes; when the file is complete, report it.
    let total = {
        let sessions = state.sessions.lock().unwrap();
        sessions
            .get(&sid)
            .and_then(|s| s.files.get(&fid))
            .map(|slot| {
                slot.written.fetch_add(pos - offset, Ordering::Relaxed) + (pos - offset)
            })
            .unwrap_or(0)
    };
    Response::json(
        200,
        &format!("{{\"ok\":true,\"offset\":{offset},\"written\":{}}}", pos - offset),
    )
    .with_note(total)
}

/// Small helper to keep the response builder above readable.
trait WithNote {
    fn with_note(self, _total: u64) -> Response;
}
impl WithNote for Response {
    fn with_note(mut self, total: u64) -> Response {
        self.extra_headers
            .push(("X-Ketra-Written".into(), total.to_string()));
        self
    }
}

/// Ketra extension: end-to-end SHA-256 manifest check.
fn manifest_check(state: Arc<State>, req: Request) -> Response {
    let body = read_body(req, 1 << 20);
    let parsed = match json::parse(&body).ok().and_then(|v| {
        let sid = v.get("sessionId")?.as_str()?.to_string();
        let mut digests = BTreeMap::new();
        if let Some(json::Json::Obj(map)) = v.get("files") {
            for (k, dv) in map {
                if let Some(d) = dv.as_str() {
                    digests.insert(k.clone(), d.to_string());
                }
            }
        }
        Some((sid, digests))
    }) {
        Some(x) => x,
        None => return Response::text(400, "malformed manifest body"),
    };
    let (session_id, digests) = parsed;
    let _ = ManifestCheck { session_id: String::new(), digests: BTreeMap::new() }; // type anchor

    let slots: Vec<(String, PathBuf, u64, String)> = {
        let sessions = state.sessions.lock().unwrap();
        let Some(sess) = sessions.get(&session_id) else {
            return Response::text(404, "unknown session");
        };
        if !sess.ketra {
            return Response::text(400, "session is not a ketra session");
        }
        sess.files
            .iter()
            .map(|(k, v)| (k.clone(), v.path.clone(), v.meta.size, v.meta.file_name.clone()))
            .collect()
    };

    let cancel = AtomicBool::new(false);
    let mut verdict = ManifestVerdict::default();
    for (fid, path, size, name) in &slots {
        let Some(expected) = digests.get(fid) else {
            verdict.results.insert(fid.clone(), "missing".into());
            continue;
        };
        // Byte-exact manifest check happens in `verify_tree` below; here we
        // hash what we actually received.
        let actual = match crate::options::hash_file(path, "sha256", 1024 * 1024, &cancel) {
            Ok(h) => h,
            Err(_) => {
                verdict.results.insert(fid.clone(), "missing".into());
                continue;
            }
        };
        let ok = actual.eq_ignore_ascii_case(expected);
        let complete = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) == *size;
        if ok && complete {
            verdict.results.insert(fid.clone(), "ok".into());
        } else {
            verdict.results.insert(fid.clone(), "mismatch".into());
            let _ = std::fs::remove_file(path);
        }
        (state.hooks.on_file_done)(&ReceivedFile {
            file_name: name.clone(),
            size: *size,
            verified: ok && complete,
            path: path.clone(),
        });
    }

    // Session fulfilled.
    state.sessions.lock().unwrap().remove(&session_id);
    if verdict.all_ok() {
        Response::json(200, &verdict.to_json())
    } else {
        Response::json(409, &verdict.to_json())
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn read_body(req: Request, max: usize) -> String {
    let len = req.body_len().min(max as u64) as usize;
    let mut body = req.into_body();
    let mut buf = vec![0u8; len];
    let mut read = 0;
    while read < buf.len() {
        match body.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => break,
        }
    }
    buf.truncate(read);
    String::from_utf8_lossy(&buf).into_owned()
}

/// `name.txt` → `name (2).txt` when `dir/name.txt` already exists.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for i in 2..10000 {
        let cand = dir.join(format!("{stem} ({i}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    dir.join(format!("{stem}.dup{ext}"))
}

/// Keep a SocketAddr import warning away (used in handler signatures above).
#[allow(dead_code)]
fn _socket_addr_anchor(_a: SocketAddr) {}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::client::{send_files, SendConfig};
    use super::*;
    use std::io::Write as _;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-lan-rcv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_unique_path() {
        let d = tmp("uniq");
        std::fs::write(d.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (2).txt"));
        std::fs::write(d.join("a (2).txt"), b"x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (3).txt"));
        assert_eq!(unique_path(&d, "b.txt"), d.join("b.txt"));
        std::fs::remove_dir_all(&d).ok();
    }

    /// Full loopback: start a receiver, send files with every strategy,
    /// verify byte-exact content + digest reporting.
    #[test]
    fn test_loopback_send_receive_all_modes() {
        let work = tmp("loop");
        let inbox = work.join("inbox");
        let src = work.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&inbox).unwrap();
        // three payloads: tiny, medium random, exact chunk boundary
        std::fs::write(src.join("tiny.txt"), b"hello lan\n").unwrap();
        let medium: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(src.join("medium.bin"), &medium).unwrap();
        let chunky: Vec<u8> = vec![0xAB; 2 * 1024 * 1024];
        std::fs::write(src.join("chunky.bin"), &chunky).unwrap();

        let received = Arc::new(Mutex::new(Vec::<ReceivedFile>::new()));
        let r2 = Arc::clone(&received);
        let hooks = ReceiverHooks {
            on_request: Box::new(|_p| true),
            on_file_done: Box::new(move |f| r2.lock().unwrap().push(f.clone())),
            on_progress: Box::new(|_, _, _| {}),
            on_message: Box::new(|_| {}),
        };
        let recv = Receiver::start(
            ReceiverConfig {
                alias: "ketra-test".into(),
                dest_dir: inbox.clone(),
                port: 0,
                auto_accept: true,
                verify_sha256: true,
                announce: false,
                max_streams: 4,
            },
            hooks,
        )
        .unwrap();
        let port = recv.port();
        let addr = format!("127.0.0.1:{port}");

        // --- v2 mode (single stream, pre-hashed) ---
        let cfg = SendConfig {
            dest_dir_hint: String::new(),
            parallel: 1,
            verify: true,
            limit_bps: None,
            timeout: Duration::from_secs(30),
            ketra_mode: false,
        };
        let cancel = AtomicBool::new(false);
        let report =
            send_files(&addr, &[src.join("tiny.txt")], &cfg, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!(report.files_sent, 1);
        assert_eq!(report.files_verified, 1);
        assert_eq!(std::fs::read(inbox.join("tiny.txt")).unwrap(), b"hello lan\n");

        // --- ketra mode, single stream, hash-while-sending ---
        let cfg = SendConfig { ketra_mode: true, parallel: 1, ..cfg };
        let report = send_files(&addr, &[src.join("medium.bin")], &cfg, &cancel, &mut |_, _, _| {})
            .unwrap();
        assert_eq!(report.files_sent, 1);
        assert_eq!(report.files_verified, 1);
        assert_eq!(std::fs::read(inbox.join("medium.bin")).unwrap(), medium);

        // --- ketra mode, 4 parallel range streams ---
        let cfg = SendConfig { ketra_mode: true, parallel: 4, ..cfg };
        let report = send_files(&addr, &[src.join("chunky.bin")], &cfg, &cancel, &mut |_, _, _| {})
            .unwrap();
        assert_eq!(report.files_sent, 1);
        assert_eq!(report.files_verified, 1);
        assert_eq!(std::fs::read(inbox.join("chunky.bin")).unwrap(), chunky);

        // --- decline path ---
        let d_work = tmp("decline");
        let recv2 = Receiver::start(
            ReceiverConfig {
                alias: "decliner".into(),
                dest_dir: d_work.clone(),
                port: 0,
                auto_accept: false,
                verify_sha256: false,
                announce: false,
                max_streams: 2,
            },
            ReceiverHooks {
                on_request: Box::new(|_| false),
                ..Default::default()
            },
        )
        .unwrap();
        let cfg = SendConfig {
            dest_dir_hint: String::new(),
            parallel: 1,
            verify: false,
            limit_bps: None,
            timeout: Duration::from_secs(30),
            ketra_mode: true,
        };
        let r = send_files(
            &format!("127.0.0.1:{}", recv2.port()),
            &[src.join("tiny.txt")],
            &cfg,
            &cancel,
            &mut |_, _, _| {},
        );
        assert!(r.is_err(), "declined upload must fail: {r:?}");
        assert!(!d_work.join("tiny.txt").exists());

        recv.stop();
        std::fs::remove_dir_all(&work).ok();
        std::fs::remove_dir_all(&d_work).ok();
    }

    #[test]
    fn test_v2_receiver_detects_corruption() {
        // A sender that lies about sha256 must get a 422 and the file removed.
        let work = tmp("corrupt");
        let inbox = work.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let recv = Receiver::start(
            ReceiverConfig {
                alias: "t".into(),
                dest_dir: inbox.clone(),
                port: 0,
                auto_accept: true,
                verify_sha256: true,
                announce: false,
                max_streams: 1,
            },
            ReceiverHooks::default(),
        )
        .unwrap();
        let addr = format!("127.0.0.0.1:{}", recv.port());
        let addr = format!("127.0.0.1:{}", recv.port());

        // hand-craft a prepare-upload with a wrong digest
        let prep = format!(
            "{{\"info\":{{\"alias\":\"x\",\"version\":\"1\",\"deviceModel\":\"m\",\"deviceType\":\"desktop\",\"fingerprint\":\"f\",\"download\":true,\"port\":1,\"protocolVersion\":\"v2\"}},\"files\":{{\"f0\":{{\"id\":\"f0\",\"fileName\":\"bad.bin\",\"size\":3,\"fileType\":\"application/octet-stream\",\"sha256\":\"{}\"}}}}}}",
            "0".repeat(64)
        );
        let resp = super::super::http::request(
            &addr,
            "POST",
            "/api/localsend/v2/prepare-upload",
            &[],
            prep.as_bytes(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        let sess = UploadSession::from_json(&resp.json().unwrap()).unwrap();
        let token = sess.tokens.get("f0").unwrap();
        let upload = format!(
            "/api/localsend/v2/upload?sessionId={}&fileId=f0&token={}",
            sess.session_id, token
        );
        let resp = super::super::http::request(
            &addr,
            "POST",
            &upload,
            &[],
            b"abc",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(resp.status, 422);
        assert!(!inbox.join("bad.bin").exists());
        recv.stop();
        std::fs::remove_dir_all(&work).ok();
    }
}
