//! Local-network file transfer (LocalSend-compatible) — feature introduced
//! in KetraCopy 1.1.0.
//!
//! Layers, bottom-up:
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`mdns`] | DNS-SD discovery (`_localsend._tcp.local`, `_ketra._tcp.local`) over multicast DNS, std-only FFI |
//! | [`http`] | minimal HTTP/1.1 client + threaded server with **streaming** bodies |
//! | [`protocol`] | LocalSend v2 message types (JSON) + the Ketra accelerated extension |
//! | [`server`] | the receiver: routes, sessions, tokens, parallel-range writes, digests |
//! | [`client`] | the sender: session negotiation, single/parallel uploads, end-to-end SHA-256 |
//!
//! Security model (documented honestly in `docs/ARCHITECTURE.md`): this
//! implementation speaks plain HTTP/1.1 like LocalSend does on trusted LANs,
//! but adds **end-to-end SHA-256 manifests** so both sides can prove the
//! bytes that arrived are the bytes that were sent. TLS termination and
//! AES session pinning are roadmap items (`SECURITY.md`); until then, only
//! use this on networks you trust.
//!
//! Quick start (CLI):
//!
//! ```text
//! ketra receive --dest-dir ~/Incoming     # terminal A
//! ketra peers                             # terminal B, on another machine
//! ketra send photos.zip --to laptop       # terminal B
//! ```

pub mod client;
pub mod http;
pub mod mdns;
pub mod protocol;
pub mod server;

pub use client::{send_files, SendConfig, SendReport};
pub use mdns::{local_ip, Discovered, Mdns};
pub use protocol::{FileMeta, Identity};
pub use server::{discover, ReceivedFile, Receiver, ReceiverConfig, ReceiverHooks};

/// Default port (the LocalSend convention).
pub const LAN_PORT: u16 = 53317;

/// Make a user-supplied file name safe to store: keeps only the final
/// path component (any `/` or `\` content is dropped), replaces reserved
/// characters, strips control characters, and prefixes Windows reserved
/// device names.
pub struct SecureName;

/// Format object that sanitizes file names (see [`SECURE_NAME`]).
impl SecureName {
    /// Return a sanitized version of `name`: only the **final path
    /// component** is kept (any `/` or `\` content is dropped), reserved
    /// characters are replaced and control characters removed.
    pub fn replace(&self, name: &str) -> String {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let mut out = String::with_capacity(base.len());
        for c in base.chars() {
            match c {
                ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
                c if (c as u32) < 0x20 => {}
                c => out.push(c),
            }
        }
        let trimmed = out.trim().trim_matches('.').to_string();
        if trimmed.is_empty() {
            return "unnamed".to_string();
        }
        // Windows reserved device names (CON, PRN, AUX, NUL, COM1-9,
        // LPT1-9 — with or without an extension) are unusable on FAT/NTFS
        // destinations; prefix them so received files stay extractable
        // cross-platform.
        if is_windows_reserved(&trimmed) {
            format!("_{trimmed}")
        } else {
            trimmed
        }
    }
}

/// Windows reserved device-name check on the stem before the first dot
/// (`CON`, `con.txt`, `COM1`, `lpt9.tar.gz` …; `COM0`/`LPT0` are NOT
/// reserved).
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("");
    if stem.is_empty() {
        return false;
    }
    let up = stem.to_ascii_uppercase();
    match up.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let b = up.as_bytes();
            b.len() == 4
                && (up.starts_with("COM") || up.starts_with("LPT"))
                && (b'1'..=b'9').contains(&b[3])
        }
    }
}

/// Sanitizer singleton used by the receiver.
pub static SECURE_NAME: SecureName = SecureName;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secure_name() {
        assert_eq!(SECURE_NAME.replace("photo 1.jpg"), "photo 1.jpg");
        // Path traversal is neutralized: only the final component survives.
        assert_eq!(SECURE_NAME.replace("../../etc/passwd"), "passwd");
        // Only '/' and '\' split components; reserved characters are replaced.
        assert_eq!(SECURE_NAME.replace("a/b\\c:d*e?f\"g<h>i|j"), "c_d_e_f_g_h_i_j");
        assert_eq!(SECURE_NAME.replace("..."), "unnamed");
        assert_eq!(SECURE_NAME.replace(""), "unnamed");
        assert_eq!(SECURE_NAME.replace("  spaced  "), "spaced");
        // Windows reserved device names are prefixed, not dropped.
        assert_eq!(SECURE_NAME.replace("CON"), "_CON");
        assert_eq!(SECURE_NAME.replace("con.txt"), "_con.txt");
        assert_eq!(SECURE_NAME.replace("COM9"), "_COM9");
        assert_eq!(SECURE_NAME.replace("lpt1.tar.gz"), "_lpt1.tar.gz");
        assert_eq!(SECURE_NAME.replace("aux"), "_aux");
        // COM0/LPT0 are not reserved; normal names stay untouched.
        assert_eq!(SECURE_NAME.replace("com0.bin"), "com0.bin");
        assert_eq!(SECURE_NAME.replace("console.txt"), "console.txt");
        let ctrl = format!("bad{}\nname", 0x01 as char);
        assert_eq!(SECURE_NAME.replace(&ctrl), "badname");
    }

    #[test]
    fn test_lan_port_matches_localsend() {
        assert_eq!(LAN_PORT, 53317);
    }
}
