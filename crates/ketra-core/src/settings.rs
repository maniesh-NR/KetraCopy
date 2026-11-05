//! Persistent settings (`ketracopy.json`).

use crate::error::{Error, Result};
use crate::json::{self, Json};
use std::path::Path;

/// User-editable defaults shared by CLI and GUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// Default verify mode: "off" | "inline" | "manifest".
    pub verify: Option<String>,
    /// Speed limit in bytes/sec (None = unlimited).
    pub speed_limit_bps: Option<u64>,
    /// Streaming chunk size in bytes.
    pub buffer: Option<usize>,
    /// Reflink fast path (default true).
    pub use_clone: Option<bool>,
    /// Direct I/O (default false).
    pub direct_io: Option<bool>,
    /// GUI theme: "light" | "dark" | "system".
    pub theme: Option<String>,
}

impl Settings {
    /// Load settings from `path`; missing file → defaults.
    pub fn load(path: &Path) -> Result<Settings> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default())
            }
            Err(e) => return Err(Error::ioctx(path, e)),
        };
        let v = json::parse(&text).map_err(Error::Config)?;
        let mut s = Settings::default();
        s.verify = v.get("verify").and_then(|j| j.as_str()).map(str::to_string);
        s.speed_limit_bps = v.get("speed_limit_bps").and_then(|j| j.as_u64());
        s.buffer = v.get("buffer").and_then(|j| j.as_u64()).map(|n| n as usize);
        s.use_clone = v.get("use_clone").and_then(|j| j.as_bool());
        s.direct_io = v.get("direct_io").and_then(|j| j.as_bool());
        s.theme = v.get("theme").and_then(|j| j.as_str()).map(str::to_string);
        Ok(s)
    }

    /// Save settings to `path`.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| Error::ioctx(parent, e))?;
            }
        }
        let mut s = String::from("{\n");
        if let Some(v) = &self.verify {
            s.push_str(&format!("  \"verify\": {},\n", json::quote(v)));
        }
        if let Some(r) = self.speed_limit_bps {
            s.push_str(&format!("  \"speed_limit_bps\": {r},\n"));
        }
        if let Some(b) = self.buffer {
            s.push_str(&format!("  \"buffer\": {b},\n"));
        }
        if let Some(c) = self.use_clone {
            s.push_str(&format!("  \"use_clone\": {c},\n"));
        }
        if let Some(d) = self.direct_io {
            s.push_str(&format!("  \"direct_io\": {d},\n"));
        }
        if let Some(t) = &self.theme {
            s.push_str(&format!("  \"theme\": {},\n", json::quote(t)));
        }
        s.push_str("  \"kind\": \"ketracopy-settings\"\n}\n");
        std::fs::write(path, s).map_err(|e| Error::ioctx(path, e))
    }

    /// Apply stored settings on top of default options.
    pub fn apply_to(&self, o: &mut crate::options::Options) {
        match self.verify.as_deref() {
            Some("inline") => o.verify = crate::options::VerifyMode::Inline,
            Some("manifest") => o.verify = crate::options::VerifyMode::Manifest,
            _ => {}
        }
        if let Some(r) = self.speed_limit_bps {
            o.speed_limit_bps = Some(r);
        }
        if let Some(b) = self.buffer {
            o.buffer = b;
        }
        if let Some(c) = self.use_clone {
            o.use_clone = c;
        }
        if let Some(d) = self.direct_io {
            o.direct_io = d;
        }
    }

    /// Platform default config path.
    pub fn default_path() -> std::path::PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
            return std::path::PathBuf::from(dir).join("ketracopy").join("ketracopy.json");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home)
                .join(".config")
                .join("ketracopy")
                .join("ketracopy.json");
        }
        std::path::PathBuf::from("ketracopy.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tmp("rt");
        let p = dir.join("ketracopy.json");
        let s = Settings {
            verify: Some("inline".into()),
            speed_limit_bps: Some(10 * 1024 * 1024),
            buffer: Some(262_144),
            use_clone: Some(false),
            direct_io: Some(true),
            theme: Some("dark".into()),
        };
        s.save(&p).unwrap();
        let back = Settings::load(&p).unwrap();
        assert_eq!(back, s);

        let mut o = crate::options::Options::default();
        back.apply_to(&mut o);
        assert_eq!(o.verify, crate::options::VerifyMode::Inline);
        assert_eq!(o.speed_limit_bps, Some(10 * 1024 * 1024));
        assert!(!o.use_clone && o.direct_io);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_missing_file_gives_defaults() {
        let dir = tmp("missing");
        let s = Settings::load(&dir.join("nope.json")).unwrap();
        assert_eq!(s, Settings::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_malformed_file_is_config_error() {
        let dir = tmp("bad");
        let p = dir.join("ketracopy.json");
        std::fs::write(&p, "{not json").unwrap();
        assert!(Settings::load(&p).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
