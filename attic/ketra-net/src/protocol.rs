//! Wire-format schemas for the LocalSend v2 protocol plus the Ketra
//! accelerated extension. All messages are JSON over HTTPS.
//!
//! ## LocalSend v2 endpoints (port 53317)
//!
//! | method | path                         | purpose                              |
//! |--------|------------------------------|--------------------------------------|
//! | POST   | `/api/localsend/v2/register` | sender announces; receiver responds with its info + accept/decline |
//! | POST   | `/api/localsend/v2/prepare-upload` | sender describes files; receiver returns `{ fileId: "accept"|"decline", sessionId }` |
//! | POST   | `/api/localsend/v2/upload?sessionId=&fileId=&token=` | sender streams the file body |
//! | POST   | `/api/localsend/v2/cancel?sessionId=` | either side cancels |
//! | GET    | `/api/localsend/v2/info`     | receiver's identity (no body needed) |
//!
//! ## Ketra extension
//!
//! When `info.ketra` is present and `ketra.accelerated == true` on **both**
//! peers, the prepare-upload response carries a `ketra` block describing the
//! receiver's preferred transfer mode. The upload endpoint then accepts
//! `Range:` headers for parallel chunked upload and a `X-Ketra-Checksum`
//! header (hex SHA-256 / XXH64 / CRC32C of the chunk) for inline verify.

use crate::identity::{DeviceFingerprint, DeviceIdentity, DeviceType};

/// Device info broadcast over mDNS and returned by `/info` and `/register`.
///
/// Fields beyond the LocalSend v2 spec are nested under `ketra`.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Human-friendly name.
    pub alias: String,
    /// Protocol version (2).
    pub version: u16,
    /// Device model.
    pub device_model: String,
    /// `mobile` / `desktop` / …
    pub device_type: DeviceType,
    /// SHA-256 fingerprint (hex).
    pub fingerprint: String,
    /// HTTPS port.
    pub port: u16,
    /// Whether this device accepts downloads (default true).
    pub download: bool,
    /// Ketra extension block, omitted for plain LocalSend peers.
    pub ketra: Option<KetraExtension>,
}

/// The Ketra accelerated extension block, advertised under `info.ketra`.
/// Present only on KetraCopy peers.
#[derive(Debug, Clone, Default)]
pub struct KetraExtension {
    /// True when the peer supports parallel ranged upload streams + chunk
    /// checksums. When false we fall back to plain sequential LocalSend v2.
    pub accelerated: bool,
    /// Preferred hash for chunk verification: `"sha256"` (default),
    /// `"xxh64"`, or `"crc32c"`.
    pub checksum: String,
    /// Max parallel streams the receiver will accept (1..=8).
    pub parallel_streams: u8,
    /// Receiver supports `Range:`-based resume on `/upload`.
    pub resume_supported: bool,
    /// Receiver wants the final manifest (`manifest.jsonl`) uploaded as the
    /// last "file" of the session, named `KetraCopy-manifest.jsonl`.
    pub manifest_exchange: bool,
}

impl DeviceInfo {
    /// Build the info block for our own identity.
    pub fn from_identity(id: &DeviceIdentity, ketra: KetraExtension) -> Self {
        Self {
            alias: id.alias.clone(),
            version: id.version,
            device_model: id.model.clone(),
            device_type: id.device_type,
            fingerprint: id.fingerprint.hex(),
            port: id.port,
            download: true,
            ketra: Some(ketra),
        }
    }

