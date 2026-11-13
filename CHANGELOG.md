# Changelog

## 1.2.1 — 2026-11-13

### Adversarial-gate fixes (Gate 1 — harness/reviews/)

KetraCopy 1.2.0 passed the project's first adversarial consensus review
with one BLOCK verdict (LAN data loss) and four major findings; all are
fixed here, each with a regression test:

* **LAN receiver filename collision could silently destroy the first of two
colliding uploads** (CRITICAL): destinations are now *reserved* at prepare
time by atomically creating each file (`O_EXCL`), deduplicated within the
session and across concurrent sessions — a second file with the same name
(duplicate, or after sanitization: `a:b.txt` + `a_b.txt`) lands at
`name (2)`, never on top of the first. Abandoned zero-byte reservations are
cleaned up when a session expires.
* **A source that shrank mid-copy was zero-padded to its planned size and
journaled `done`** — silent corruption. The copy now fails loudly
(`Error::Shrank`) and keeps the partial + journal for `--resume`.
* **`--no-times` re-copies silently skipped same-size modified files** (the
fast scan degenerated to size-only). Without mtime preservation a size
match alone never shortcuts a copy; everything falls through to the
conflict policy.
* **A hostile `Content-Length` response header could abort the sender**
(unbounded pre-allocation). Client-side response bodies are capped at
8 MiB and rejected cleanly.
* **Resume-journal durability gap**: a batched `partial` record could become
durable while the data below the bound was not (power loss → resume trusts
the bound → silent zero-tail). The destination is now synced before every
journal record — any recoverable bound implies all bytes below it are on
the media — and `probe_resume` additionally verifies the chunk below a
trusted bound before trusting it.
* **`--conflict overwrite` (and the GUI's overwrite-all) failed with
`File exists` on existing destinations** — the reflink fast path opens
with `create_new` ("destination must not exist") and EEXISTed instead of
falling through to the streaming path. Found while fixing the `--no-times`
defect above, whose size-only skip used to mask it; the EEXIST now falls
through to the truncating streaming copy (regression-tested both at unit
level and end-to-end).
* Received-file names now prefix Windows reserved device names (`CON`,
`COM1`…) so LAN-received archives stay extractable on FAT/NTFS; the
generated `ketra-paste-smart` script rejects window titles containing `..`
or absolute paths; `manifest_check` removes never-confirmed reservations.

### Docs / CI honesty pass

* README, `docs/LAN-PROTOCOL.md`, `docs/ARCHITECTURE.md`, `docs/BUILD.md`
and `docs/ANDROID.md` no longer describe the attic HTTPS/rustls prototype
as shipped: the 1.2.x LAN transport is plain HTTP/1.1 (official LocalSend
apps interoperate after the planned TLS transport); the macOS GUI is
deferred (the pinned egui 0.28 stack does not compile against
rustc >= 1.100 on macOS); Android is source/CI-track only.
* `release.yml` builds the Windows (msvc, CLI + GUI) and macOS (CLI, both
darwin targets) release binaries natively and is runnable via
`workflow_dispatch`; every release asset now has exactly one producer
(sandbox: Linux + src; CI: Windows + macOS) so hash-baselined artifacts
cannot be silently overwritten. The sandbox zigbuild cross-compile was
attempted and abandoned (zig's linker rejects rustc's mingw link line —
evidence in `harness/evidence/2026-11-13-S7/`); CI-native builds replace
it.
* `build-apk.yml` is dispatch-only until the Android Gradle build is
re-pointed at the `ketra-core` JNI bridge (post-1.2.x roadmap).
* Packaging metadata: real GitHub URLs replace the `example.invalid`
placeholders (deb control + copyright, Inno Setup, metainfo); a tracked
`packaging/dist-README.txt` template makes `package-dist.sh` reproducible
from a fresh clone.

### Verification

* 133 unit tests (129 + 4 new regression tests: LAN reserve/collision,
source shrink, huge Content-Length, overwrite fall-through), 41
integration checks (39 + 2), interop suite 105/105 on Linux x86-64 (the
Windows/macOS cross-binary packages are produced natively by the CI
release workflow — single-producer rule — and are explicitly skipped in
the sandbox run). Evidence: `harness/evidence/2026-11-13-S7/`.

## 1.2.0 — 2026-11-05

### Performance / stability / memory optimization release

Measured on the fixed benchmark workload (Linux x86-64, 2 vCPU, 256 MiB
random file + 2 000-file tree, RAM-hot page cache); "before" = the 1.1.1
release binary re-measured in the same session, best of two runs per side
(the small-file scenarios swing up to 3× between runs on a shared sandbox;
the deltas below exceed that noise):

| Scenario | 1.1.1 | 1.2.0 | Δ |
|---|---|---|---|
| Hash 256 MiB with xxh64 | 858 MiB/s, **266 MB peak RSS** | 4 351 MiB/s, **9.5 MB RSS** | 5.1×, O(1) memory |
| Hash 256 MiB with sha256 | 246 MiB/s | 293 MiB/s | +19% |
| Copy 256 MiB, no verify | 2 383 MiB/s | 2 294 MiB/s | par (RAM-hot; fewer syscalls matter on real media) |
| Copy 256 MiB, inline verify | 1 354 MiB/s | 1 335 MiB/s | par on RAM-hot; **−50% source reads** |
| Copy + SHA-256 manifest | 224 MiB/s | 264 MiB/s | +18%, **one read per byte** |
| Copy 2 000 small files (off) | 1.70 s | 0.12 s | **14×** |
| Copy 2 000 small files (inline) | 1.82 s | 0.40 s | **4.6×** (run-to-run 0.16-0.40 s) |
| Bench write+read 256 MiB | 0.59 s, 267 MB RSS | 0.20 s, 9.5 MB RSS | 2.9×, O(1) memory |
| Verify manifest (256 MiB) | 1.04 s | 0.87 s | −16% |

### Added (engine, second optimization pass)

* **Inline SHA-256 manifest hashing: `--verify manifest` no longer
  re-reads the data.** When a manifest run starts a fresh file, the
  engine hashes every streamed chunk inside the copy loop — including
  hole regions (hashed as zeros, CPU-only) and zero-run chunks — and
  hands the digest to the manifest phase, which reuses it instead of
  opening and reading the file a second time. Each byte of a
  `--verify manifest` run is now read from the source exactly once
  (previously: one read during the copy + one full read during the
  manifest phase). On storage-bound destinations (HDD, USB, SMB) this
  removes the entire second read pass; on cache-hot runs the win is the
  eliminated open/seek/latency of the second pass. Inline hashing opts
  the file out of the kernel-side `copy_file_range` fast path (bytes
  must pass through userspace to be hashed); reflink-cloned, resumed
  and multi-stream files still hash via a manifest-phase read, and
  their digests are computed the old way. Digest equality with a full
  read of the destination is verified for sparse (multi-hole), dense,
  empty and resumed files.
* **Batched journal records (32 MiB stride).** `partial` journal
  entries were appended per 1 MiB chunk — a 1 TiB transfer wrote ~1 M
  records (~60 MB of journal I/O) on top of the data itself. Records
  are now coalesced to a 32 MiB stride, with two anchors that preserve
  resume semantics: the **first completed chunk of every file is
  journaled immediately** (a hard kill / power loss before the first
  stride still leaves a resumable record instead of falling into the
  conflict policy), and a cancel still records the exact position.
  Large transfers append ~32× fewer records (a 1 TiB run: ~32 k
  instead of ~1 M); small files keep their two records (first-chunk
  partial + done). The resume probe re-verifies content from the
  recorded bound, so a coarser stride only ever re-copies a bounded
  tail, never corrupts.
* **`journal::latest_offsets` streams the journal** line by line with a
  bounded reader instead of materializing every record: resuming over
  an interrupted multi-TiB run previously held the whole journal (one
  `Vec` entry per chunk ever written) in memory at once; now only the
  final map (one entry per file) is kept.
* **Per-chunk readahead**: the pipeline issues `posix_fadvise(WILLNEED)`
  (Linux) / `F_RDADVISE` (macOS) for the next chunk before writing the
  current one, so the kernel prefetches ahead of the write cursor on
  rotational/network storage. No-op on Windows and under direct I/O.
* **`--net-streams` retries transient I/O.** The ranged multi-stream
  path previously aborted a whole file on the first `EINTR`/`EBUSY`/
  `EIO` (or Windows sharing violation) mid-range; it now reuses the
  engine's retry policy (same backoff, `--retries`) in place at the
  failing offset — a share hiccup no longer costs a multi-hour
  transfer its progress.

### Changed (engine)

* **XXH64 and CRC32C are now true streaming states** (O(1) memory).
  Previously the stateful wrappers accumulated the entire input in a
  `Vec` before hashing — hashing a 10 GiB file with `--hash xxh64`
  (or the `bench` read stage, or LAN SHA-256 probing at scale) held the
  whole file in RAM and could OOM. The streaming XXH64 mirrors the
  one-shot stripe/tail algorithm exactly; CRC32C now keeps the reflected
  register per `update` call. Validated against the xxHash spec vectors,
  RFC 3720 B.4 vectors, and exhaustive streaming/one-shot equivalence
  tests (every length 0–200 split at every position).
* **CRC32C uses slicing-by-8** (H. Anvin): eight bytes folded per step
  through eight lookup tables instead of one table walk per byte.
* **SHA-256 schedules message words through a rolling 16-word window**
  instead of a 64-word array, keeping the compression loop cache-resident.
* **Inline verification no longer re-reads the source.** The destination
  read-back is compared against the source bytes still held in the
  streaming buffer; only `copy_file_range`-transferred chunks (which
  bypass the buffer kernel-side) fetch a fresh source view. One full
  source read less per chunk, identical corruption coverage.
* **File data is finalized with `fdatasync` on Linux** (`sync_all`
  elsewhere) — the copy changes no inode metadata that `fdatasync`
  would skip (mtime is set after the sync). On the benchmark FS this
  alone cut the many-small-files workload ~12×.
* **Scratch buffers are no longer zero-filled** and the inline-verify
  read-back buffer is allocated only when verification is on (up to
  2×chunk of pointless memset per file removed; slices are always
  bounded by actually-transferred byte counts).
* **`is_all_zero` folds 8 bytes per step** (`chunks_exact(8)` +
  `u64::from_le_bytes`, auto-vectorized) — the per-byte iterator was
  measurable on the zero-run sparse path used by Windows/macOS.
* **Kernel readahead hint** (`posix_fadvise SEQUENTIAL` on Linux,
  `F_RDADVISE`-equivalent on macOS; no-op elsewhere) applied to the
  source file at open.
* **Multi-stream (`--net-streams`) per-stream buffers are capped** so
  16 streams × 16 MiB chunks can no longer hold up to 256 MiB of scratch
  (now bounded to ~32 MiB total, ≥64 KiB per stream).
* **Parallel file-level copying: `--parallel N` (1..=16, default 1).**
  N workers pull fully-prepared tasks (conflict/resume decisions stay on
  the main thread with the UI callbacks), so one file's fsync overlaps
  the next file's transfer — the win shows on sync-heavy destinations
  (HDD, USB, network shares). Workers share one mutex-guarded journal
  writer; a `JournalHandle` was added for this. Manifest order is
  normalized to plan order. Cancel keeps partials + journal for
  `--resume` exactly like the sequential path (verified end-to-end).

### Added (stability)

* **Transient-I/O retry policy: `--retries N` (default 2).** Interrupted
  syscalls, `EIO`/`EBUSY` and — on Windows — sharing/lock violations
  (the classic "file in use" case) are retried with a short linear
  backoff (25 ms per attempt, capped 250 ms) at open/read/write;
  `--retries 0` restores fail-fast.
* **`TCP_NODELAY` on every LAN connection** (client and accepted server
  streams): request/response exchanges no longer risk Nagle-induced
  multi-100 ms stalls behind large upload bodies.

### Docs

* README: 1.2.0 release summary, CLI cheat sheet documents `--parallel`
  and `--retries`; verification status updated (129 unit tests).

## 1.1.1 — 2026-11-05

### Fixed (interop test campaign)

* **mDNS discovery was silently dead on little-endian hosts** (x86-64,
  ARM64): `SockAddrIn::new` stored `sin_addr` with `u32::from_be_bytes`,
  which byte-reverses the multicast address on little-endian memory
  layouts — every announce/query went to `251.0.0.224` instead of
  `224.0.0.251` while `sendto` still reported success. Fixed with
  `from_ne_bytes` (raw network-order octets); the mDNS responder also
  now includes the **A record** in PTR replies so browsers can complete
  instances. `KETRA_MDNS_LIVE=1 cargo test` exercises discovery live.
* **Linux clipboard was broken at runtime**: the X11 backend resolved
  the dlsym symbol `ConnectionNumber`, which is a *macro* in Xlib, not
  an exported function — `clip put/get/paste` failed with "missing
  symbol" on every machine. Now resolves the real `XConnectionNumber`.
* **`ketra clip get --json` double-quoted paths** (`""path""`): the
  output formatter wrapped the already-quoted JSON string again.
* **`ketra __clip-server` rejected its state file**: `clip put` spawns
  the daemon with a positional path while the parser only accepted
  `--file <path>` — every `clip put` died with "unknown flag". Both
  forms are accepted now.
* **Settings file was ignored by the CLI for `verify` and
  `use_clone`**: `build_options` unconditionally overrode them with
  parser defaults, so `"verify": "manifest"` / `"use_clone": false` in
  the shared `ketracopy.json` (used by both CLI and GUI) never applied,
  and `--limit 0` could not clear a settings-configured speed limit.
  Precedence is now: explicit CLI flag > settings file > CLI default.
* **Nautilus/Caja copy & move scripts were dead**: the generated
  scripts ran `ketra copy <files>` *without* `-d` (usage error).
  They now route through the `ketra-pick` helper; the helper accepts
  `KETRA_DEST=<dir>` to pre-select a destination (scripting/CI).
* **`ketra peers` listed the same device twice** (once per announced
  service); deduped by fingerprint.
* **Cross-filesystem copies failed** (`Invalid cross-device link`): the
  Linux reflink fast path did not treat `EXDEV` as "not supported", so
  every copy to a USB stick / another partition failed with default
  settings. Now falls back to a regular verified copy (macOS mapping
  mirrored; `ENOSYS` too).
* **`GET /api/localsend/v2/info` returned "not found"** — only `POST
  /register` answered identity probes. The documented info endpoint is
  registered on both `/api/localsend/v2` and `/api/ketra/v1`.
* **`ketra clip paste -d <new-dir>` refused to create the destination**
  while `ketra copy -d` auto-creates it; paste now mirrors copy.
* **Nautilus/Caja "Paste Here" URI decoding was broken**: the generated
  sed pipeline had unterminated `s` commands for the `%20`/`%23`
  escapes; replaced with pure-bash parameter expansion.
* **`ketra shell uninstall pick` kept `~/.local/bin/ketra-pick`**;
  the helper is now removed (the `ketra` CLI binary itself stays).
* **dist/SHA256SUMS.txt listed itself** with a stale checksum; the
  packager now excludes it from its own manifest.

### Tests

* New `scripts/test_interop.sh` — 106 checks covering feature
  *combinations*: settings→engine, throttle×resume×verify×manifest,
  conflicts×manifest refresh, sparse×manifest, filters×flatten,
  dry-run↔real-run parity, real X11 clipboard round-trips under Xvfb,
  shell-script→CLI→clipboard→engine chains, mDNS↔send↔receive↔verify
  LAN loopback, GUI smoke, artifact cross-consistency and bench.
* `KETRA_MDNS_LIVE=1 cargo test` runs a live announce/browse test
  (gated so CI without multicast still passes).

### Notes

* The LAN receiver speaks plain HTTP/1.1 on port 53317 (see
  `docs/LAN-PROTOCOL.md`). Ketra-to-Ketra transfers are verified
  end-to-end with SHA-256; HTTPS with self-signed certificates (strict
  LocalSend v2 compatibility) remains on the roadmap — the rustls
  design lives in `attic/ketra-net`.


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
