//! LAN transfer protocol messages.
//!
//! The wire protocol follows the publicly documented **LocalSend v2** REST
//! structure (see https://localsend.org protocol docs) so KetraCopy can
//! interoperate with LocalSend-style senders and receivers, plus a
//! **Ketra accelerated extension** (`/api/ketra/v1/…`) used automatically
//! when both endpoints are KetraCopy:
//!
//! * range-parallel uploads (`PUT …?offset=N`, one request per stream,
//!   same range-splitting idea as bbcp/GridFTP, see `docs/ALGORITHMS.md`),
//! * end-to-end SHA-256 manifests exchanged after the payload
//!   (both sides hash while streaming, the receiver compares and reports).
//!
//! v2 routes (receiver side):
//!
//! | Route | Method | Purpose |
//! |---|---|---|
//! | `/api/localsend/v2/register` | POST | identity exchange |
//! | `/api/localsend/v2/prepare-upload` | POST | request a session + tokens |
//! | `/api/localsend/v2/upload?sessionId&fileId&token` | POST | file payload |
//! | `/api/localsend/v2/cancel?sessionId` | POST | cancel a session |
//!
//! Ketra extension routes (receiver side):
//!
//! | Route | Method | Purpose |
//! |---|---|---|
//! | `/api/ketra/v1/prepare-upload` | POST | session + tokens + capabilities |
//! | `/api/ketra/v1/upload?sessionId&fileId&token&offset` | PUT | one range |
//! | `/api/ketra/v1/manifest?sessionId` | POST | end-to-end SHA-256 check |
//!
//! Everything is JSON over HTTP/1.1 (no TLS in this implementation — see the
//! security note in `docs/ARCHITECTURE.md`).

use crate::json::{self, Json};
use std::collections::BTreeMap;

/// Protocol version string on the wire.
pub const PROTOCOL_VERSION: &str = "v2";
/// Ketra accelerated-extension version.
pub const KETRA_EXT: &str = "ketra-v1";

/// Device identity, announced via mDNS TXT records and `/register`.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub alias: String,
    pub version: String,
    pub device_model: String,
    /// "desktop" | "mobile" | "headless" | "server".
    pub device_type: String,
    /// Random per-installation fingerprint (two installs never collide).
    pub fingerprint: String,
    /// Whether this device is currently willing to receive files.
    pub download: bool,
    /// TCP port of the transfer server.
    pub port: u16,
}

impl Identity {
    /// Our own identity for `alias` (device model from the OS, port bound).
    pub fn new(alias: &str, port: u16) -> Identity {
        Identity {
            alias: alias.to_string(),
            version: crate::VERSION.to_string(),
            device_model: device_model(),
            device_type: device_type(),
            fingerprint: fingerprint(),
            download: true,
            port,
        }
    }

    /// Serialize to the v2 `info` JSON object.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"alias\":{},\"version\":{},\"deviceModel\":{},\"deviceType\":{},\"fingerprint\":{},\"download\":{},\"port\":{},\"protocolVersion\":\"{PROTOCOL_VERSION}\",\"ext\":\"{KETRA_EXT}\"}}",
            json::quote(&self.alias),
            json::quote(&self.version),
            json::quote(&self.device_model),
            json::quote(&self.device_type),
            json::quote(&self.fingerprint),
            self.download,
            self.port
        )
    }

    /// Parse from a v2 `info` JSON object.
    pub fn from_json(v: &Json) -> Option<Identity> {
        Some(Identity {
            alias: v.get("alias")?.as_str()?.to_string(),
            version: v.get("version").and_then(|j| j.as_str()).unwrap_or("").to_string(),
            device_model: v
                .get("deviceModel")
                .and_then(|j| j.as_str())
                .unwrap_or("")
                .to_string(),
            device_type: v
                .get("deviceType")
                .and_then(|j| j.as_str())
                .unwrap_or("desktop")
                .to_string(),
            fingerprint: v
                .get("fingerprint")
                .and_then(|j| j.as_str())
                .unwrap_or("")
                .to_string(),
            download: v.get("download").and_then(|j| j.as_bool()).unwrap_or(false),
            port: v.get("port").and_then(|j| j.as_u64()).unwrap_or(0) as u16,
        })
    }

    /// True when the peer speaks the Ketra accelerated extension.
    pub fn is_ketra(&self) -> bool {
        self.version.starts_with("ketra") || self.fingerprint == "ketra"
    }
}

/// One offered file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileMeta {
    /// Session-unique id (v2 `id`).
    pub id: String,
    /// Destination file name.
    pub file_name: String,
    /// Exact size in bytes.
    pub size: u64,
    /// MIME type (best effort, `application/octet-stream` fallback).
    pub file_type: String,
    /// SHA-256 hex digest when known (always set by Ketra senders).
    pub sha256: Option<String>,
}

impl FileMeta {
    pub fn to_json(&self) -> String {
        let sha = match &self.sha256 {
            Some(s) => json::quote(s),
            None => "null".to_string(),
        };
        format!(
            "{{\"id\":{},\"fileName\":{},\"size\":{},\"fileType\":{},\"sha256\":{}}}",
            json::quote(&self.id),
            json::quote(&self.file_name),
            self.size,
            json::quote(&self.file_type),
            sha
        )
    }

