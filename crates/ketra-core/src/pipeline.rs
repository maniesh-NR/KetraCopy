//! Phase 2 — the streaming copy pipeline.
//!
//! Responsibilities (each mapping to a documented engine behavior):
//!
//! * **Reflink fast path** — `clonefile` (macOS) / `FICLONE` (Linux) /
//!   ReFS block cloning (Windows) when enabled and resuming from zero.
//! * **Sparse-aware writing** — sources are scanned for holes
//!   (`SEEK_DATA`/`SEEK_HOLE`); only data extents are written at their
//!   absolute offsets, so holes never touch the destination. On platforms
//!   without hole scans, all-zero chunks are skipped instead. Files with
//!   holes are **never** preallocated (a fixed defect: preallocating made
//!   the destination dense) and hole regions disable direct I/O.
//! * **`copy_file_range` fast path** — per data extent; `EINTR` is retried
//!   in place with identical offsets.
//! * **Inline verification** — every transferred chunk is read back from
//!   the destination and compared byte-for-byte before the offset advances.
//! * **Resumable journaling** — the trusted offset is appended to the
//!   journal only after the data is on disk, so `--resume` never trusts
//!   more than what was flushed.
//! * **Cooperative cancellation** — a canceled copy keeps the partial
//!   destination file and journal record; nothing is truncated (a fixed
//!   defect where cancel fell through into a truncating path).

use crate::controller::SpeedController;
use crate::error::{Error, Result};
use crate::journal::{JournalEntry, JournalHandle};
use crate::options::{Options, VerifyMode};
use crate::platform::{self, CloneOutcome};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Platforms with kernel sparse scans (SEEK_DATA/SEEK_HOLE).
const CAN_SCAN: bool = cfg!(target_os = "linux");

/// Granularity of `partial` journal records. The resume probe re-verifies
/// content from the last recorded bound, so a coarser granularity only
/// ever re-copies a bounded tail after a crash — while shrinking the
/// journal (and its write() traffic) by ~32× versus per-chunk records:
/// a 1 TiB transfer used to append ~1M records (~60 MB of journal), it
/// now appends ~32k.
const JOURNAL_BATCH: u64 = 32 * 1024 * 1024;

/// Per-file copy statistics.
#[derive(Debug, Clone, Default)]
pub struct CopyStats {
    /// Bytes transferred by this run (excludes bytes resumed from journal).
    pub bytes: u64,
    /// Destination is a reflink of the source.
    pub cloned: bool,
    /// `copy_file_range` was used for at least one chunk.
    pub used_cfr: bool,
    /// Logical bytes skipped because they were holes / zero runs.
    pub sparse_bytes: u64,
    /// Destination offset reached when the transfer was canceled (the
    /// journal carries a record at this exact position).
    pub canceled_at: Option<u64>,
    /// SHA-256 digest of the logical destination content when the caller
    /// supplied a hasher and the file was fully streamed (set by the
    /// engine after `copy_file` returns; the pipeline only feeds bytes).
    pub hash_digest: Option<String>,
}

/// 4096-aligned scratch buffer for direct-I/O reads and writes.
struct AlignedBuf {
    ptr: *mut u8,
    cap: usize,
}

// Safety: exclusively owned raw allocation; never aliased.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn new(len: usize) -> AlignedBuf {
        let len = len.max(4096);
        let layout = std::alloc::Layout::from_size_align(len, 4096).expect("valid layout");
        unsafe {
            let ptr = std::alloc::alloc(layout) as *mut u8;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            // No zero-fill: every use reads/slices only the byte range that
            // was actually filled by `read_exact_at` first, so uninitialized
            // memory is never observed. This used to memset up to 2×chunk
            // per file — measurable on the many-small-files workload.
            AlignedBuf { ptr, cap: len }
        }
    }

    fn as_mut(&mut self, len: usize) -> &mut [u8] {
        debug_assert!(len <= self.cap);
        unsafe { std::slice::from_raw_parts_mut(self.ptr, len.min(self.cap)) }
    }

    fn as_ref(&self, len: usize) -> &[u8] {
        debug_assert!(len <= self.cap);
        unsafe { std::slice::from_raw_parts(self.ptr, len.min(self.cap)) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                let layout = std::alloc::Layout::from_size_align_unchecked(self.cap, 4096);
                std::alloc::dealloc(self.ptr as *mut u8, layout);
            }
        }
    }
}

