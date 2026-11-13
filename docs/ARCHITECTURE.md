# Architecture

KetraCopy is a four-crate workspace. The two shipped crates are dependency-free: hashing, JSON, globbing, throttling and all OS calls are implemented in-tree. `ketra-md3` (Material Design 3 design system for egui) and `ketra-gui` (eframe/egui 0.28 shell, OpenGL `glow` renderer, `rfd` file dialogs) depend on crates.io; both compile (`cargo check --workspace`, GUI release build) — this document covers the engine and CLI, which are the runtime-verified surface. (`Cargo.lock` lists the GUI's egui/eframe/rfd dependency tree; `ketra-core` itself has none.)

## Crate map

`ketra-core` (`crates/ketra-core/src/`):

| Module | Role |
|---|---|
| `lib.rs` | Crate map, re-exports (`run`, `Summary`, `Controller`, `TransferOptions`, ...), `VERSION` |
| `options.rs` | `TransferOptions` and its enums (`Op`, `VerifyLevel`, `ConflictPolicy`, `HashAlgo`, `SparseMode`); `Default` values |
| `plan.rs` | `build_plan`: source walk -> `TransferPlan` of `Task`s (`MakeDir`/`Link`/`File`), include/exclude filters, dest-inside-source guard, `\\?\` path hygiene on Windows |
| `engine.rs` | `run`: worker pool, resume bookkeeping, retries wiring, Phase 3 manifest assembly, `Summary` |
| `pipeline.rs` | `run_file_task`: conflict resolution, fast paths, Phase 2 stream copy + inline verification, metadata preservation |
| `netmode.rs` | Experimental parallel ranged copy for network mounts |
| `journal.rs` | Resume journal, `<dest>/.ketra/journal.jsonl` |
| `resume.rs` | Prefix probe: first divergent 1 MiB block between source and partial destination |
| `manifest.rs` | `.ketra/manifest.jsonl` write/read/verify (`ketra verify`) and `build_for_tree` |
| `aimd.rs` | AIMD chunk-size controller |
| `throttle.rs` | Shared token-bucket rate limiter |
| `device.rs` | Destination profiling (device class -> worker count, buffer bounds, unbuffered threshold, net streams) |
| `controller.rs` | Pause/resume/cancel/skip flags + live speed limit (atomics + condvar) |
| `progress.rs` | `Event` enum over `std::sync::mpsc`, aggregate `Stats` with EMA throughput |
| `error.rs` | `KetraError`/`ErrorKind` taxonomy and OS-code classification |
| `hash.rs` | SHA-256, MD5, XXH64, CRC32C implemented from their public specs |
| `glob.rs` | `*`, `**`, `?` matcher (case-insensitive; bare patterns also match basenames) |
| `json.rs` | Minimal JSON writer/parser |
| `settings.rs` | Portable `ketra.json` defaults (exe-adjacent, `portable.flag`, or per-user config dir) |
| `platform/` | Per-OS capability layer (`mod.rs` dispatchers; `unix.rs`, `linux.rs`, `macos.rs`, `windows.rs`, `aligned.rs`) |

`ketra-cli` (`crates/ketra-cli/src/`): `main.rs` (dispatch/exit codes), `args.rs` (hand-rolled parser + `usage()`), `run_cmd.rs` (copy/move/test), `manifest_cmd.rs` (manifest/verify), `bench.rs`, `view.rs` (progress rendering, JSONL), `sig.rs` (SIGINT / `SetConsoleCtrlHandler`).

## Engine data flow

```text
 sources, dest, TransferOptions
        |
        v
 plan::build_plan  --> TransferPlan { tasks: Vec<Task>, total_bytes, dest_dir }
        |              (dirs first, then links, then files; duplicate dsts pruned)
        v
 engine::run(plan, opts, ctrl, ev)                     (engine.rs)
   |-- device::profile(dest)        -> DeviceProfile (class, workers, buffers,
   |                                    unbuffered threshold, net_streams, sparse_ok)
   |-- check_free_space             -> Event::Warning when tight
   |-- Journal::open + journal::read -> done_set / partial maps (resume input)
   |-- std::thread::scope: N workers pop task indices from a shared VecDeque
         |-- skip tasks whose dst is in done_set
         |-- probe partial dsts -> resume::find_resume_offset (bounded by journal)
         |-- dispatch_task:
         |     MakeDir -> create_dir_all
         |     Link    -> symlink replicate (Windows: copy target fallback)
         |     File    -> TaskCtx { throttle: shared bucket, skip flag,
         |                 start_offset, effective_verify }
         |                 -> pipeline::run_file_task
         |                       |-- Events (mpsc) -> CLI view / JSONL
         |                       '-- FileOutcome { bytes, sha256, xxh64, ... }
         '-- results -> OutcomeSlot list / ErrorRecord list
   |-- Summary { files_ok/failed/skipped, bytes, ms, bps, canceled, errors }
   |-- Phase 3: manifest::write(<dest>/.ketra/manifest.jsonl)   [copy|move, not canceled]
   '-- journal::clear on clean success
```

`Summary`, `ErrorRecord` and the event stream are the only outputs; the CLI maps them to exit codes (`run_cmd.rs::run`).

## The 3-phase verification pipeline

Phase 1 — quick scan. `plan.rs` records every source's size and mtime at plan time. In `pipeline.rs::resolve_conflict`, a destination with identical size and mtime within 2 s of the source is treated as already present (`TaskSkipped`). Otherwise the configured `ConflictPolicy` applies (`Overwrite`, `OverwriteIfNewer` — default: overwrite when the destination is older or smaller, since a newer-but-smaller destination is an interrupted copy — `Skip`, `Rename` via `unique_sibling` producing `name (1).ext`, or `Ask` through the `on_conflict` callback; the CLI binds this to an interactive prompt when `--conflict ask` and stdin is a TTY). A resume with `start_offset > 0` always continues into the partial file.

Phase 2 — stream copy with inline verification. `pipeline.rs::stream_copy` copies positioned chunks (`pread`/`pwrite`, i.e. `read_at`/`write_at`/`seek_read`/`seek_write`); when `VerifyLevel::Inline` (the default) each written chunk is immediately read back from the destination and byte-compared, so corruption is caught before the file completes. A mismatch restarts the file from scratch (digests reset) up to `min(retries, 2)` inline retries, then fails with `VerifyMismatch`. `VerifyLevel::FullHash` compares digests instead (`finish_full_hash` reuses the source digest streamed during the copy when the algorithm is SHA-256). `VerifyLevel::QuickScan` restricts checking to the Phase 1 size+mtime comparison; `Off` checks write success only.

Phase 3 — SHA-256 manifest. `engine.rs::build_manifest_entries` collects `{rel_path, size, mtime_ms, sha256, xxh64}` from stream-copy outcomes (files copied through fast paths that skip the stream do not compute digests and contribute no entries — with `--verify quick` plus same-volume fast paths the manifest can therefore cover a subset), and `manifest.rs::write` appends them to `<dest>/.ketra/manifest.jsonl`. `ketra verify` re-hashes and reports `missing`/`size`/`sha256` mismatches; exit code 4. Digests see the file sequentially even when sparse holes were skipped: `feed_zeros` folds hole regions into the digests as zeros (see "Sparse handling").

## Fast-path decision ladder

`pipeline.rs::copy_inner` tries, in order:

```text
 1. Move + verify Off/QuickScan + same volume      -> rename (fall through to
                                                      copy+delete if rename fails)
 2. Copy + verify Off/QuickScan + resumed_from==0
    + same volume                                  -> reflink: FICLONE ioctl
                                                      (btrfs/XFS and other
                                                      reflink-capable filesystems)
                                                      or macOS clonefile (APFS)
 3. Copy + verify Off/QuickScan + resumed_from==0
    + Linux                                        -> copy_file_range, chunked,
                                                      AIMD-sized, throttled
 4. Copy + net_streams>1 + size>=64 MiB + verify
    Off/QuickScan + start_offset==0                -> netmode parallel ranges
                                                      (experimental)
 5. otherwise                                      -> stream_copy: userspace loop
                                                      with AIMD chunks, sparse
                                                      extents, inline compare
```

Paths 1-4 emit `FileOutcome.cloned` / `kernel_copied` so `ketra bench` can report which path engaged. Cancellation never falls through from a fast path into a fresh truncating stream pass; netmode failures fall back to `stream_copy` (the partial destination is removed first). `platform::kernel_copy_chunk` (`platform/linux.rs`) returns `Ok(None)` for `EPERM/EBADF/EXDEV/EINVAL/ENOSYS/EOPNOTSUPP`, making the userspace fallback permanent for that filesystem pair, and retries `EINTR` in place.

## Resume design

Interrupted runs leave two artifacts: the journal (`journal.rs`, JSONL lines `session`/`done`/`partial` under `<dest>/.ketra/journal.jsonl`) and partial destination files. On the next run with the same destination, `engine.rs` skips `done` files, and for `partial` files (or any non-empty destination smaller than its source — hard-crash fallback) calls `resume.rs::find_resume_offset`.

`find_resume_offset` exploits the prefix invariant of interrupted copies: the destination matches the source for its first N bytes and diverges after. "Block at offset m is identical" is then monotone, so the probe binary-searches block-aligned 1 MiB positions (after an anchor check of the first block) and finishes with a short linear scan for the exact boundary — O(log n) chunk reads instead of a full re-read. Probes byte-compare both files directly (stronger than checksum probing at these scales). The engine bounds the probe by the journal's recorded written length (`probe_limit`) and truncates the result to 4096 alignment (`off & !4095`).

Preallocation caveat: `pipeline.rs::open_write` reserves real blocks via `platform::preallocate` for files > 4 MiB. A preallocated (zero-filled) tail would look like a valid zero-prefix to the probe, so the journal-bounded limit exists precisely to stop prealloc zeros from being mistaken for transferred content when the source itself contains zero holes. When resuming (`resumed_from > 0`) the sequential digest position is disabled (`dpos = u64::MAX`) because the verified prefix was not read by this run; resumed files therefore carry no manifest digest. The journal is deleted only after a clean, uncanceled run (`engine.rs`).

## Threading model

- `engine.rs` spawns `workers` threads inside one `std::thread::scope` (worker count: `opts.parallel_files` or `DeviceProfile.workers` — 1 for HDD/removable, 2 for SSD/unknown, 3 for network).
- One shared token bucket per file task: `dispatch_task` creates `throttle::new_shared(ctrl.speed_limit())`; every chunk from any stream of that task draws from it. `pipeline.rs::gate` re-syncs the bucket's rate from `Controller::speed_limit()` each chunk, so live rate changes apply.
- Per-task skip flags: an `Arc<AtomicBool>` registered in `Controller::skips`; `skip_current()`/`skip_task(idx)` set them, the chunk loop observes them.
- `Controller` state is an `AtomicU8` (running/paused/canceled) plus an `AtomicU64` speed limit and a condvar gate (`wait_if_paused` with 200 ms timed waits so cancellation is noticed while paused).
- `Stats` counters are atomics; the only mutexes on the hot path are the task queue, the token bucket and the EMA sampler.

## Event / progress model

The engine emits `Event`s (`progress.rs`: `Started`, `TaskStart`, `TaskPhase` (scan/copy/verify/manifest), `TaskProgress`, `TaskDone`, `TaskSkipped`, `TaskError`, `ConflictResolved`, `Warning`, `Paused`/`Resumed`, `Finished`) through one `std::sync::mpsc` channel. The CLI renders a single-line bar (`view.rs`, with ETA over a ~2 s sample window) or machine-readable JSONL (`--progress json`), and quiet mode suppresses everything but the summary. `Stats::add_bytes` maintains an EMA of instantaneous throughput (`ema = 0.8*ema + 0.2*inst`, sampled at >= 100 ms granularity) exposed as `current_bps()`.

## Platform layer capabilities

`platform/mod.rs` dispatches to per-OS implementations; every capability degrades gracefully to the portable pipeline when unsupported.

| Capability | Linux | macOS | Windows |
|---|---|---|---|
| Unbuffered open | `O_DIRECT` (`unix.rs`, write side) | none in this layer | `FILE_FLAG_NO_BUFFERING` + `FILE_FLAG_SEQUENTIAL_SCAN` (`CreateFileW`) |
| Sequential readahead | `posix_fadvise(POSIX_FADV_SEQUENTIAL)` | `F_RDADVISE` implemented in `macos.rs` (dispatcher currently routes to the POSIX no-op) | `SEQUENTIAL_SCAN` at open |
| Buffer-cache bypass | `O_DIRECT` | `F_NOCACHE` implemented in `macos.rs` (not currently dispatched) | `FILE_FLAG_NO_BUFFERING` |
| Hole maps | `lseek(SEEK_DATA/SEEK_HOLE)` with 1,000,000-iteration guard | none (dense) | `FSCTL_QUERY_ALLOCATED_RANGES` (1 MiB extent buffer) |
| Reflink | `FICLONE` ioctl | `clonefile(2)` | not exposed (ReFS block cloning stub returns false) |
| Kernel copy | `copy_file_range(2)`, EINTR-safe, errno fallback ladder | not used | not used |
| Preallocation | `fallocate(2)` (mode 0) | `F_PREALLOCATE` implemented in `macos.rs`; dispatcher routes to `unix::preallocate` | none; destination created with `CREATE_ALWAYS`, final length via `set_len` |
| Durability | `fsync(file)`, `fsync(dir)` | same as unix | `FlushFileBuffers` (file; dir handle opened with `FILE_FLAG_BACKUP_SEMANTICS`) |
| Volume identity | `st_dev` | `st_dev` | `GetFileInformationByHandle` volume serial + drive-letter/UNC fallback |
| FS name / network detect | `/proc/self/mounts` longest-prefix, network FS type list | `statfs64` + `MNT_LOCAL` | `GetVolumeInformationW`, `GetDriveTypeW == DRIVE_REMOTE` |
| Long paths | n/a | n/a | `\\?\` / `\\?\UNC\` via `to_extended_path`, applied in `plan::absolutize` |
| Locked files | blocking opens | blocking opens | sharing-violation wait loop (`wait_lock`, 300 ms poll) + `clear_readonly` before delete |
| Free space | `statvfs` (64-bit ABIs; stub on 32-bit) | `statvfs` | `GetDiskFreeSpaceExW` |

Unbuffered I/O uses 4096-aligned heap buffers (`platform/aligned.rs::AlignedBuf`); the engine snaps `buffer_min`/`buffer_max` to 4096 multiples and pads short direct-I/O tails on write, trimming them with `set_len`.

## Error taxonomy and retry classification

`error.rs` defines `ErrorKind` (`AccessDenied`, `NotFound`, `AlreadyExists`, `SharingViolation`, `NoSpace`, `VerifyMismatch`, `Canceled`, `Skipped`, `InvalidPath`, `Unsupported`, `IoOther`) and maps raw OS codes per-platform (e.g. Win32 `ERROR_SHARING_VIOLATION`/`ERROR_LOCK_VIOLATION` -> `SharingViolation`; POSIX `ENOSPC`/`EDQUOT` -> `NoSpace`). Defaults: `SharingViolation` and `IoOther` are retriable; `VerifyMismatch` is additionally retried by `run_file_task`'s loop. File-level retries use exponential backoff (250 ms doubling, capped at x32) for up to `opts.retries` attempts; locked-file waits poll every 300 ms until `wait_lock` (default 60 s) expires. `continue_on_error` (default true) keeps the batch alive; `--abort` cancels the whole run on the first failure. `Canceled` aborts fast paths without truncating partial output.

## Sparse handling

With `SparseMode::Auto` (default) and a sparse-capable destination profile, `platform::hole_ranges` returns data extents; the copy loop touches only those ranges. In inline mode the complement (`hole_spans`) is verified to read back as zeros in the destination, and the file length is forced to the source size with `set_len`. Digests require sequential input, so `feed_zeros` folds skipped hole spans into SHA-256/XXH64 as zeros, keeping manifests hash-correct for whole files. `direct` I/O is disabled for holed files (unaligned tails could bleed into holes), and the network profile has `sparse_ok = false`. Verified on Linux x86-64: the integration suite checks destination block counts against the source (`stat -c %b`).

## Netmode design (experimental)

`netmode.rs` splits a file into contiguous, 4096-aligned ranges, one thread per range, each with its own `Aimd` and `AlignedBuf`, all drawing from the task's shared token bucket (aggregate rate = the configured limit). Eligibility (`is_eligible`): more than one stream (auto only when the device profile says network; `--net-streams n` forces, `1` disables), file >= `MIN_NET_FILE_SIZE` (64 MiB), `VerifyLevel::Off`/`QuickScan` (inline byte-compare cannot work across independent range workers), plain copy, not a resumed file. A progress thread aggregates per-worker counters every 120 ms. Any worker error cancels the rest; the caller then falls back to `stream_copy`. Rationale and citations: `docs/ALGORITHMS.md`.

## Android + LAN extension (1.0.0)

The 1.0.0 release extends KetraCopy from a desktop-only TeraCopy
reimplementation into a cross-platform file transfer tool: the same Rust
engine now also runs on Android, and two devices (Android-Android,
Android-desktop, desktop-desktop) can transfer files over the LAN using the
open LocalSend v2 protocol plus a Ketra accelerated extension. The
extension keeps `ketra-core` std-only by isolating all network code in a
new crate and exposing the result to Kotlin via a thin UniFFI facade.

### New crates

| Crate | Path | Role |
|---|---|---|
| `ketra-net` | `crates/ketra-net/src/` | LocalSend v2 protocol + Ketra accelerated extension. mDNS discovery (`discovery.rs`), rustls server/client (`tls.rs`), minimal HTTP/1.1 server (`http.rs`), LocalSend v2 message schemas (`protocol.rs`), receive server (`server.rs`), send client (`client.rs`), high-level `TransferJob` (`transfer.rs`), device identity + self-signed Ed25519 cert (`identity.rs`). Pulls in `rustls`, `mdns-sd`, `rcgen`, `rustls-pemfile` — all pure-Rust, all cross-compile cleanly to the Android NDK targets. |
| `ketra-ffi` | `crates/ketra-ffi/src/` | UniFFI 0.28 proc-macro facade (`#[derive(uniffi::Object)]`, `#[uniffi::export]`). Exposes one `KetraEngine` object plus `EngineOptions`, `PeerInfo`, `ProgressEvent`, `TransferResult` records, and a `ProgressCallback` interface Kotlin implements. The compiled `cdylib` is `libketraffi.so`; the matching `Ketra.kt` is generated by `uniffi-bindgen` from the metadata embedded in the `.so`. |
| `ketra-core::hash::sha256` | `crates/ketra-core/src/hash.rs` | Not a new crate, but the `pub fn sha256(&[u8]) -> [u8; 32]` helper exposed at the crate root is the trust primitive reused by `ketra-net::identity` (fingerprint) and `ketra-net::tls::PinningVerifier` (cert SPKI hashing). Implemented from FIPS 180-4 in-tree; no `ring`/`sha2` dependency for the hash itself. |

`ketra-core` itself stays std-only and unchanged in role: the same engine,
pipeline, manifest, resume, throttle, and platform layer power both the
desktop CLI/GUI and (transitively, through `ketra-ffi`) the Android app.

### Desktop CLI subcommands

`ketra-cli` gains three new subcommands in `crates/ketra-cli/src/lan.rs`:

| Subcommand | One-line semantics |
|---|---|
| `ketra peers [--wait N]` | One-shot mDNS browse for `_localsend._tcp.local.` peers; prints alias, IP, port, fingerprint, and whether the peer advertises the Ketra extension. Exits after N seconds (default 5). |
| `ketra send <peer-alias|ip> <files...>` | Resolves the peer, fetches its `/info`, prints the fingerprint for human confirmation, then runs the full send flow: register -> prepare-upload (with Ketra block) -> parallel chunked upload with per-chunk checksums -> manifest exchange. Blocks until the transfer completes or fails. |
| `ketra receive [--dest DIR]` | Starts the mDNS announce + HTTPS receive server on port 53317. Foreground-blocking: prints each incoming session's sender alias + fingerprint, accepts all files (after the human confirms the sender's fingerprint in the log), writes them to `DIR` (default cwd), writes the manifest sidecar if requested. Ctrl-C cancels cleanly. |

