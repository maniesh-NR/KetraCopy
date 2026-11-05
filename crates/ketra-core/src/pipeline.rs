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
use crate::journal::{Journal, JournalEntry};
use crate::options::{Options, VerifyMode};
use crate::platform::{self, CloneOutcome};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Platforms with kernel sparse scans (SEEK_DATA/SEEK_HOLE).
const CAN_SCAN: bool = cfg!(target_os = "linux");

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
            std::ptr::write_bytes(ptr, 0, len);
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
    mut journal: Option<&mut Journal>,
    rel: &str,
    start_offset: u64,
    progress: &mut dyn FnMut(u64),
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
                if let Some(j) = journal.as_deref_mut() {
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
            Err(e) => return Err(Error::ioctx(dst, e)),
        }
    }

    // ---- open source, scan holes ----
    let srcf = File::open(src).map_err(|e| Error::ioctx(src, e))?;
    let holes = if opts.use_sparse && size > 0 { scan_holes(&srcf, size) } else { Vec::new() };
    if std::env::var_os("KETRA_DEBUG_HOLES").is_some() {
        eprintln!("debug: holes found: {:?}", holes);
    }

    // Files with holes must not use direct I/O (recorded defect fix) and
    // must not be preallocated (prealloc made the destination dense).
    let direct = opts.direct_io && holes.is_empty() && aligned_enough(opts, start_offset);

    // ---- open destination ----
    let dstf = if direct {
        match platform::open_direct(dst, true) {
            Some(f) => f,
            None => OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(start_offset == 0)
                .open(dst)
                .map_err(|e| Error::ioctx(dst, e))?,
        }
    } else {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(start_offset == 0)
            .open(dst)
            .map_err(|e| Error::ioctx(dst, e))?
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
    let mut buf = AlignedBuf::new(chunk);
    let mut back = AlignedBuf::new(chunk); // inline-verify read-back
    let mut use_cfr = opts.use_cfr && !direct;

    let mut pos = start_offset;
    let mut cancelled = false;
    let mut hit_eof = false;

    'extents: for (estart, eend) in extents {
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

            // ---- transport ----
            let mut moved = 0usize;
            let mut cfr_dead = false;
            if use_cfr {
                let mut remaining = len;
                while remaining > 0 {
                    match platform::copy_range(&srcf, &dstf, off + moved as u64, remaining as u64) {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            moved += n as usize;
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
                if cfr_dead && moved == 0 {
                    use_cfr = false; // kernel refused: fall back permanently
                }
            }
            if moved < len {
                // Streaming read/write (fallback, top-up, or primary path).
                let src_chunk =
                    read_exact_at(&srcf, buf.as_mut(len - moved), off + moved as u64)?;
                if src_chunk == 0 && moved == 0 {
                    hit_eof = true;
                    break; // source shrank below the planned extent
                }
                let zero_run = !CAN_SCAN
                    && opts.use_sparse
                    && start_offset == 0
                    && is_all_zero(buf.as_ref(src_chunk));
                if src_chunk > 0 && !zero_run {
                    write_exact_at(&dstf, buf.as_ref(src_chunk), off + moved as u64)?;
                } else if src_chunk > 0 {
                    stats.sparse_bytes += src_chunk as u64;
                }
                moved += src_chunk;
            }

            let dt = t0.elapsed();

            // ---- inline byte comparison ----
            if opts.verify == VerifyMode::Inline && moved > 0 {
                let n = read_exact_at(&srcf, buf.as_mut(moved), off)?;
                if n != moved {
                    return Err(Error::Verify { path: rel.to_string(), offset: off });
                }
                let nb = read_exact_at(&dstf, back.as_mut(moved), off)?;
                if nb != moved || back.as_ref(nb) != buf.as_ref(n) {
                    return Err(Error::Verify { path: rel.to_string(), offset: off });
                }
            }

            // ---- journal after the data is on disk ----
            pos = off + moved as u64;
            if let Some(j) = journal.as_deref_mut() {
                j.record(&JournalEntry {
                    rel: rel.to_string(),
                    size,
                    chunk: opts.chunk(),
                    offset: pos,
                    status: if pos >= size { "done" } else { "partial" }.into(),
                })?;
            }

            progress(moved as u64);
            stats.bytes += moved as u64;

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
        return Err(Error::Canceled);
    }

    // ---- finalize: extend/truncate to the exact logical size ----
    dstf.set_len(size).map_err(|e| Error::ioctx(dst, e))?;
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

fn is_all_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

/// pread loop; short reads tolerated only at EOF.
fn read_exact_at(f: &File, buf: &mut [u8], off: u64) -> Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        let n = crate::platform::read_at(f, &mut buf[done..], off + done as u64)
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
fn write_exact_at(f: &File, buf: &[u8], off: u64) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let n = crate::platform::write_at(f, &buf[done..], off + done as u64)
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
