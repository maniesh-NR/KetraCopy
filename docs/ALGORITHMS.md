# Algorithms and references

Every technique below is actually implemented in this repository; each entry states what the code does, where it lives, and the public source the implementation follows. Behavior marked "verified on Linux x86-64" was exercised by `cargo test -p ketra-core --lib` or `scripts/test_integration.sh`. Throughput figures printed by `ketra bench` are environment-specific and are intentionally not quoted here as product claims.

## AIMD chunk sizing

**What we do.** The I/O chunk size starts at `buffer_min` (default 256 KiB) and adapts per file and per stream: a chunk that completes in under 30 ms (`FAST_CHUNK_MS`) counts as "fast" and grows the next chunk additively by 12.5% (with a 4 KiB floor); a slow chunk halves the chunk size multiplicatively, never dropping below `buffer_min` (default 8 MiB ceiling via `buffer_max`, snapped to 4096 multiples by the engine). This keeps large sequential reads/writes on spinning or network storage while shrinking quickly around stalls.

**Where.** `crates/ketra-core/src/aimd.rs` (`Aimd::feedback`); consumed by `pipeline.rs::stream_copy`, `pipeline.rs::kernel_copy` and `netmode.rs::range_worker`. Unit tests verify the bounds (`grows_additively_and_caps`, `shrinks_multiplicatively_and_floors`, `never_exceeds_bounds`).

**Citation.** D.-M. Chiu and R. Jain, "Analysis of the Increase and Decrease Algorithms for Congestion Avoidance in Computer Networks", *Computer Networks and ISDN Systems* 17(1), 1989. KetraCopy applies the classic additive-increase/multiplicative-decrease law to the I/O chunk size instead of a TCP congestion window.

## Token-bucket speed limiting

**What we do.** A bucket fills at `rate` bytes/sec up to a capacity and each I/O chunk drains it; `acquire` sleeps in slices of at most 50 ms so rate changes and cancellation stay responsive. Capacity is `rate/4` (at least 1 MiB) as burst allowance, and grows to cover any chunk larger than the bucket. One bucket is shared per task across all workers and netmode streams, so `--limit` throttles the whole batch, not each file. The rate is re-read from the `Controller` at every chunk gate, so it can be changed while a run is in progress.

**Where.** `crates/ketra-core/src/throttle.rs`; shared bucket created in `engine.rs::dispatch_task`; live re-sync in `pipeline.rs::gate`; used by `netmode.rs` (per-stream draws from the same bucket).

**Citation.** J. Turner, "New directions in communications (or which way to the information expressway?)", 1986 — the token bucket. Verified on Linux x86-64: integration test 6 holds a 2 MiB/s limit for a real copy; unit tests cover unlimited/limited/rate-change behavior.

## Kernel-side copy

**What we do.** On Linux, when the requested verify level allows it (Off/QuickScan), whole files are copied with `copy_file_range(2)` in AIMD-sized, throttled chunks — no user-space data pass. The FFI wrapper returns `Ok(None)` — triggering a permanent fallback to the userspace pipeline — for `EPERM`, `EBADF`, `EXDEV`, `EINVAL`, `ENOSYS` and `EOPNOTSUPP` (unsupported kernel/filesystem pairs, cross-device), retries `EINTR` in place (a signal such as Ctrl+C interrupts the syscall without consuming the request), and propagates other errors. A `0` return mid-file is treated as "source shrank during copy" and fails the file rather than reporting success.

**Where.** `crates/ketra-core/src/platform/linux.rs::kernel_copy_chunk`; decision ladder in `pipeline.rs::copy_inner`/`kernel_copy`.

**Citation.** Linux man page `copy_file_range(2)`. Verified on Linux x86-64: `ketra bench`'s fast-path probe reports which path engaged and reports the kernel-side path on this sandbox's filesystem.

## Reflink / clone fast paths

**What we do.** Same-volume copies that do not require inline verification are attempted as instant clones: the `FICLONE` ioctl on Linux (btrfs, XFS, and other reflink-capable filesystems) and `clonefile(2)` on macOS (APFS). Clones are only taken for fresh (non-resumed), non-empty files; metadata is finalized afterwards and `FileOutcome.cloned` is reported. Windows ReFS block cloning is not exposed by this layer (the stub returns false).

**Where.** `crates/ketra-core/src/platform/linux.rs::clone_file`, `platform/macos.rs::clone_file`, dispatch in `platform/mod.rs::try_clone_file`, use in `pipeline.rs::copy_inner`.