The three commands share a common `DeviceIdentity` loaded or generated at
`~/.config/ketra/ketra-identity.{crt,key}.pem` and advertise the same
`KetraExtension { accelerated: true, checksum: "sha256", parallel_streams:
4, resume_supported: true, manifest_exchange: true }`.

### Android app package layout

The Android app lives under `android/app/src/main/kotlin/io/ketracopy/`
and follows a "Rust core, Kotlin thin" split: every byte-touching
operation is dispatched through the UniFFI-generated
`io.ketracopy.uniffi.ketra.KetraEngine` facade; Kotlin only orchestrates
UI state, SAF URIs, foreground services, and Android permissions. See
[`docs/ANDROID.md`](ANDROID.md) for the full package map (ui/theme,
ui/screens, ui/viewmodel, data, storage, net, provider) and the four UI
scopes (local storage, LAN peers, cloud/SMB, compression/share).

### GitHub Actions CI

[`/.github/workflows/build-apk.yml`](../.github/workflows/build-apk.yml)
runs two parallel jobs on `ubuntu-24.04` on every push to `main`/`master`
(and on PRs and manual dispatch): `build-apk` (JDK 17 + Rust stable with
the four Android targets + cargo-ndk 3.0.0 + uniffi-bindgen-cli 0.28.3 +
Android SDK platform 35 / build-tools 35 / NDK 27, then `./gradlew
assembleDebug`, then uploads the APK as the `ketracopy-debug-apk` artifact)
and `cargo-test` (`cargo test --workspace --locked` plus
`bash scripts/test_integration.sh`). A `concurrency` block cancels
in-flight runs on the same ref.