/// Scan `file` for holes below `end`; returns hole ranges `(start, len)`.
/// An empty result means "no holes" *or* "cannot scan" (dense copy).
fn scan_holes(file: &File, end: u64) -> Vec<(u64, u64)> {
    let mut holes = Vec::new();
    let mut pos: u64 = 0;
    for _ in 0..4_096_000 {
        let data_start = match platform::seek_region(file, pos) {
            Some((d, true)) => d,
            Some((_, false)) => {
                // `pos` is inside the trailing hole.
                if pos < end {
                    holes.push((pos, end - pos));
                }
                break;
            }
            None => break, // scan unsupported
        };
        if data_start >= end {
            break;
        }
        if data_start > pos {
            holes.push((pos, data_start - pos));
        }
        match platform::next_hole(file, data_start, end) {
            Some((hstart, hlen)) => {
                // The hole itself is part of the answer (mid-file holes were
                // previously dropped here, making destinations dense).
                holes.push((hstart, hlen));
                let next = hstart + hlen;
                if next <= pos {
                    break; // defensive: no forward progress
                }
                pos = next;
                if pos >= end {
                    break;
                }
            }
            None => break,
        }
    }
    holes
}

/// Copy `src` → `dst`.
///
/// * `rel` — destination-relative path for journal records.
/// * `start_offset` — resume position (0 for a fresh copy).
/// * `progress` — called with the number of newly transferred bytes.
#[allow(clippy::too_many_arguments)]
pub fn copy_file(
    src: &Path,
    dst: &Path,
    opts: &Options,
    ctl: &mut SpeedController,
    cancel: &AtomicBool,
    journal: Option<&JournalHandle>,
    rel: &str,
    start_offset: u64,
    progress: &mut dyn FnMut(u64),
    mut hasher: Option<&mut dyn crate::hash::Hasher>,
) -> Result<CopyStats> {
    let mut stats = CopyStats::default();
    let smd = std::fs::symlink_metadata(src).map_err(|e| Error::ioctx(src, e))?;
    let size = smd.len();

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::ioctx(parent, e))?;
        }
    }

    // ---- reflink fast path (fresh full copies only) ----
    if opts.use_clone && start_offset == 0 && size > 0 {
        match platform::clone_file(src, dst) {
            Ok(CloneOutcome::Cloned) => {
                let f = File::open(dst).map_err(|e| Error::ioctx(dst, e))?;
                preserve_meta(&f, &smd, dst, opts)?;
                stats.cloned = true;
                stats.bytes = size;
                progress(size);
                if let Some(j) = journal {
                    j.record(&JournalEntry {
                        rel: rel.to_string(),
                        size,
                        chunk: opts.chunk(),
                        offset: size,
                        status: "done".into(),
                    })?;
                }
                return Ok(stats);
            }
            Ok(CloneOutcome::NotSupported) => {}
            // `create_new` semantics: the destination already exists (an
            // overwrite resolved by the conflict policy — `--conflict
            // overwrite`, the GUI's overwrite-all — or a create race).
            // Fall through to the streaming path, which opens with plain
            // create/truncate semantics (recorded defect fix: this used to
            // fail the whole file with EEXIST).
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::ioctx(dst, e)),
        }
    }

    // ---- open source, scan holes ----
    let srcf = open_with_retry(retries_of(opts), || {
        OpenOptions::new().read(true).open(src)
    }).map_err(|e| Error::ioctx(src, e))?;
    // Kernel readahead hint for the sequential pass-through (Linux fadvise,
    // macOS fcntl; no-ops elsewhere).
    platform::bypass_cache(&srcf);
    let holes = if opts.use_sparse && size > 0 { scan_holes(&srcf, size) } else { Vec::new() };
    if std::env::var_os("KETRA_DEBUG_HOLES").is_some() {
        eprintln!("debug: holes found: {:?}", holes);
    }

    // Files with holes must not use direct I/O (recorded defect fix) and
    // must not be preallocated (prealloc made the destination dense).
    let direct = opts.direct_io && holes.is_empty() && aligned_enough(opts, start_offset);

    // ---- open destination ----
    let dstf = if direct {
        match open_with_retry(retries_of(opts), || {
            platform::open_direct(dst, true).ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, "O_DIRECT unavailable")
            })
        }) {
            Ok(f) => f,
            // O_DIRECT refused (unsupported FS, alignment, …): buffered IO.
            Err(_) => buffered_open(dst, start_offset, opts).map_err(|e| Error::ioctx(dst, e))?,
        }
    } else {
        buffered_open(dst, start_offset, opts).map_err(|e| Error::ioctx(dst, e))?
    };
    platform::set_sparse(&dstf).ok(); // best effort (Windows/NTFS)

    // Preallocate only dense, fresh, non-direct files.
    if holes.is_empty() && size > 0 && start_offset == 0 && !direct {
        platform::prealloc(&dstf, size).ok();
    }

    // Punch known holes when resuming over an older partial file.
    if start_offset > 0 {
        for (hoff, hlen) in &holes {
            if *hoff < start_offset {
                platform::punch_hole(&dstf, *hoff, *hlen).ok();
            }
        }
    }

    // ---- DATA extents to transfer (holes skipped entirely) ----
    let extents: Vec<(u64, u64)> = if holes.is_empty() {
        vec![(0, size)]
    } else {
        let mut v = Vec::new();
        let mut cur = 0u64;
        for (hstart, hlen) in &holes {
            if *hstart > cur {
                v.push((cur, *hstart));
            }
            cur = hstart + hlen;
        }
        if cur < size {
            v.push((cur, size));
        }
        v
    };

    let chunk = opts.chunk() as usize;
    // Set when the source reads shorter than the planned extent mid-copy.
    let mut src_shrank = false;
    let mut buf = AlignedBuf::new(chunk);
    // Inline-verify read-back buffer: only allocated when verification is
    // actually requested (one allocation less per file otherwise).
    let mut back = if opts.verify == VerifyMode::Inline {
        Some(AlignedBuf::new(chunk))
    } else {
        None
    };
    // Inline hashing must see every byte in userspace, so the kernel-side
    // copy_file_range fast path is bypassed for hashed files.
    let mut use_cfr = opts.use_cfr && !direct && hasher.is_none();
    let retries = retries_of(opts);

    // Reusable zero run for hole regions (only when hashing): hole bytes
    // are part of the logical file content a full read would see.
    let mut zero_buf: Vec<u8> = if hasher.is_some() { vec![0u8; chunk] } else { Vec::new() };
    // Logical destination offset up to which `hasher` has digested.
    let mut hashed_to: u64 = 0;

    let mut pos = start_offset;
    let mut last_journaled = start_offset;
    // The first completed chunk of a file is always journaled immediately:
    // a hard kill (SIGKILL / power loss) before the first batch stride would
    // otherwise leave no record at all, and a `--resume` run would fall into
    // the conflict policy instead of probing and continuing the file.
    let mut journaled_first = start_offset > 0;
    let mut cancelled = false;
    let mut hit_eof = false;

    'extents: for (estart, eend) in extents {
        // Hole bytes between the last hashed offset and this extent are
        // part of the logical file: digest them as zeros (CPU only — no
        // I/O; identical to what a full read of the destination sees).
        if estart > hashed_to {
            if let Some(h) = hasher.as_mut() {
                let mut left = estart - hashed_to;
                while left > 0 {
                    let n = (left as usize).min(chunk);
                    h.update(&zero_buf[..n]);
                    left -= n as u64;
                }
            }
            hashed_to = estart;
        }
        let mut off = estart.max(start_offset);
        while off < eend {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break 'extents;
            }
            let len = ((eend - off) as usize).min(chunk);
            if len == 0 {
                break;
            }

            ctl.before_chunk(len as u64);
            let t0 = Instant::now();

            // Async read-ahead of the next chunk: the kernel pulls it from
            // the source while this chunk is written (fadvise WILLNEED on
            // Linux/macOS, no-op elsewhere). Direct I/O is left alone —
            // O_DIRECT manages its own synchronization.
            if !direct {
                let next = off as u64 + len as u64;
                if next < eend {
                    platform::read_advise(&srcf, next, ((eend - next) as usize).min(chunk) as u64);
                }
            }

            // ---- transport ----
            let mut moved = 0usize;
            let mut cfr_moved = 0usize;
            let mut cfr_dead = false;
            if use_cfr {
                let mut remaining = len;
                while remaining > 0 {
                    match platform::copy_range(&srcf, &dstf, off + moved as u64, remaining as u64) {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            moved += n as usize;
                            cfr_moved += n as usize;
                            remaining -= n as usize;
                            stats.used_cfr = true;
                        }
                        Err(e) if cfr_unsupported(&e) => {
                            cfr_dead = true;
                            break;
                        }
                        Err(e) => {
                            if cancel.load(Ordering::Relaxed) {
                                cancelled = true;
                            }
                            break 'extents;
                        }
                    }
                }
                if cfr_dead && cfr_moved == 0 {
                    use_cfr = false; // kernel refused: fall back permanently
                }
            }
            if moved < len {
                // Streaming read/write (fallback, top-up, or primary path).
                let src_chunk =
                    read_exact_at(&srcf, buf.as_mut(len - moved), off + moved as u64, retries)?;
                if src_chunk == 0 && moved == 0 {
                    // Source shrank below the planned extent mid-copy.
                    // Zero-padding to the planned size (set_len) would
                    // silently corrupt the destination and journal it
                    // `done`; fail loudly instead and keep the partial
                    // (plus journal partials) for --resume.
                    src_shrank = true;
                    hit_eof = true;
                    break;
                }
                let zero_run = !CAN_SCAN
                    && opts.use_sparse
                    && start_offset == 0
                    && is_all_zero(buf.as_ref(src_chunk));
                // Hash the source bytes regardless of whether they are
                // written: zero-run chunks are still zeros in the logical
                // destination content.
                if src_chunk > 0 {
                    if let Some(h) = hasher.as_mut() {
                        h.update(buf.as_ref(src_chunk));
                    }
                }
                if src_chunk > 0 && !zero_run {
                    write_exact_at(&dstf, buf.as_ref(src_chunk), off + moved as u64, retries)?;
                } else if src_chunk > 0 {
                    stats.sparse_bytes += src_chunk as u64;
                }
                moved += src_chunk;
            }

            let dt = t0.elapsed();

            // ---- inline byte comparison ----
            if opts.verify == VerifyMode::Inline && moved > 0 {
                // When the whole chunk was streamed through `buf`, the
                // source bytes are still in memory — compare the destination
                // read-back against them instead of re-reading the source
                // (one full read less per chunk, same corruption coverage).
                // Kernel-side copies (copy_file_range) bypass `buf`, so
                // those chunks still fetch a fresh source view.
                let n = if cfr_moved == 0 {
                    moved
                } else {
                    read_exact_at(&srcf, buf.as_mut(moved), off, retries)?
                };
                let nb = read_exact_at(&dstf, back.as_mut().unwrap().as_mut(moved), off, retries)?;
                if n != moved || nb != moved || back.as_ref().unwrap().as_ref(nb) != buf.as_ref(n) {
                    return Err(Error::Verify { path: rel.to_string(), offset: off });
                }
            }

            // ---- journal after the data is durable ----
            // `partial` records are batched (see JOURNAL_BATCH), but a
            // bound may only become durable AFTER the data it covers:
            // sync before every batched record. fdatasync/fsync flush
            // every dirty page of the file — including writes from a
            // previous crashed run — so any recoverable `partial` bound
            // implies all bytes below it are on the media, and the resume
            // probe can trust the bound (its tail check then only guards
            // against journal/disk corruption). The `done` record is
            // written once at finalize, after the full-file sync below.
            pos = off + moved as u64;
            if let Some(j) = journal {
                if !journaled_first || pos - last_journaled >= JOURNAL_BATCH {
                    sync_dst(&dstf, dst)?;
                    j.record(&JournalEntry {
                        rel: rel.to_string(),
                        size,
                        chunk: opts.chunk(),
                        offset: pos,
                        status: "partial".into(),
                    })?;
                    last_journaled = pos;
                    journaled_first = true;
                }
            }

            progress(moved as u64);
            stats.bytes += moved as u64;
            hashed_to = pos;

            // AIMD signal: a stall = much slower than the current target.
            let target = ctl.effective_rate();
            let expected = if target > 0.0 {
                Duration::from_secs_f64(moved as f64 / target)
            } else {
                Duration::ZERO
            };
            let stalled = dt > expected * 4 && dt > Duration::from_millis(100);
            ctl.after_chunk(moved as u64, stalled);

            if moved == 0 {
                hit_eof = true;
                break;
            }
            off += moved as u64;
        }
        if hit_eof {
            break;
        }
    }

    if cancelled {
        // Keep the partial file + journal records for --resume; the fixed
        // defect let cancellation fall through into a truncating path.
        // Data written after the last batched record is still on disk:
        // record the exact position so `--resume` can trust it (the probe
        // re-verifies content regardless).
        if let Some(j) = journal {
            if pos > last_journaled {
                // Same durability invariant: sync data before the record.
                let _ = sync_dst(&dstf, dst);
                let _ = j.record(&JournalEntry {
                    rel: rel.to_string(),
                    size,
                    chunk: opts.chunk(),
                    offset: pos,
                    status: "partial".into(),
                });
            }
        }
        stats.canceled_at = Some(pos);
        return Err(Error::Canceled);
    }

    if src_shrank {
        // Fail loudly: the destination is a partial copy of a now-shorter
        // source. Leave it (and any journal partials) for --resume, which
        // re-probes content and restarts the file when the source is
        // shorter than the destination.
        return Err(Error::Shrank {
            path: rel.to_string(),
            copied: pos,
            planned: size,
        });
    }

    // ---- finalize: extend/truncate to the exact logical size ----
    dstf.set_len(size).map_err(|e| Error::ioctx(dst, e))?;
    // Trailing hole beyond the last data extent is logically zeros — part
    // of the file a full read would see. Hash it so the inline digest
    // matches what Phase 3 would compute on the destination.
    if size > hashed_to {
        if let Some(h) = hasher.as_mut() {
            let mut left = size - hashed_to;
            while left > 0 {
                let n = (left as usize).min(chunk);
                h.update(&zero_buf[..n]);
                left -= n as u64;
            }
        }
    }
    // Data durability: on Linux fdatasync skips inode metadata the copy
    // does not change (mtime is set afterwards anyway); everywhere else
    // sync_all is the strongest portable flush.
    #[cfg(target_os = "linux")]
    dstf.sync_data().map_err(|e| Error::ioctx(dst, e))?;
    #[cfg(not(target_os = "linux"))]
    dstf.sync_all().map_err(|e| Error::ioctx(dst, e))?;
    preserve_meta(&dstf, &smd, dst, opts)?;

    if let Some(j) = journal {
        j.record(&JournalEntry {
            rel: rel.to_string(),
            size,
            chunk: opts.chunk(),
            offset: size,
            status: "done".into(),
        })?;
    }
    Ok(stats)
}