    /// Serialize to JSON matching the LocalSend v2 schema.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        s.push_str(&format!("\"alias\":{},", json_str(&self.alias)));
        s.push_str(&format!("\"version\":{},", self.version));
        s.push_str(&format!("\"deviceModel\":{},", json_str(&self.device_model)));
        s.push_str(&format!("\"deviceType\":{},", json_str(self.device_type.as_str())));
        s.push_str(&format!("\"fingerprint\":{},", json_str(&self.fingerprint)));
        s.push_str(&format!("\"port\":{},", self.port));
        s.push_str(&format!("\"download\":{}", self.download));
        if let Some(k) = &self.ketra {
            s.push_str(",\"ketra\":{");
            s.push_str(&format!("\"accelerated\":{},", k.accelerated));
            s.push_str(&format!("\"checksum\":{},", json_str(&k.checksum)));
            s.push_str(&format!("\"parallelStreams\":{},", k.parallel_streams));
            s.push_str(&format!("\"resumeSupported\":{},", k.resume_supported));
            s.push_str(&format!("\"manifestExchange\":{}", k.manifest_exchange));
            s.push('}');
        }
        s.push('}');
        s
    }

    /// Parse a JSON DeviceInfo (from `/info` or `/register`). Tolerant of
    /// missing fields except `alias`, `fingerprint`, `port`.
    pub fn parse(s: &str) -> Option<Self> {
        let alias = json_str_field(s, "alias")?;
        let fingerprint = json_str_field(s, "fingerprint")?;
        let port = json_num_field(s, "port").map(|v| v as u16).unwrap_or(53317);
        let version = json_num_field(s, "version").map(|v| v as u16).unwrap_or(2);
        let device_model = json_str_field(s, "deviceModel").unwrap_or_default();
        let device_type = json_str_field(s, "deviceType")
            .as_deref()
            .and_then(DeviceType::parse)
            .unwrap_or(DeviceType::Desktop);
        let download = json_bool_field(s, "download").unwrap_or(true);
        let ketra = if s.contains("\"ketra\"") {
            Some(KetraExtension {
                accelerated: json_bool_field(s, "accelerated").unwrap_or(false),
                checksum: json_str_field(s, "checksum").unwrap_or_else(|| "sha256".into()),
                parallel_streams: json_num_field(s, "parallelStreams").map(|v| v as u8).unwrap_or(1),
                resume_supported: json_bool_field(s, "resumeSupported").unwrap_or(false),
                manifest_exchange: json_bool_field(s, "manifestExchange").unwrap_or(false),
            })
        } else {
            None
        };
        Some(Self { alias, version, device_model, device_type, fingerprint, port, download, ketra })
    }
}

/// LocalSend `FileType` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Image,
    Video,
    Text,
    Audio,
    Other,
}

impl FileType {
    pub fn as_str(self) -> &'static str {
        match self {
            FileType::File => "file",
            FileType::Image => "image",
            FileType::Video => "video",
            FileType::Text => "text",
            FileType::Audio => "audio",
            FileType::Other => "other",
        }
    }
    pub fn from_ext(filename: &str) -> Self {
        let lower = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if matches!(lower.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp") {
            FileType::Image
        } else if matches!(lower.as_str(), "mp4" | "mov" | "mkv" | "avi" | "webm") {
            FileType::Video
        } else if matches!(lower.as_str(), "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a") {
            FileType::Audio
        } else if matches!(lower.as_str(), "txt" | "md" | "json" | "yaml" | "yml" | "csv" | "log") {
            FileType::Text
        } else {
            FileType::File
        }
    }
}

/// Metadata for one file in a `prepare-upload` request.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Sender-assigned id (string, unique within the session).
    pub id: String,
    /// Destination filename (no path; receiver places under its upload dir).
    pub filename: String,
    /// Size in bytes.
    pub size: u64,
    /// LocalSend file type.
    pub file_type: FileType,
    /// Optional SHA-256 of the whole file (hex). When present, the receiver
    /// verifies the completed upload against it.
    pub sha256: Option<String>,
    /// Optional thumbnail / preview (base64 data URL). Omitted for non-media.
    pub preview: Option<String>,
    /// Ketra extension: chunk size the sender will use (bytes). 0 = sequential.
    pub ketra_chunk_size: u64,
}