    pub fn from_json(v: &Json) -> Option<FileMeta> {
        Some(FileMeta {
            id: v.get("id").and_then(|j| j.as_str()).unwrap_or_default().to_string(),
            file_name: v.get("fileName").and_then(|j| j.as_str())?.to_string(),
            size: v.get("size").and_then(|j| j.as_u64()).unwrap_or(0),
            file_type: v
                .get("fileType")
                .and_then(|j| j.as_str())
                .unwrap_or("application/octet-stream")
                .to_string(),
            sha256: v
                .get("sha256")
                .and_then(|j| j.as_str())
                .map(str::to_string),
        })
    }
}

/// `POST /prepare-upload` request body.
#[derive(Debug, Clone)]
pub struct PrepareUpload {
    pub info: Identity,
    pub files: Vec<FileMeta>,
}

impl PrepareUpload {
    pub fn to_json(&self) -> String {
        let files: Vec<String> = self.files.iter().map(|f| f.to_json()).collect();
        format!(
            "{{\"info\":{},\"files\":{{{}}}}}",
            self.info.to_json(),
            files
                .iter()
                .enumerate()
                .map(|(i, f)| format!("\"f{i}\":{f}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn from_json(v: &Json) -> Option<PrepareUpload> {
        let info = Identity::from_json(v.get("info")?)?;
        let mut files = Vec::new();
        if let Some(Json::Obj(map)) = v.get("files") {
            for (k, fv) in map {
                let mut f = FileMeta::from_json(fv)?;
                if f.id.is_empty() {
                    f.id = k.clone();
                }
                files.push(f);
            }
        }
        Some(PrepareUpload { info, files })
    }
}

/// `POST /prepare-upload` response body.
#[derive(Debug, Clone)]
pub struct UploadSession {
    pub session_id: String,
    /// fileId → per-file token.
    pub tokens: BTreeMap<String, String>,
    /// Ketra extension: max parallel streams the receiver allows.
    pub max_streams: u32,
    /// Ketra extension: receiver verifies sha256 and expects /manifest.
    pub wants_manifest: bool,
}

impl UploadSession {
    pub fn to_json(&self) -> String {
        let toks: Vec<String> = self
            .tokens
            .iter()
            .map(|(k, v)| format!("{}:{}", json::quote(k), json::quote(v)))
            .collect();
        format!(
            "{{\"sessionId\":{},\"files\":{{{}}},\"maxStreams\":{},\"wantsManifest\":{}}}",
            json::quote(&self.session_id),
            toks.join(","),
            self.max_streams,
            self.wants_manifest
        )
    }

    pub fn from_json(v: &Json) -> Option<UploadSession> {
        let session_id = v.get("sessionId")?.as_str()?.to_string();
        let mut tokens = BTreeMap::new();
        if let Some(Json::Obj(map)) = v.get("files") {
            for (k, tv) in map {
                if let Some(t) = tv.as_str() {
                    tokens.insert(k.clone(), t.to_string());
                }
            }
        }
        Some(UploadSession {
            session_id,
            tokens,
            max_streams: v.get("maxStreams").and_then(|j| j.as_u64()).unwrap_or(1) as u32,
            wants_manifest: v.get("wantsManifest").and_then(|j| j.as_bool()).unwrap_or(false),
        })
    }
}

/// `POST /api/ketra/v1/manifest` request body (end-to-end integrity).
#[derive(Debug, Clone)]
pub struct ManifestCheck {
    pub session_id: String,
    /// fileId → sha256 of the bytes the sender actually streamed.
    pub digests: BTreeMap<String, String>,
}

impl ManifestCheck {
    pub fn to_json(&self) -> String {
        let items: Vec<String> = self
            .digests
            .iter()
            .map(|(k, v)| format!("{}:{}", json::quote(k), json::quote(v)))
            .collect();
        format!(
            "{{\"sessionId\":{},\"files\":{{{}}}}}",
            json::quote(&self.session_id),
            items.join(",")
        )
    }
}

/// `POST /api/ketra/v1/manifest` response body.
#[derive(Debug, Clone, Default)]
pub struct ManifestVerdict {
    /// fileId → "ok" | "mismatch" | "missing".
    pub results: BTreeMap<String, String>,
}

impl ManifestVerdict {
    pub fn all_ok(&self) -> bool {
        self.results.values().all(|v| v == "ok")
    }

    pub fn to_json(&self) -> String {
        let items: Vec<String> = self
            .results
            .iter()
            .map(|(k, v)| format!("{}:{}", json::quote(k), json::quote(v)))
            .collect();
        format!(
            "{{\"ok\":{},\"files\":{{{}}}}}",
            self.all_ok(),
            items.join(",")
        )
    }

    pub fn from_json(v: &Json) -> Option<ManifestVerdict> {
        let mut results = BTreeMap::new();
        if let Some(Json::Obj(map)) = v.get("files") {
            for (k, rv) in map {
                if let Some(r) = rv.as_str() {
                    results.insert(k.clone(), r.to_string());
                }
            }
        }
        Some(ManifestVerdict { results })
    }
}

/// MIME type by file extension (best effort, for v2 `fileType`).
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "md" | "log" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "xz" => "application/x-xz",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "apk" => "application/vnd.android.package-archive",
        "exe" => "application/vnd.microsoft.portable-executable",
        "deb" => "application/vnd.debian.binary-package",
        _ => "application/octet-stream",
    }
}

/// Deterministic per-installation fingerprint (host name + boot-ish salt).
fn fingerprint() -> String {
    use std::sync::OnceLock;
    static FP: OnceLock<String> = OnceLock::new();
    FP.get_or_init(|| {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "device".into());
        let mut seed = format!("ketra:{host}:{}", std::process::id());
        // Mix in a coarse time component so re-installs differ.
        seed.push_str(&format!(":{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)));
        // XXH64 (self-implemented, see hash.rs) → 16 hex chars.
        let h = crate::hash::xxh64(seed.as_bytes(), 0x4b455452 /* "KETR" */);
        format!("{h:016x}")
    })
    .clone()
}

fn device_model() -> String {
    if cfg!(target_os = "windows") {
        "Windows PC".into()
    } else if cfg!(target_os = "macos") {
        "Mac".into()
    } else if cfg!(target_os = "android") {
        "Android device".into()
    } else {
        "Linux PC".into()
    }
}

fn device_type() -> String {
    if cfg!(target_os = "android") {
        "mobile".into()
    } else {
        "desktop".into()
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_json_roundtrip() {
        let id = Identity {
            alias: "Ketra-Desk".into(),
            version: "1.1.0".into(),
            device_model: "Linux PC".into(),
            device_type: "desktop".into(),
            fingerprint: "abcd1234abcd1234".into(),
            download: true,
            port: 53317,
        };
        let j = id.to_json();
        let parsed = json::parse(&j).unwrap();
        let back = Identity::from_json(&parsed).unwrap();
        assert_eq!(back, id);
        assert!(j.contains("\"protocolVersion\":\"v2\""));
        assert!(j.contains("\"ext\":\"ketra-v1\""));
    }

    #[test]
    fn test_prepare_upload_roundtrip() {
        let p = PrepareUpload {
            info: Identity {
                alias: "phone".into(),
                version: "1.1.0".into(),
                device_model: "Android device".into(),
                device_type: "mobile".into(),
                fingerprint: "ff".into(),
                download: true,
                port: 53317,
            },
            files: vec![
                FileMeta {
                    id: "f0".into(),
                    file_name: "photo 1.jpg".into(),
                    size: 12345,
                    file_type: "image/jpeg".into(),
                    sha256: Some("aa".repeat(32)),
                },
                FileMeta {
                    id: "f1".into(),
                    file_name: "notes.txt".into(),
                    size: 7,
                    file_type: "text/plain".into(),
                    sha256: None,
                },
            ],
        };
        let j = p.to_json();
        let parsed = json::parse(&j).unwrap();
        let back = PrepareUpload::from_json(&parsed).unwrap();
        assert_eq!(back.files.len(), 2);
        assert_eq!(back.files[0].file_name, "photo 1.jpg");
        assert_eq!(
            back.files[0].sha256.as_deref(),
            Some("aa".repeat(32).as_str())
        );
        assert_eq!(back.files[1].id, "f1");
        assert_eq!(back.info.alias, "phone");
    }

    #[test]
    fn test_upload_session_roundtrip() {
        let mut tokens = BTreeMap::new();
        tokens.insert("f0".to_string(), "tok-abc".to_string());
        tokens.insert("f1".to_string(), "tok-def".to_string());
        let s = UploadSession {
            session_id: "s-1".into(),
            tokens,
            max_streams: 4,
            wants_manifest: true,
        };
        let j = s.to_json();
        let parsed = json::parse(&j).unwrap();
        let back = UploadSession::from_json(&parsed).unwrap();
        assert_eq!(back.session_id, "s-1");
        assert_eq!(back.tokens.get("f0").unwrap(), "tok-abc");
        assert_eq!(back.max_streams, 4);
        assert!(back.wants_manifest);
    }

    #[test]
    fn test_manifest_verdict() {
        let mut v = ManifestVerdict::default();
        v.results.insert("f0".into(), "ok".into());
        v.results.insert("f1".into(), "ok".into());
        assert!(v.all_ok());
        v.results.insert("f2".into(), "mismatch".into());
        assert!(!v.all_ok());
        let j = v.to_json();
        let parsed = json::parse(&j).unwrap();
        let back = ManifestVerdict::from_json(&parsed).unwrap();
        assert_eq!(back.results.get("f2").unwrap(), "mismatch");
    }

    #[test]
    fn test_fingerprint_is_stable_and_dotted_hex() {
        let a = fingerprint();
        let b = fingerprint();
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_mime_for() {
        assert_eq!(mime_for("x.PNG"), "image/png");
        assert_eq!(mime_for("a.b.tar"), "application/x-tar");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }
}