/// Flush destination data (Linux: fdatasync; elsewhere: fsync) so journal
/// bounds only ever become durable after the data they cover.
fn sync_dst(dstf: &File, dst: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    let r = dstf.sync_data();
    #[cfg(not(target_os = "linux"))]
    let r = dstf.sync_all();
    r.map_err(|e| Error::ioctx(dst, e))
}

/// `copy_file_range` rejections that mean "unsupported here", not "error".
fn cfr_unsupported(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices
    ) || matches!(e.raw_os_error(), Some(22) /* EINVAL */ | Some(18) /* EXDEV */ | Some(38) /* ENOSYS */ | Some(9) /* EBADF */)
}

fn aligned_enough(opts: &Options, start_offset: u64) -> bool {
    opts.chunk() % 4096 == 0 && start_offset % 4096 == 0
}

/// `--retries` value for this run (0 disables retrying).
fn retries_of(opts: &Options) -> u32 {
    opts.retries
}

/// Transient I/O conditions worth a short backoff: interrupted syscalls,
/// busy devices, dying links — and (Windows only) sharing violations,
/// the classic "file in use by another process" case TeraCopy waits out.
pub(crate) fn retryable(e: &io::Error) -> bool {
    match e.kind() {
        io::ErrorKind::Interrupted | io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => true,
        _ => {
            #[cfg(windows)]
            if matches!(e.raw_os_error(), Some(32) | Some(33)) {
                // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
                return true;
            }
            matches!(e.raw_os_error(), Some(5) /* EIO */ | Some(16) /* EBUSY */)
        }
    }
}

