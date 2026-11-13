//! User-selectable copy options.

use crate::hash::Hasher;

/// Verification strategy (Phase 2 / Phase 3 of the pipeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// No verification.
    Off,
    /// Inline byte-for-byte comparison of every written chunk.
    Inline,
    /// SHA-256 manifest written after the copy (Phase 3).
    Manifest,
}

/// What to do when the destination file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Interactively ask (CLI provides the callback).
    Ask,
    /// Keep the existing destination file.
    Skip,
    /// Replace the destination file.
    Overwrite,
    /// Copy next to it with a numeric suffix.
    Rename,
    /// Copy only when the source is newer.
    KeepNewer,
}

/// Everything a copy/move run can be configured with.
#[derive(Debug, Clone)]
pub struct Options {
    pub verify: VerifyMode,
    pub conflict: OnConflict,
    /// Preserve modification times (default true).
    pub preserve_mtime: bool,
    /// Preserve unix permission bits (default true).
    pub preserve_perms: bool,
    /// Ignore the directory hierarchy; every file lands in the dest root.
    pub flatten: bool,
    /// Include only relative paths matching these globs.
    pub includes: Vec<String>,
    /// Skip relative paths matching these globs.
    pub excludes: Vec<String>,
    /// Hard speed limit in bytes/sec.
    pub speed_limit_bps: Option<u64>,
    /// Streaming chunk size in bytes.
    pub buffer: usize,
    /// Try reflink/clonefile fast path first.
    pub use_clone: bool,
    /// Try Linux `copy_file_range` fast path.
    pub use_cfr: bool,
    /// Preserve sparseness of source files.
    pub use_sparse: bool,
    /// Bypass the page cache (direct I/O) when the platform supports it.
    pub direct_io: bool,
    /// Plan only; touch nothing.
    pub dry_run: bool,
    /// Experimental: parallel range streams for network-scale files (0 = off).
    pub net_streams: u32,
    /// Continue an interrupted run recorded in the destination journal.
    pub resume: bool,
    /// Where to write the archive manifest (default: `<dst>/.ketra-manifest.json`).
    pub manifest_path: Option<String>,
    /// Retry transient I/O failures (EINTR/EBUSY/EIO, Windows sharing
    /// violations) this many times with a short backoff before failing.
    pub retries: u32,
    /// Copy this many files concurrently (1 = sequential, the classic
    /// behavior). Overlaps per-file fsync with the next file's transfer.
    pub parallel: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            verify: VerifyMode::Off,
            conflict: OnConflict::Ask,
            preserve_mtime: true,
            preserve_perms: true,
            flatten: false,
            includes: Vec::new(),
            excludes: Vec::new(),
            speed_limit_bps: None,
            buffer: crate::DEFAULT_CHUNK as usize,
            use_clone: true,
            use_cfr: true,
            use_sparse: true,
            direct_io: false,
            dry_run: false,
            net_streams: 0,
            resume: false,
            manifest_path: None,
            retries: 2,
            parallel: 1,
        }
    }
}

impl Options {
    /// Effective chunk size, clamped to 4 KiB..=16 MiB.
    pub fn chunk(&self) -> u64 {
        self.buffer.max(4096).min(16 * 1024 * 1024) as u64
    }

    /// Verify mode that implies hashing (for the manifest phase).
    pub fn wants_manifest(&self) -> bool {
        self.verify == VerifyMode::Manifest
    }
}

/// Default manifest algorithm name used in manifest files.
pub const MANIFEST_ALGO: &str = "sha256";

/// Hash anything implementing [`Hasher`] over a full file, in chunks.
/// Returns the lowercase hex digest. `cancel` aborts mid-file.
pub fn hash_file(
    path: &std::path::Path,
    algo: &str,
    chunk: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> crate::error::Result<String> {
    hash_file_with_progress(path, algo, chunk, cancel, &mut |_| {})
}

/// Like [`hash_file`] but reports every read through `progress`.
pub fn hash_file_with_progress(
    path: &std::path::Path,
    algo: &str,
    chunk: usize,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> crate::error::Result<String> {
    use std::io::Read;
    use std::sync::atomic::Ordering;

    let mut h = crate::hash::by_name(algo)
        .ok_or_else(|| crate::error::Error::Config(format!("unknown hash algorithm '{algo}'")))?;
    let mut f = std::fs::File::open(path).map_err(|e| crate::error::Error::ioctx(path, e))?;
    let mut buf = vec![0u8; chunk.clamp(64 * 1024, 4 * 1024 * 1024)];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(crate::error::Error::Canceled);
        }
        let n = f.read(&mut buf).map_err(|e| crate::error::Error::ioctx(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        progress(n as u64);
    }
    Ok(h.finish_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_defaults_are_sane() {
        let o = Options::default();
        assert_eq!(o.verify, VerifyMode::Off);
        assert_eq!(o.conflict, OnConflict::Ask);
        assert!(o.preserve_mtime && o.preserve_perms && o.use_clone);
        assert!(!o.direct_io && !o.dry_run);
        assert_eq!(o.chunk(), crate::DEFAULT_CHUNK);
    }

    #[test]
    fn test_chunk_clamping() {
        let mut o = Options::default();
        o.buffer = 10;
        assert_eq!(o.chunk(), 4096);
        o.buffer = 1 << 30;
        assert_eq!(o.chunk(), 16 * 1024 * 1024);
    }

    #[test]
    fn test_hash_file_matches_hasher() {
        let dir = std::env::temp_dir().join(format!("ketra-test-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("data.bin");
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 7) as u8).collect();
        std::fs::write(&p, &data).unwrap();

        let got = hash_file(&p, "sha256", 4096, &AtomicBool::new(false)).unwrap();
        let mut h = crate::hash::Sha256::new();
        h.update(&data);
        assert_eq!(got, Box::new(h).finish_hex());

        assert!(hash_file(&p, "bogus", 4096, &AtomicBool::new(false)).is_err());

        // canceled before first read
        let cancel = AtomicBool::new(true);
        assert!(hash_file(&p, "sha256", 4096, &cancel).unwrap_err().is_canceled());

        std::fs::remove_dir_all(&dir).ok();
    }
}
