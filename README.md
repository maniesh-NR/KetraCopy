# KetraCopy

KetraCopy is a clean-room, TeraCopy-style file-transfer engine for desktop,
extended in 1.1.1 with **LocalSend-compatible LAN transfer**, **OS clipboard
integration** (Explorer / Finder / Nemo / Nautilus / Caja / Dolphin /
Thunar) and a **Material Design 3 GUI**. The engine and CLI are
zero-external-dependency Rust crates; the GUI uses only `egui`/`eframe`.

Version 1.2.0 is a **performance / stability / memory optimization release**:
streaming XXH64 & CRC32C (O(1) memory — hashing no longer holds the file in
RAM), inline verification without the extra source re-read, inline SHA-256
manifest hashing (`--verify manifest` reads each byte exactly once — no
second read pass), batched resume journals (32 MiB stride, first-chunk and
cancel anchors keep `--resume` exact), per-chunk kernel readahead,
`fdatasync` finalization, transient-I/O retries (`--retries`, also on the
multi-stream network path), parallel file-level copying (`--parallel`,
opt-in for sync-heavy devices), slice-by-8 CRC32C, and a rolling-window
SHA-256 schedule — 14× faster on many-small-file copies, 5× faster xxh64
hashing, single-digit-MiB RSS across the board (same-session paired
measurements; see CHANGELOG).

No TeraCopy code, strings or assets were used — see `docs/CLEANROOM.md`.

## Feature matrix

| Component | Status |
|---|---|
| Engine (`ketra-core`) | Complete — 85 unit tests + 39 integration checks pass on Linux x86-64 |
| CLI (`ketra`) | Complete — 20 unit tests; copy/move/test, manifest/verify, bench, **clip** (clipboard), **shell** (integration), **peers/send/receive** (LAN) |
| LAN transfer (`ketra-core::lan`) | std-only LocalSend v2 protocol + Ketra accelerated extension (mDNS discovery, HTTP/1.1, parallel ranged streams, end-to-end SHA-256); see `docs/LAN-PROTOCOL.md` |
| Clipboard / shell integration | Windows `CF_HDROP`+`Preferred DropEffect` (FFI), Linux X11 selection owner (runtime `dlopen`), macOS AppleScript bridge; `ketra shell install` wires context menus + shortcuts per OS — see `SHELL-INTEGRATION.md` |
| GUI (`ketra-md3` 24 tests, `ketra-gui`) | eframe 0.28 / glow, MD3 components; Transfer / LAN / Settings / About tabs; built for Linux + Windows x64/x86 |
| Windows binaries | Built (x86-64 + i686, windows-gnu toolchain); not runtime-tested in this environment |
| macOS | Type-checks clean for x86-64 + aarch64; binaries are built by the GitHub Actions release workflow (no Apple SDK in the dev sandbox) |
| Packaging | deb, portable Linux bundle, per-target archives, Inno Setup script, Flatpak manifest, macOS .app/DMG scripts |

## Quick start

```sh
cargo build --release -p ketra-cli
./target/release/ketra copy ~/Photos -d /mnt/backup --verify inline
./target/release/ketra manifest --root /mnt/backup          # SHA-256 manifest
./target/release/ketra verify --root /mnt/backup            # detect bit rot
./target/release/ketra bench --size 128M
```

`ketra-core` and `ketra-cli` never touch crates.io. The GUI crates pin
`egui`/`eframe` at 0.28.1.

## The engine (per file)

```text
plan::build (plan.rs)  scan sources, filters, conflict-free mapping
        |
        v
engine::Engine::copy (engine.rs)
 +--------------------------------------------------------------+
 | Phase 1  fast scan: same size + mtime -> skip (up_to_date)   |
 |          else apply conflict policy (overwrite / newer /     |
 |          skip / rename / ask via callback)                   |
 +--------------------------------------------------------------+
 | Phase 2  stream copy: chunked, inline byte-compare verify,   |
 |          reflink (FICLONE/clonefile) + copy_file_range fast  |
 |          paths, sparse hole-preserving writes (SEEK_DATA/    |
 |          SEEK_HOLE scan; holes are skipped, never written),  |
 |          token-bucket + AIMD throttling, resumable journal   |
 |          (<dst>/.ketracopy-journal.jsonl)                    |
 +--------------------------------------------------------------+
 | Phase 3  SHA-256 manifest at <dst>/.ketra-manifest.json      |
 |          (verify mode) or written by `ketra manifest`        |
 +--------------------------------------------------------------+
```

Directory sources mirror their **contents** into the destination root
(`S/a/b.txt` → `dst/a/b.txt`), like TeraCopy's "paste into folder" flow.

## CLI cheat sheet

