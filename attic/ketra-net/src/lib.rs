//! # ketra-net — Local-network file transfer for KetraCopy
//!
//! Implements the open **LocalSend v2** protocol (mDNS `_localsend._tcp`
//! discovery + HTTPS REST API on port 53317, self-signed cert with
//! SHA-256 fingerprint) and a **Ketra accelerated extension** that is
//! negotiated when both endpoints are KetraCopy peers. The extension adds:
//!
//!   * parallel ranged upload streams (controlled by the AIMD throttle in
//!     `ketra-core`),
//!   * per-chunk SHA-256 / XXH64 / CRC32C integrity (Phase 2 inline verify,
//!     same primitives as the on-disk engine),
//!   * a Phase-3 manifest exchange so the receiver ends up with the same
//!     `<dest>/.ketra/manifest.jsonl` the desktop CLI writes.
//!
//! ## Layout
//!
//! | module      | responsibility                                         |
//! |-------------|--------------------------------------------------------|
//! | `identity`  | device identity, self-signed cert, fingerprint         |
//! | `discovery` | mDNS `_localsend._tcp` announce + browse              |
//! | `tls`       | rustls server + client config from the self-signed key |
//! | `http`      | minimal HTTP/1.1 server (request line, headers, body)  |
//! | `protocol`  | LocalSend v2 message schemas + Ketra extension fields  |
//! | `server`    | receive server (register/prepare-upload/upload/cancel) |
//! | `client`    | send client (register → prepare-upload → parallel upload) |
//! | `transfer`  | high-level `TransferJob` + `ProgressSink` trait        |
//!
//! ## Why these dependencies
//!
//! `ketra-core` stays std-only. Only `ketra-net` pulls in pure-Rust crates
//! that cross-compile cleanly to the Android NDK targets: `rustls` (TLS 1.2/1.3,
//! no OpenSSL), `mdns-sd` (no avahi/dbus), `rcgen` (self-signed cert generation,
//! ring backend). See `docs/LAN-PROTOCOL.md` for the wire format.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod client;
pub mod discovery;
pub mod http;
pub mod identity;
pub mod protocol;
pub mod server;
pub mod tls;
pub mod transfer;

pub use client::{SendClient, SendOptions};
pub use discovery::{Peer, PeerEvent, PeerRegistry};
pub use identity::{DeviceFingerprint, DeviceIdentity};
pub use protocol::{DeviceInfo, FileMetadata, FileType, KetraExtension, PrepareUploadRequest, PrepareUploadResponse, RegisterRequest};
pub use server::{ReceiveServer, ReceiveOptions};
pub use transfer::{ProgressEvent, ProgressSink, TransferJob, TransferOutcome};

// Re-export the mDNS daemon handle so downstream crates (ketra-ffi) can name
// the type without taking a direct dependency on mdns-sd.
pub use mdns_sd::ServiceDaemon;

/// The canonical LocalSend port (53317).
pub const LOCALSEND_PORT: u16 = 53317;
/// The mDNS service type used for discovery. mdns-sd requires the
/// fully-qualified `.<proto>.local.` form for `browse`; `ServiceInfo::new`
/// accepts it too (it normalizes internally).
pub const LOCALSEND_SERVICE: &str = "_localsend._tcp.local.";

/// Ketra's user-agent string on the LAN.
pub const KETRA_AGENT: &str = concat!("KetraCopy/", env!("CARGO_PKG_VERSION"));