**Citation.** `FICLONE` ioctl as documented for btrfs/XFS (linux `ioctl_ficlone(2)` man page); `clonefile(2)` macOS man page (APFS copy-on-write clones).
## Unbuffered sequential I/O

**What we do.** Files at or above the device-class threshold (`DeviceProfile.unbuffered_threshold`: 512 MiB HDD, 128 MiB SSD, 32 MiB network, 256 MiB unknown; override with `unbuffered_threshold`/CLI equivalent) are opened for writing with `O_DIRECT` on Linux and `FILE_FLAG_NO_BUFFERING` + `FILE_FLAG_SEQUENTIAL_SCAN` on Windows; read handles get `SEQUENTIAL_SCAN`/`posix_fadvise(POSIX_FADV_SEQUENTIAL)` hints (`F_RDADVISE` on macOS is implemented in `macos.rs`). Direct mode requires sector-aligned buffers (`platform/aligned.rs::AlignedBuf`, 4096-byte aligned) and sector-multiple chunk sizes; the final short chunk is padded on write and the file trimmed with `set_len`. Direct I/O is disabled for files with holes (see sparse handling below). macOS `F_NOCACHE`/`F_RDADVISE`/`F_PREALLOCATE` helpers exist in `macos.rs`; the generic dispatcher currently routes readahead and preallocation through the POSIX layer, whose `posix_fadvise` implementation is Linux-only (a no-op elsewhere).

**Where.** `crates/ketra-core/src/platform/unix.rs::open_file` (custom `O_DIRECT` flag), `platform/windows.rs::open_file`, `platform/aligned.rs`, threshold selection in `pipeline.rs::unbuffered_threshold`, `advise_sequential` calls in `pipeline.rs::stream_copy` and `netmode.rs`.

**Citation.** Linux `open(2)`/`O_DIRECT` and `posix_fadvise(2)` man pages; Microsoft Win32 documentation for `CreateFileW` flags `FILE_FLAG_NO_BUFFERING` and `FILE_FLAG_SEQUENTIAL_SCAN`; macOS `fcntl(2)` `F_NOCACHE`/`F_RDADVISE`.

## Sparse hole skipping

**What we do.** With `SparseMode::Auto` (default) and a sparse-capable destination profile, the engine queries the source's data extents and copies only allocated ranges: `lseek(SEEK_DATA/SEEK_HOLE)` on Linux (with a 1,000,000-iteration guard against pathological filesystems; `ENXIO` past EOF ends the map) and `FSCTL_QUERY_ALLOCATED_RANGES` on Windows. Hole spans are verified to read back as zeros in the destination during inline verification, and digests are kept hash-correct for the *whole* file by feeding hole regions into SHA-256/XXH64 as zero bytes (`feed_zeros`), since digests must see a sequential byte stream. On filesystems without hole support the file is treated as dense.

**Where.** `crates/ketra-core/src/platform/linux.rs::hole_ranges`, `platform/windows.rs::hole_ranges`, `pipeline.rs::stream_copy` (range loop, `feed_zeros`, `hole_spans` verification).

**Citation.** Linux man page `lseek(2)` (SEEK_DATA/SEEK_HOLE, from the original Solaris sparse-file interface); Microsoft Win32 `FSCTL_QUERY_ALLOCATED_RANGES` documentation. Verified on Linux x86-64: integration test 3 checks that destination allocated block counts match the source's.

## Parallel ranged network transfer

**What we do.** For large files (>= 64 MiB, `MIN_NET_FILE_SIZE`) on network mounts, `netmode.rs` splits the file into contiguous, 4096-aligned ranges and moves one range per thread: N threads each own a contiguous range, one shared token bucket paces the aggregate rate, and AIMD tunes per-stream chunk sizes. The mode engages automatically only when the device profile classifies the destination as network (`DeviceProfile.net_streams` = 4); `--net-streams n>1` forces it, `1` disables it. It requires `--verify off|quick` because inline byte-compare cannot be preserved across independent range workers, and it is skipped for resumed files. Any stream error cancels the rest and the engine falls back to the single-stream pipeline. This is experimental: it was designed against SMB/NFS latency behavior but multi-stream gains on real networks have not been measured in this environment.

**Where.** `crates/ketra-core/src/netmode.rs` (`is_eligible`, `copy_parallel`, `range_worker`); engagement point `pipeline.rs::copy_inner`.