/// Run `f`, retrying transient errors up to `retries` times with a short
/// linear backoff (25 ms per attempt, capped at 250 ms). Shared with the
/// multi-stream network path (netmode).
pub(crate) fn with_retry<T>(retries: u32, mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempt = 0u32;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if attempt < retries && retryable(&e) => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis((25u64 * attempt as u64).min(250)));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Open helper honoring the retry policy (sharing violations surface at
/// open time on Windows).
fn open_with_retry<T, F>(retries: u32, f: F) -> io::Result<T>
where
    F: Fn() -> io::Result<T>,
{
    with_retry(retries, f)
}

/// Buffered (non-direct) destination open with retry.
fn buffered_open(dst: &Path, start_offset: u64, opts: &Options) -> io::Result<File> {
    open_with_retry(opts.retries, || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(start_offset == 0)
            .open(dst)
    })
}

fn is_all_zero(b: &[u8]) -> bool {
    // Word-at-a-time (auto-vectorized): the byte iterator spent ~4 ns/byte,
    // which showed up on the (non-Linux) zero-run path for every chunk of
    // sparse files.
    let mut chunks = b.chunks_exact(8);
    if !chunks.all(|c| {
        u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) == 0
    }) {
        return false;
    }
    chunks.remainder().iter().all(|&x| x == 0)
}

