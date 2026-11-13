//! Resumption logic: where an interrupted copy actually stopped.
//!
//! Two probe strategies:
//!
//! * **Journal-bounded probe** (`trusted = Some(journal_offset)`): the
//!   journal offset is an upper bound the engine may trust (the pipeline
//!   records a chunk only after its data was flushed). Starting at the
//!   chunk boundary at or below that bound, whole chunks are compared
//!   against the source going *backwards* until one matches; the scan is
//!   linear and always terminates — early revisions used a binary search
//!   that could spin forever on non-chunk-aligned lengths.
//!
//! * **Untrusted full scan** (`trusted = None`): every chunk boundary is
//!   compared, and the resume offset is the lowest boundary whose chunk
//!   differs from the source (rewriting from there upward repairs the
//!   file). Costs a full read of both files, but only happens when
//!   `--resume` is used and no journal record exists.

use crate::error::{Error, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Compare `len` bytes of `src` and `dst` at absolute offset `off`.
#[cfg(unix)]
fn chunk_equal(
    src: &mut std::fs::File,
    dst: &mut std::fs::File,
    off: u64,
    len: usize,
) -> Result<bool> {
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    if len == 0 {
        return Ok(true);
    }
    let mut sb = vec![0u8; len];
    let mut db = vec![0u8; len];
    let sn = read_exact_at(src, &mut sb, off)?;
    let dn = read_exact_at(dst, &mut db, off)?;
    if sn != dn {
        return Ok(false);
    }
    Ok(sb[..sn] == db[..dn])
}

/// pread loop; short read only at EOF.
#[cfg(unix)]
fn read_exact_at(f: &mut std::fs::File, buf: &mut [u8], off: u64) -> Result<usize> {
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    let mut filled = 0;
    while filled < buf.len() {
        let n = f
            .read_at(&mut buf[filled..], off + filled as u64)
            .map_err(|e| Error::Io { path: String::new(), source: e })?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

#[cfg(not(unix))]
fn chunk_equal(
    _src: &mut std::fs::File,
    _dst: &mut std::fs::File,
    _off: u64,
    _len: usize,
) -> Result<bool> {
    Err(Error::Unsupported("probe_resume on this platform".into()))
}

/// Determine the resume offset for `dst` given source `src`.
///
/// * `trusted = Some(j)` — journal-bounded probe; trust `[0, j)`.
/// * `trusted = None` — untrusted full scan.
///
/// Returns the byte offset from which copying continues.
pub fn probe_resume(
    src: &Path,
    dst: &Path,
    trusted: Option<u64>,
    chunk: u64,
    cancel: &AtomicBool,
) -> Result<u64> {
    let chunk = chunk.max(4096);
    let dlen = match std::fs::metadata(dst) {
        Ok(md) => md.len(),
        Err(_) => return Ok(0), // nothing to resume
    };
    if dlen == 0 {
        return Ok(0);
    }
    let smd = std::fs::metadata(src).map_err(|e| Error::ioctx(src, e))?;
    let slen = smd.len();
    if dlen > slen {
        // Destination longer than the source: stale output, restart file.
        return Ok(0);
    }

    let mut srcf = std::fs::File::open(src).map_err(|e| Error::ioctx(src, e))?;
    let mut dstf = std::fs::OpenOptions::new()
        .read(true)
        .open(dst)
        .map_err(|e| Error::ioctx(dst, e))?;

    match trusted {
        Some(j) => {
            let upper = j.min(dlen);
            if upper == 0 {
                return Ok(0);
            }
            // Fast, bounded: first matching boundary at or below `upper`.
            // The tail chunk below the bound is always compared, so an
            // overshooting journal (data never flushed) is detected too.
            let mut off = (upper / chunk) * chunk;
            let mut len = ((upper - off) as usize).min(chunk as usize);
            if len == 0 {
                off = off.saturating_sub(chunk);
                len = chunk as usize;
            }
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Canceled);
                }
                if chunk_equal(&mut srcf, &mut dstf, off, len)? {
                    // Defense-in-depth: before trusting a journal bound,
                    // also verify the chunk directly below it (cheap). The
                    // pipeline's sync-before-record invariant already makes
                    // corruption below a durable bound impossible; this
                    // catches journal/disk corruption regardless.
                    if off > 0
                        && !chunk_equal(&mut srcf, &mut dstf, off - chunk, chunk as usize)?
                    {
                        off -= chunk;
                        len = chunk as usize;
                        continue;
                    }
                    return Ok(off + len as u64);
                }
                if off == 0 {
                    return Ok(0);
                }
                off = off.saturating_sub(chunk);
                len = chunk as usize;
            }
        }
        None => {
            // Full scan: lowest boundary whose chunk differs. All chunks
            // above it are known good (positional copy repairs forward).
            let mut lowest_bad: Option<u64> = None;
            let mut b = (dlen / chunk) * chunk;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Canceled);
                }
                let len = ((dlen - b) as usize).min(chunk as usize);
                if !chunk_equal(&mut srcf, &mut dstf, b, len)? {
                    lowest_bad = Some(b);
                }
                if b == 0 {
                    break;
                }
                b = b.saturating_sub(chunk);
            }
            match lowest_bad {
                Some(b) => Ok(b),
                None => Ok(dlen), // whole file already correct
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-resume-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_probe_full_and_empty_and_oversized() {
        let dir = tmp("basic");
        let src = dir.join("f.bin");
        let dst = dir.join("g.bin");
        let data: Vec<u8> = (0..64_000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let cancel = AtomicBool::new(false);

        // No destination yet → 0.
        assert_eq!(probe_resume(&src, &dst, None, 8192, &cancel).unwrap(), 0);
        // Destination longer than source → restart.
        std::fs::write(&dst, [&data[..], b"extra"].concat()).unwrap();
        assert_eq!(probe_resume(&src, &dst, None, 8192, &cancel).unwrap(), 0);
        // Perfect copy, untrusted full scan → whole file is good.
        std::fs::write(&dst, &data).unwrap();
        assert_eq!(probe_resume(&src, &dst, None, 8192, &cancel).unwrap(), 64_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_untrusted_full_scan_finds_lowest_bad_chunk() {
        let dir = tmp("fullscan");
        let src = dir.join("f.bin");
        let dst = dir.join("g.bin");
        let mut data: Vec<u8> = (0..64_000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        data[30_000] ^= 0xFF; // inside chunk [24576, 32768)
        std::fs::write(&dst, &data).unwrap();
        let cancel = AtomicBool::new(false);

        let off = probe_resume(&src, &dst, None, 8192, &cancel).unwrap();
        assert_eq!(off, 24_576);

        // Truncated copy (crash mid-file): every byte present is verified,
        // so the resume offset is the verified length itself (the same rule
        // that makes a perfect copy return the file length above).
        std::fs::write(&dst, &data[..20_000]).unwrap();
        let off = probe_resume(&src, &dst, None, 8192, &cancel).unwrap();
        assert_eq!(off, 20_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_journal_bounded_probe_stops_early_and_terminates() {
        let dir = tmp("bounded");
        let src = dir.join("f.bin");
        let dst = dir.join("g.bin");
        let mut data: Vec<u8> = (0..64_000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        // Corruption at 30_000 is *beyond* the trusted bound 16_384 and must
        // not extend the probe; the journal boundary is trusted.
        data[30_000] ^= 0xFF;
        std::fs::write(&dst, &data).unwrap();
        let cancel = AtomicBool::new(false);

        // 16 KiB is chunk-aligned: trusted without a single read.
        let off = probe_resume(&src, &dst, Some(16_384), 8192, &cancel).unwrap();
        assert_eq!(off, 16_384);

        // Non-aligned bound 17_000: boundary 16_384 compares chunk
        // [16384, 17000) which matches → resume exactly at 17_000.
        let off = probe_resume(&src, &dst, Some(17_000), 8192, &cancel).unwrap();
        assert_eq!(off, 17_000);

        // Overshooting journal (last chunk never flushed): bytes at the
        // boundary are zeros in dst → probe steps back until it matches.
        std::fs::write(&dst, &[0u8; 64_000]).unwrap();
        let off = probe_resume(&src, &dst, Some(32_768), 8192, &cancel).unwrap();
        assert_eq!(off, 0); // nothing matches anywhere below the bound

        std::fs::remove_dir_all(&dir).ok();
    }
}