```text
ketra copy|move <src>... -d <dest> [options]
  options: --verify off|inline|manifest   integrity strategy (default inline)
           --limit <rate>                 speed cap (e.g. 30m)
           --parallel N                   concurrent file workers, 1..=16
                                          (default 1; use 2-4 on HDD/USB/
                                          network destinations where per-file
                                          sync is slow)
           --retries N                    transient-I/O retry budget (default 2:
                                          EINTR/EBUSY/EIO, Windows sharing
                                          violations; 0 disables)
           --conflict ask|skip|overwrite|rename|newer
           --sparse on|off · --no-clone · --net-streams N · --flatten
           --include <glob> · --exclude <glob> · --dry-run · --resume/--no-resume
ketra test <src>... [--hash sha256|md5|xxh64|crc32c]   read-test (throughput)
ketra manifest --root <dir> [--out <file>]
ketra verify  --root <dir> [--manifest <file>]
ketra bench   [--path <dir>] [--size 128M]

ketra clip put <path>... [--cut]    put files on the OS clipboard
ketra clip get [--json]             show clipboard files + copy/cut mode
ketra clip paste -d <dir> [options] transfer the clipboard via the engine

ketra shell install [targets]       wire file-manager context menus
ketra shell uninstall [targets]
ketra shell status

ketra peers   [--wait 5]                     browse LocalSend/Ketra peers
ketra send    <file>... --to <peer>          send over the LAN
ketra receive --dest <dir> [--port 53317]    receive (LocalSend compatible)
```

Exit codes: `0` ok · `1` file errors · `2` canceled · `3` usage · `4` verify failed.

## Clipboard & shell integration

`ketra shell install` (or the GUI's Settings tab) registers, per OS:

| OS | What you get |
|---|---|
| Windows | Explorer right-click: **Copy / Cut** (clipboard put), **Paste here** (background + folder), **Copy to Folder… / Move to Folder… / Test**, optional AutoHotkey v2 helper for Ctrl+Shift+C/X/V |
| Linux (Mint/Ubuntu/Fedora/KDE) | Nemo actions, Nautilus/Caja scripts (incl. *Paste Here*), Dolphin service menu, Thunar custom actions, `ketra-pick` destination picker, optional Cinnamon Super+V shortcut |
| macOS | Finder Quick Actions (Services): *Copy / Cut / Move with KetraCopy… / Paste with KetraCopy* — assign keyboard shortcuts in System Settings |

`ketra clip paste` reads whatever the *native* file manager put on the
clipboard — including its cut state — and runs the KetraCopy engine on it,
so native copy/cut keeps working while paste is accelerated and verified.

Full details, formats and manual installs: **`SHELL-INTEGRATION.md`**.

## LAN transfer (LocalSend compatible)

`ketra receive` announces via mDNS (`_localsend._tcp.local` +
`_ketra._tcp.local`) and accepts uploads on port 53317 from official
LocalSend clients; Ketra peers additionally get parallel ranged streams and
end-to-end SHA-256 manifests. `ketra send` discovers peers and sends.
Plain HTTP/1.1 on trusted LANs (as LocalSend does), hardened with digest
verification — see `docs/LAN-PROTOCOL.md` and `docs/ARCHITECTURE.md`.

## Android track

`android/` holds the Kotlin/Compose file manager that links the engine
through JNI (`ketra-core/src/jni.rs`, compiled only for `target_os =
"android"`). Prototype crates used during development live in `attic/`
(they are **not** workspace members). Build pipeline: `docs/ANDROID.md`.

## Project layout

```text
crates/ketra-core    engine (std-only): plan, pipeline, journal, resume,
                     throttle/AIMD, manifest, LAN, platform FFI layers
crates/ketra-cli     ketra binary (std-only): copy/move/test, manifest/verify,
                     bench, clip, shell, peers/send/receive
crates/ketra-md3     Material Design 3 design system for egui
crates/ketra-gui     ketracopy-gui binary (eframe)
integrations/        file-manager integration assets (Linux + Windows)
packaging/           deb, Inno Setup, Flatpak, macOS .app/DMG, portable bundlers
scripts/             build-all.sh, test_integration.sh (39 checks)
docs/                ARCHITECTURE, ALGORITHMS, CLEANROOM, LAN-PROTOCOL,
                     ANDROID, BUILD, FEATURE-PARITY
android/             Kotlin/Compose file manager (JNI over ketra-core)
```

## Verification status

* `cargo test --workspace` — 129 unit tests green (85 core, 20 CLI, 24 MD3)
* `scripts/test_integration.sh` — 39 end-to-end checks green on Linux x86-64
  (bit-rot detection, sparse preservation, resume after SIGINT, speed limit,
  conflict policies, filters, flatten, JSON progress, bench, clipboard
  graceful degradation, manifest round-trip)
* Windows x86-64/i686 binaries cross-built and smoke-linked; runtime
  behaviour shares the cfg-gated engine paths exercised on Linux
* macOS targets type-check; release binaries come from GitHub Actions

## License and clean-room note

MIT — see `LICENSE`. Clean-room methodology and the algorithm references
(token bucket, AIMD, SHA-256, XXH64, CRC32C) are documented in
`docs/CLEANROOM.md` and `docs/ALGORITHMS.md`. Development was assisted by
the Z.ai coding agent (GLM model) — see `ATTRIBUTIONS.md`.