**Citations.** J. Shoshani et al., bbcp — "A High-Performance Distributed File Transfer Facility" (SLAC, ~2005): parallel TCP streams per file over wide-area links. W. Allcock et al., "Data Management and Transfer in High-Performance Computational Grid Environments" (2005) — GridFTP's striped/parallel transfers. Y. Gu and R. Grossman, "UDT: UDP-based Data Transfer for High-Speed Wide Area Networks", *Computer Networks* 51(7), 2007 — pacing so aggressive streams do not collapse into congestion (KetraCopy borrows the pacing idea via the shared token bucket, not the UDP protocol).

## Block-based resume

**What we do.** For an interrupted copy, `resume.rs::probe_resume` walks chunk-aligned positions backwards from the highest trustworthy bound — the journal's last recorded offset (see the durability invariant below) or the destination length. At each position it byte-compares one full chunk of source and destination; with a journal bound it additionally verifies the chunk directly *below* the accepted boundary (defense-in-depth against journal or disk corruption), and the first boundary whose checks all pass wins. Without a journal it scans every chunk and resumes from the lowest differing boundary. Each probe byte-compares both files directly — at these chunk sizes that is stronger than a rolling-checksum probe.

**Durability invariant (since 1.2.1).** The pipeline flushes the destination (Linux `fdatasync`, elsewhere `fsync`) *before* appending any batched `partial` journal record (32 MiB stride, first-chunk anchor, cancel anchor). A recoverable bound therefore implies every byte below it is already on the media; the probe's chunk checks guard against journal/disk corruption rather than unflushed data.

**Honesty note.** This is prefix probing, valid for interrupted sequential copies. It is *not* the full rsync delta-matching algorithm: KetraCopy never recomputes differing middle blocks of a changed file; if the prefix invariant does not hold (destination diverges early), the probe returns a small offset and the rest of the file is simply recopied.

**Where.** `crates/ketra-core/src/resume.rs` (`probe_resume`: trusted backward walk with below-bound verification, untrusted full scan); journal in `journal.rs`; engine wiring and journal-bounded limit in `engine.rs::run`; sync-before-record in `pipeline.rs` (`sync_dst`).

**Citation.** A. Tridgell and P. Mackerras, "The rsync algorithm", ANU Technical Report TR-CS-0606, 1996. Verified on Linux x86-64: integration test 7 interrupts a copy with SIGINT, then completes via the journal and `diff -r` confirms a byte-identical tree; unit test `finds_divergence_point` probes a boundary at a non-block-aligned offset.

## SHA-256 integrity manifests

**What we do.** Phase 3 writes `<dest>/.ketra/manifest.jsonl` — a header line (`{"t":"manifest","tool":"KetraCopy","ver":1,"time_ms":...,"root":"..."}`) plus one line per file (`{"t":"f","p":"rel/path","s":size,"m":mtime_ms,"sha256":"...","xxh64":"..."}`), paths relative and `/`-separated, sorted. `ketra manifest --root DIR` builds the same manifest for an existing tree; `ketra verify` re-hashes every listed file and reports `missing`, `size`, or `sha256` mismatches (exit code 4) — the bit-rot detector. The manifest always uses SHA-256 regardless of the `--hash` selection, and is skipped for canceled runs.

**Where.** `crates/ketra-core/src/manifest.rs` (`write`, `read`, `verify`, `build_for_tree`), digests streamed during copy in `pipeline.rs::stream_copy`, assembly in `engine.rs::build_manifest_entries`, CLI in `crates/ketra-cli/src/manifest_cmd.rs`.

**Citation.** NIST FIPS 180-4, "Secure Hash Standard" (SHA-256). Verified on Linux x86-64: SHA-256 test vectors in `hash.rs`; integration tests 1-2 write a manifest and detect a single flipped byte (exit 4).

## Digests shipped, and what is not integrated

`hash.rs` implements four algorithms from their public specifications, no third-party code:

| Algorithm | Spec | Role |
|---|---|---|
| SHA-256 | NIST FIPS 180-4 | Phase 2 `--verify full` (default) and the archival manifest |
| XXH64 | xxHash specification, Yann Collet (reference implementation BSD-2 licensed; our implementation is independent) | Fast fingerprint column in the manifest |
| CRC32C | Castagnoli polynomial, as used by iSCSI (RFC 3720, Appendix B.4) | Available via `--hash crc32c` |
| MD5 | RFC 1321 | Compatibility only; not collision-resistant, not used for manifests |

**BLAKE3** (O'Connor, Dowling, Aumasson, Neves, "BLAKE3: one function, fast everywhere", 2020) is **not integrated**. It is listed as future work: its Merkle tree hashing would parallelize digests across the netmode range workers. The shipped default (FIPS 180-4 SHA-256) was chosen because the manifest is an archival, long-lived artifact.