### Runtime-verification status (honest)

- **Rust workspace tests**: the 70 unit and integration tests across
  `ketra-core`, `ketra-net`, `ketra-ffi`, `ketra-md3`, `ketra-cli`, and
  `ketra-gui` pass on Linux x86-64 (`cargo test --workspace`).
- **HTTPS `/info` endpoint**: verified via `curl -k https://127.0.0.1:53317/api/localsend/v2/info`
  against a loopback receiver started by `ketra receive --dest /tmp/x`; the
  JSON body and the TLS handshake (Ed25519 self-signed cert, SHA-256
  fingerprint) match the schema documented in `docs/LAN-PROTOCOL.md`.
- **Full send -> receive transfer with mDNS discovery**: **not verified
  in-sandbox.** mDNS multicast is not routed in the headless build
  sandbox, so a real two-device send requires a physical LAN. The send and
  receive code paths are exercised by the unit tests against the loopback
  HTTPS transport (bypassing mDNS by passing the receiver's `127.0.0.1`
  address directly to the send client), but the end-to-end mDNS ->
  `/info` -> `/register` -> `/prepare-upload` -> parallel `/upload` ->
  manifest exchange -> receiver-side `.ketra/manifest.jsonl` flow on a
  real LAN is the canonical remaining verification step.
- **Android APK**: **not built in-sandbox** (no Android SDK, NDK, or
  JDK 17 available in the headless environment). The Gradle build
  graph (`buildRustLibs` -> `generateUniFFIBindings` -> `assembleDebug`)
  is wired and the `buildRustLibs` task has been verified to invoke
  `cargo ndk` correctly; the GitHub Actions workflow is the canonical
  build and is expected to produce a working `app-debug.apk` once it runs
  on a GitHub-hosted Ubuntu runner with the SDK installed.
