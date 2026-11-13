//! # ketra-ffi — UniFFI bindings for KetraCopy
//!
//! Exposes the `ketra-core` engine and the `ketra-net` LAN transfer layer to
//! Kotlin via UniFFI's proc-macro scaffolding. `uniffi-bindgen generate`
//! (called by the Gradle build and by the GitHub Actions workflow) produces
//! the matching `Ketra.kt` facade that the Android app calls into.
//!
//! ## Shape
//!
//! * `KetraEngine` — a single `#[derive(uniffi::Object)]` struct the Android
//!   UI constructs once per app session. Holds the device identity, the mDNS
//!   daemon handle, the peer registry, and (optionally) a running receive
//!   server.
//! * `ProgressCallback` — a `#[uniffi::export(callback_interface)]` trait
//!   that Kotlin implements; Rust worker threads call `on_event` to stream
//!   progress back to the UI.
//! * `Record` dictionaries — `EngineOptions`, `PeerInfo`, `ProgressEvent`,
//!   `TransferResult`.
//!
//! See `docs/ANDROID.md` for the build pipeline (cargo-ndk cross-compile +
//! uniffi-bindgen + Gradle assembleDebug).

#![allow(clippy::needless_lifetimes)]

uniffi::setup_scaffolding!("ketra");

pub mod facade;