/// pread loop; short reads tolerated only at EOF. Retries transient
/// failures per `retries`.
fn read_exact_at(f: &File, buf: &mut [u8], off: u64, retries: u32) -> Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        let n = with_retry(retries, || {
            crate::platform::read_at(f, &mut buf[done..], off + done as u64)
        })
        .map_err(|e| Error::Io { path: String::new(), source: e })?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(done)
}

/// pwrite loop; offsets are absolute so hole gaps never append
/// (recorded defect fix: hole zeros must interleave at their `dpos`).
fn write_exact_at(f: &File, buf: &[u8], off: u64, retries: u32) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let n = with_retry(retries, || {
            crate::platform::write_at(f, &buf[done..], off + done as u64)
        })
        .map_err(|e| Error::Io { path: String::new(), source: e })?;
        if n == 0 {
            return Err(Error::Io {
                path: String::new(),
                source: io::Error::new(io::ErrorKind::WriteZero, "short write"),
            });
        }
        done += n;
    }
    Ok(())
}

fn preserve_meta(f: &File, smd: &std::fs::Metadata, dst: &Path, opts: &Options) -> Result<()> {
    if opts.preserve_mtime {
        if let Ok(mtime) = smd.modified() {
            let mut times = std::fs::FileTimes::new();
            times.set_modified(mtime);
            if let Ok(atime) = smd.accessed() {
                times.set_accessed(atime);
            }
            f.set_times(times).map_err(|e| Error::ioctx(dst, e))?;
        }
    }
    if opts.preserve_perms {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = crate::platform::st_mode(smd) & 0o7777;
            std::fs::set_permissions(dst, std::fs::Permissions::from_mode(mode))
                .map_err(|e| Error::ioctx(dst, e))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (smd, dst);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::SpeedController;
    use crate::journal::{Journal, JournalHandle};
    use crate::options::Options;
    use std::sync::atomic::AtomicBool;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-pipe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sparse_file(p: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new()
            .read(true).write(true).create(true).truncate(true)
            .open(p).unwrap();
        f.seek(SeekFrom::Start(1024 * 1024)).unwrap();
        f.write_all(&vec![0x5Au8; 64 * 1024]).unwrap();
        f.seek(SeekFrom::Start(1024 * 1024 + 64 * 1024 + 1536 * 1024)).unwrap();
        f.write_all(&vec![0xC3u8; 128 * 1024]).unwrap();
        f.set_len(1024 * 1024 + 64 * 1024 + 1536 * 1024 + 128 * 1024 + 256 * 1024).unwrap();
        f.sync_all().unwrap();
    }

    /// The inline digest fed through the copy loop must equal a full,
    /// independent SHA-256 read of the destination — for sparse
    /// (multi-hole), dense and empty files alike.
    #[test]
    fn test_inline_hash_matches_full_read() {
        let dir = tmp("hash");
        sparse_file(&dir.join("s.bin"));
        let dense: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("d.bin"), &dense).unwrap();
        std::fs::write(dir.join("e.bin"), b"").unwrap();

        for name in ["s.bin", "d.bin", "e.bin"] {
            let dst = dir.join(format!("out-{name}"));
            let mut h = crate::hash::by_name("sha256").unwrap();
            let r = copy_file(
                &dir.join(name), &dst,
                &Options { use_clone: false, ..Options::default() },
                &mut SpeedController::new(None), &AtomicBool::new(false),
                None, name, 0, &mut |_| {}, Some(&mut *h),
            ).unwrap();
            assert!(!r.cloned && !r.used_cfr, "hashing must force the streaming path");
            let inline = h.finish_hex();
            let full = crate::options::hash_file(&dst, "sha256", 65536, &AtomicBool::new(false))
                .unwrap();
            assert_eq!(inline, full, "inline digest must match a full read of {name}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Journal batching: a completed small file leaves exactly one `done`
    /// record (no per-chunk `partial` spam), and a canceled run keeps the
    /// partial file on disk with a parseable journal.
    #[test]
    fn test_journal_batching_done_and_cancel() {
        let dir = tmp("jrnl");
        let src = dir.join("a.bin");
        std::fs::write(&src, vec![7u8; 64 * 1024]).unwrap();
        let jpath = dir.join(".ketracopy-journal.jsonl");
        let j = JournalHandle::new(Journal::open(&jpath).unwrap());

        let dst = dir.join("out.bin");
        copy_file(
            &src, &dst,
            &Options { use_clone: false, ..Options::default() },
            &mut SpeedController::new(None), &AtomicBool::new(false),
            Some(&j), "a.bin", 0, &mut |_| {}, None,
        ).unwrap();
        let entries = crate::journal::load(&jpath).unwrap();
        assert_eq!(entries.len(), 2, "first-chunk partial + done, no per-chunk spam: {entries:?}");
        assert_eq!(entries[0].status, "partial");
        assert_eq!(entries[0].offset, 64 * 1024, "first chunk is journaled immediately");
        assert_eq!(entries[1].status, "done");
        assert_eq!(entries[1].offset, 64 * 1024);

        // Cancel before the first chunk: nothing is journaled (nothing was
        // written), the partial file stays for a later run.
        let dst2 = dir.join("out2.bin");
        let cancel = AtomicBool::new(true);
        let r = copy_file(
            &src, &dst2,
            &Options { use_clone: false, ..Options::default() },
            &mut SpeedController::new(None), &cancel,
            Some(&j), "a2.bin", 0, &mut |_| {}, None,
        );
        assert!(r.is_err());
        assert!(dst2.exists(), "canceled copy must keep the partial file");
        let entries = crate::journal::load(&jpath).unwrap();
        assert_eq!(entries.len(), 2, "no record added for a zero-progress file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Overwriting an existing destination must not die on the reflink
    /// fast path (`create_new` → EEXIST) — it must fall through to the
    /// streaming path (found while fixing the Gate-1 `--no-times` defect,
    /// which used to mask it by skipping).
    #[test]
    fn test_overwrite_existing_dst_with_clone_falls_through() {
        let dir = tmp("ovw");
        let src = dir.join("s.bin");
        std::fs::write(&src, b"NEW-DATA-8").unwrap();
        let dst = dir.join("out.bin");
        std::fs::write(&dst, b"OLD").unwrap();
        copy_file(
            &src, &dst,
            &Options { use_clone: true, ..Options::default() },
            &mut SpeedController::new(None), &AtomicBool::new(false),
            None, "s.bin", 0, &mut |_| {}, None,
        )
        .unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"NEW-DATA-8");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A source that shrinks below its planned extent mid-copy must fail
    /// loudly — never zero-pad to the planned size and journal it `done`.
    #[test]
    fn test_source_shrink_fails_loudly() {
        let dir = tmp("shrink");
        let src = dir.join("s.bin");
        // Dense 3 MiB: three 1 MiB chunks (default chunk).
        std::fs::write(&src, vec![7u8; 3 * 1024 * 1024]).unwrap();
        let jpath = dir.join(".ketracopy-journal.jsonl");
        let j = JournalHandle::new(Journal::open(&jpath).unwrap());
        let dst = dir.join("out.bin");
        // Shrink the source to 1.5 MiB while the first chunk is being
        // reported: the next read hits EOF before the planned extent.
        let src2 = src.clone();
        let mut shrank = false;
        let r = copy_file(
            &src, &dst,
            &Options { use_clone: false, ..Options::default() },
            &mut SpeedController::new(None), &AtomicBool::new(false),
            Some(&j), "s.bin", 0,
            &mut |_| {
                if !shrank {
                    shrank = true;
                    let f = std::fs::OpenOptions::new().write(true).open(&src2).unwrap();
                    f.set_len(1536 * 1024).unwrap();
                }
            },
            None,
        );
        let err = match r {
            Err(e) => e,
            Ok(_) => panic!("expected Error::Shrank, got Ok"),
        };
        match &err {
            Error::Shrank { copied, planned, .. } => {
                assert_eq!(*planned, 3 * 1024 * 1024);
                assert!(*copied >= 1024 * 1024, "copied={copied}");
            }
            other => panic!("expected Error::Shrank, got {other:?}"),
        }
        // The destination is preallocated to the planned size up front, so
        // its length alone says nothing — the meaningful guarantees are the
        // loud error, no `done` record, and a resume that restarts a file
        // longer than its (now shorter) source.
        assert!(
            matches!(err, Error::Shrank { .. }),
            "shrink must surface as Error::Shrank"
        );
        // No `done` record for the shrunken file.
        let entries = crate::journal::load(&jpath).unwrap();
        assert!(
            entries.iter().all(|e| e.status != "done"),
            "no done record after shrink: {entries:?}"
        );
        // A fresh copy of the (now shorter) source succeeds and is exact.
        let dst2 = dir.join("out2.bin");
        copy_file(
            &src, &dst2,
            &Options { use_clone: false, ..Options::default() },
            &mut SpeedController::new(None), &AtomicBool::new(false),
            None, "s.bin", 0, &mut |_| {}, None,
        )
        .unwrap();
        assert_eq!(std::fs::metadata(&dst2).unwrap().len(), 1536 * 1024);
        std::fs::remove_dir_all(&dir).ok();
    }
}
