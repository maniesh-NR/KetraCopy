//! Platform abstraction: fast paths and volume queries, reached through
//! hand-declared FFI (the engine links **zero external crates**).
//!
//! - Linux: `FICLONE` reflink ioctl, `copy_file_range(2)`,
//!   `lseek(SEEK_DATA/SEEK_HOLE)` sparse scanning, `fallocate(2)`
//!   preallocation / hole punching, `statvfs(3)` free space.
//! - macOS: `clonefile(2)`, `fcntl(F_NOCACHE)`, `fcntl(F_RDADVISE)`,
//!   `fcntl(F_PREALLOCATE)`.
//! - Windows: `CreateFileW` with `FILE_FLAG_NO_BUFFERING`,
//!   `\\?\` long-path conversion, `FSCTL_SET_SPARSE` /
//!   `FSCTL_SET_ZERO_DATA`, `SetFileInformationByHandle`.
//!
//! Only the module matching the build target is compiled; the others are
//! still type-checked in CI via `cargo check --target`.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux as imp;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos as imp;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub mod fallback;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback as imp;

// Re-exported portable surface used by the pipeline.

/// Result of a reflink attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneOutcome {
    /// Destination is a reflink of the source; the copy is complete.
    Cloned,
    /// Fast path unavailable — fall back to a streaming copy.
    NotSupported,
}

pub use imp::{copy_range, free_space, next_hole, prealloc, punch_hole, read_at, set_sparse, st_dev, st_mode, write_at};

/// Attempt a reflink clone `src → dst`. `dst` must not exist.
pub fn clone_file(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<CloneOutcome> {
    imp::clone_file(src, dst)
}

/// Seek to the next data/hole region (sparse scan). Returns `(offset, is_data)`.
/// `None` when the platform cannot scan.
pub fn seek_region(file: &std::fs::File, from: u64) -> Option<(u64, bool)> {
    imp::seek_region(file, from)
}

/// Enable cache bypass hints on an open descriptor (F_NOCACHE / posix_fadvise).
pub fn bypass_cache(file: &std::fs::File) {
    imp::bypass_cache(file)
}

/// Advise the kernel that `[off, off+len)` will be read soon (F_RDADVISE /
/// posix_fadvise WILLNEED).
pub fn read_advise(file: &std::fs::File, off: u64, len: u64) {
    imp::read_advise(file, off, len)
}

/// Open `path` with the platform direct-I/O flags (returns `None` when
/// unsupported — callers fall back to buffered I/O).
pub fn open_direct(path: &std::path::Path, write: bool) -> Option<std::fs::File> {
    imp::open_direct(path, write)
}

/// Convert to an extended-length path on Windows (`\\?\`); identity elsewhere.
pub fn long_path(p: &std::path::Path) -> std::path::PathBuf {
    imp::long_path(p)
}
