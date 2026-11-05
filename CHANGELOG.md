# Changelog

## 1.1.0 — 2026-11-05

### Added

* **OS shell integration** (`ketra shell install|uninstall|status`):
  * Windows Explorer context menus — Copy / Cut / Paste here / Copy to
    Folder… / Move to Folder… / Test — plus an optional AutoHotkey v2
    helper for Ctrl+Shift+C/X/V keyboard shortcuts.
  * Linux file-manager menus for Nemo (actions), Nautilus, Caja,
    Dolphin (service menu) and Thunar (custom actions), including
    *Paste Here*; `ketra-pick` destination picker; optional Cinnamon
    Super+V smart paste.
  * macOS Finder Quick Actions (Services): Copy / Cut / Move… / Paste
    with folder chooser.
* **Clipboard engine** (`ketra clip put|get|paste`): native
  copy/cut/paste/move takeover on all three desktops. Windows:
  `CF_HDROP` + `Preferred DropEffect` via FFI; Linux: X11 selection
  owner daemon (runtime dlopen, std-only) speaking `text/uri-list`,
  `text/x-special/gnome-copied-files` and `application/x-kde-cutselection`;
  macOS: AppleScript `POSIX file` bridge. Codecs are unit-tested on every
  platform.
* **LAN file transfer** (`ketra peers|send|receive`): std-only
  LocalSend v2 protocol + Ketra accelerated extension — mDNS discovery
  (`_localsend._tcp.local`), HTTP/1.1 receiver with token-confirmed
  sessions, parallel ranged streams, end-to-end SHA-256 verification.
* **GUI**: rebuilt Material Design 3 shell (`ketra-gui` + `ketra-md3`
  design system) with Transfer, LAN, Settings (persisted defaults +
  one-click shell integration) and About tabs; live progress, cancel,
  clipboard paste button.
* **Android track**: JNI bridge in `ketra-core` (android-only), Kotlin/
  Compose file manager project under `android/`, prototype crates moved
  to `attic/`.

### Fixed

* **Sparse files were written densely**: the hole scanner dropped mid-file
  holes (only leading/trailing ones survived), and `next_hole` ended every
  hole at EOF. A 32 MiB file with two 4 KiB data regions now transfers
  ~8 KiB and lands with identical block allocation (`stat -c %b`) instead
  of ~24 MiB of zero-filled extents.
* Manifest/verify/read-test behaviour aligned with the engine's verify
  levels (`off | inline | manifest`); `ketra test` no longer needs
  `--dest`.

### Changed

* Directory sources mirror their *contents* into the destination root
  (TeraCopy "paste into folder" semantics), documented in `README.md`.
* CLI is reconciled with the 1.1.0 engine API: renamed verify levels,
  conflict callback plumbing, JSONL progress passthrough, `--net-streams`
  parallel ranged reads, `--no-clone`, `--sparse on|off`.
* Engine grew 81 unit tests; 39 end-to-end integration checks in
  `scripts/test_integration.sh`.

### Verified

* `cargo test --workspace`: 124 unit tests green (Linux x86-64).
* Integration suite: 39/39 green.
* Cross-built and smoke-linked: Windows x86-64 + i686 (CLI and GUI),
  Linux x86-64 (CLI and GUI); macOS x86-64 + aarch64 type-check clean.
* `.deb`, portable bundle and per-target archives rebuilt for 1.1.0.

## 1.0.0 — 2026-11-02

* Initial clean-room release: three-phase engine (fast scan, stream copy
  with inline verify, SHA-256 manifest), resumable journals, token-bucket
  + AIMD throttling, sparse handling, conflict policies, filters, flatten,
  bench, manifest verify; CLI + MD3 GUI skeleton; packaging assets;
  35 engine unit tests + 22 integration checks.
