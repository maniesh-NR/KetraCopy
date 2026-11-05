//! ketra-core — the KetraCopy engine.
//!
//! Clean-room implementation of a TeraCopy-style copy accelerator.
//! `std`-only: no external crates are linked, platform features are reached
//! through hand-declared FFI in [`platform`].
//!
//! The engine executes the three-phase pipeline used by KetraCopy:
//!
//! 1. **Fast scan** — compare `size + mtime` of source and destination to
//!    skip files that are already up to date.
//! 2. **Stream copy** — chunked streaming copy with optional inline
//!    byte-for-byte comparison, reflink / `copy_file_range` fast paths,
//!    sparse-aware writing, throttling and resumable journals.
//! 3. **Archive manifest** — optional SHA-256 manifest of every copied file
//!    for later integrity verification.
//!
//! Cancellation is cooperative through [`std::sync::AtomicBool`] and the
//! [`Error::Canceled`] variant; a canceled file keeps its partial bytes and
//! journal offset so a later `--resume` run can continue it.

pub mod aimd;
pub mod controller;
pub mod device;
pub mod engine;
pub mod error;
pub mod glob;
pub mod hash;
pub mod journal;
pub mod json;
pub mod lan;
pub mod manifest;
pub mod netmode;
pub mod options;
pub mod pipeline;
pub mod plan;
pub mod platform;
pub mod progress;
pub mod resume;
pub mod settings;
pub mod throttle;

/// Android JNI bridge (compiled only for `target_os = "android"`).
#[cfg(target_os = "android")]
pub mod jni;

/// Crate version, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default chunk size (1 MiB) used for streaming copies.
pub const DEFAULT_CHUNK: u64 = 1024 * 1024;