impl FileMetadata {
    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        s.push_str(&format!("\"id\":{},", json_str(&self.id)));
        s.push_str(&format!("\"fileName\":{},", json_str(&self.filename)));
        s.push_str(&format!("\"size\":{},", self.size));
        s.push_str(&format!("\"fileType\":{}", json_str(self.file_type.as_str())));
        if let Some(h) = &self.sha256 {
            s.push_str(&format!(",\"sha256\":{}", json_str(h)));
        }
        if let Some(p) = &self.preview {
            s.push_str(&format!(",\"preview\":{}", json_str(p)));
        }
        if self.ketra_chunk_size > 0 {
            s.push_str(&format!(",\"ketra\":{{\"chunkSize\":{}}}", self.ketra_chunk_size));
        }
        s.push('}');
        s
    }
}

/// `POST /register` body — sender's identity + intent.
#[derive(Debug, Clone)]
pub struct RegisterRequest {
    pub info: DeviceInfo,
}

/// `POST /prepare-upload` body — sender describes what it wants to send.
#[derive(Debug, Clone)]
pub struct PrepareUploadRequest {
    pub info: DeviceInfo,
    /// Map of `fileId -> FileMetadata`.
    pub files: Vec<FileMetadata>,
}

impl PrepareUploadRequest {
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\"info\":");
        s.push_str(&self.info.to_json());
        s.push_str(",\"files\":{");
        for (i, f) in self.files.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&json_str(&f.id));
            s.push(':');
            s.push_str(&f.to_json());
        }
        s.push_str("}}");
        s
    }
}

/// `POST /prepare-upload` response.
///
/// ```jsonc
/// { "sessionId": "abc", "files": { "1": "accept", "2": "decline" } }
/// ```
/// When both peers are Ketra and accelerated, an extra `ketra` block is
/// included to finalize the transfer mode.
#[derive(Debug, Clone)]
pub struct PrepareUploadResponse {
    pub session_id: String,
    /// Per-file accept/decline.
    pub files: Vec<(String, bool)>,
    /// Negotiated Ketra extension (only when accelerated).
    pub ketra: Option<KetraExtension>,
}

impl PrepareUploadResponse {
    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        s.push_str(&format!("\"sessionId\":{},", json_str(&self.session_id)));
        s.push_str("\"files\":{");
        for (i, (id, accept)) in self.files.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&json_str(id));
            s.push(':');
            s.push_str(if *accept { "\"accept\"" } else { "\"decline\"" });
        }
        s.push('}');
        if let Some(k) = &self.ketra {
            s.push_str(",\"ketra\":{");
            s.push_str(&format!("\"accelerated\":{},", k.accelerated));
            s.push_str(&format!("\"checksum\":{},", json_str(&k.checksum)));
            s.push_str(&format!("\"parallelStreams\":{},", k.parallel_streams));
            s.push_str(&format!("\"resumeSupported\":{},", k.resume_supported));
            s.push_str(&format!("\"manifestExchange\":{}", k.manifest_exchange));
            s.push('}');
        }
        s.push('}');
        s
    }

    pub fn parse(s: &str) -> Option<Self> {
        let session_id = json_str_field(s, "sessionId")?;
        // files is a flat map of {id: "accept"|"decline"}
        let mut files = Vec::new();
        if let Some(start) = s.find("\"files\"") {
            if let Some(lbrace) = s[start..].find('{') {
                let body = &s[start + lbrace..];
                if let Some(rbrace) = body.find('}') {
                    let body = &body[1..rbrace];
                    // Scan for `"key":"value"` pairs — only count a quoted
                    // string as a key when it's immediately followed by `:`.
                    let bytes = body.as_bytes();
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i] != b'"' {
                            i += 1;
                            continue;
                        }
                        let key_start = i + 1;
                        let mut j = key_start;
                        while j < bytes.len() && bytes[j] != b'"' {
                            if bytes[j] == b'\\' {
                                j += 1;
                            }
                            j += 1;
                        }
                        if j >= bytes.len() {
                            break;
                        }
                        let key = &body[key_start..j];
                        let mut k = j + 1;
                        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        if k >= bytes.len() || bytes[k] != b':' {
                            i = j + 1;
                            continue;
                        }
                        k += 1;
                        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        if k < bytes.len() && bytes[k] == b'"' {
                            let val_start = k + 1;
                            let mut v = val_start;
                            while v < bytes.len() && bytes[v] != b'"' {
                                v += 1;
                            }
                            if v < bytes.len() {
                                let val = &body[val_start..v];
                                files.push((key.to_string(), val == "accept"));
                                i = v + 1;
                                continue;
                            }
                        }
                        i = j + 1;
                    }
                }
            }
        }
        let ketra = if s.contains("\"ketra\"") {
            Some(KetraExtension {
                accelerated: json_bool_field(s, "accelerated").unwrap_or(false),
                checksum: json_str_field(s, "checksum").unwrap_or_else(|| "sha256".into()),
                parallel_streams: json_num_field(s, "parallelStreams").map(|v| v as u8).unwrap_or(1),
                resume_supported: json_bool_field(s, "resumeSupported").unwrap_or(false),
                manifest_exchange: json_bool_field(s, "manifestExchange").unwrap_or(false),
            })
        } else {
            None
        };
        Some(Self { session_id, files, ketra })
    }
}

