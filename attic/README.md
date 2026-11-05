# attic — prototype crates kept for the Android track

These crates are **not** workspace members and do not build with the
current workspace manifests:

* `ketra-net/` — earlier LocalSend client/server built on rustls + mdns-sd +
  rcgen (external crates). Superseded by the std-only `ketra-core::lan`
  implementation shipped in KetraCopy 1.1.0.
* `ketra-ffi/` — UniFFI scaffolding for the Android app (needs `uniffi`
  plus the `ketra-net` crate). The Android project in `android/` builds
  against it with cargo-ndk; see `docs/ANDROID.md`.

They are kept for reference/history; the desktop release does not use them.