// ---- tiny JSON helpers (no serde; ketra-core::json could also be used) ----

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_str_field(s: &str, field: &str) -> Option<String> {
    let key = format!("\"{}\"", field);
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let q = rest.find('"')?;
    let rest = &rest[q + 1..];
    // find unescaped closing quote
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
    let raw = &rest[..end];
    // unescape basic sequences
    Some(raw.replace("\\\"", "\"").replace("\\\\", "\\").replace("\\n", "\n").replace("\\r", "\r").replace("\\t", "\t"))
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

fn json_bool_field(s: &str, field: &str) -> Option<bool> {
    let key = format!("\"{}\"", field);
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    fn dummy_id() -> DeviceIdentity {
        DeviceIdentity {
            alias: "Test".into(),
            model: "X".into(),
            device_type: DeviceType::Desktop,
            version: 2,
            port: 53317,
            cert_pem: String::new(),
            key_pem: String::new(),
            fingerprint: DeviceFingerprint("ab-cd".into()),
        }
    }

    #[test]
    fn device_info_round_trips() {
        let id = dummy_id();
        let info = DeviceInfo::from_identity(&id, KetraExtension { accelerated: true, checksum: "sha256".into(), parallel_streams: 4, resume_supported: true, manifest_exchange: true });
        let json = info.to_json();
        let parsed = DeviceInfo::parse(&json).unwrap();
        assert_eq!(parsed.alias, "Test");
        assert_eq!(parsed.port, 53317);
        assert!(parsed.ketra.is_some());
        let k = parsed.ketra.unwrap();
        assert!(k.accelerated);
        assert_eq!(k.parallel_streams, 4);
    }

    #[test]
    fn prepare_response_round_trips() {
        let r = PrepareUploadResponse {
            session_id: "s1".into(),
            files: vec![("1".into(), true), ("2".into(), false)],
            ketra: Some(KetraExtension { accelerated: true, checksum: "xxh64".into(), parallel_streams: 2, resume_supported: false, manifest_exchange: false }),
        };
        let json = r.to_json();
        let parsed = PrepareUploadResponse::parse(&json).unwrap();
        assert_eq!(parsed.session_id, "s1");
        assert_eq!(parsed.files.len(), 2);
        assert!(parsed.files[0].1);
        assert!(!parsed.files[1].1);
        let k = parsed.ketra.unwrap();
        assert_eq!(k.checksum, "xxh64");
    }

    #[test]
    fn file_type_classification() {
        assert_eq!(FileType::from_ext("photo.jpg"), FileType::Image);
        assert_eq!(FileType::from_ext("clip.mp4"), FileType::Video);
        assert_eq!(FileType::from_ext("readme.md"), FileType::Text);
        assert_eq!(FileType::from_ext("archive.zip"), FileType::File);
    }

    #[test]
    fn plain_localsend_peer_parses_without_ketra() {
        let raw = r#"{"alias":"Bob","version":2,"deviceModel":"iPhone","deviceType":"mobile","fingerprint":"abc","port":53317,"download":true}"#;
        let info = DeviceInfo::parse(raw).unwrap();
        assert_eq!(info.alias, "Bob");
        assert!(info.ketra.is_none());
    }
}
