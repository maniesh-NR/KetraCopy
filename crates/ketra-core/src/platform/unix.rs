//! POSIX shared layer (Linux + macOS + other unixes): open with O_DIRECT,
//! sequential advice, preallocation, fsync of files and directories, and
//! same-volume detection via `st_dev`.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

// open(2) flag not exposed by std::fs::OpenOptions.
#[cfg(target_os = "linux")]
const O_DIRECT: i32 = 0x4000;
#[cfg(not(target_os = "linux"))]
const O_DIRECT: i32 = 0; // macOS has no O_DIRECT (F_NOCACHE used instead)

extern "C" {
    fn fsync(fd: RawFd) -> i32;
    fn fallocate(fd: RawFd, mode: i32, offset: i64, len: i64) -> i32;
}

/// Open through `std` with extra raw flags so we can pass O_DIRECT.
pub fn open_file(path: &Path, write: bool, create: bool, truncate: bool, direct: bool) -> io::Result<File> {
    let mut oo = std::fs::OpenOptions::new();
    if write {
        oo.write(true);
    } else {
        oo.read(true);
    }
    if create {
        oo.create(true);
    }
    if truncate {
        oo.truncate(true);
    }
    if direct && write {
        oo.custom_flags(O_DIRECT);
    }
    oo.mode(0o644);
    oo.open(path)
}

#[cfg(target_os = "linux")]
pub fn advise_sequential(f: &File, off: u64, len: u64) {
    const POSIX_FADV_SEQUENTIAL: i32 = 2;
    // SAFETY: fd is valid; advice is a hint.
    unsafe {
        posix_fadvise(f.as_raw_fd(), off as i64, len as i64, POSIX_FADV_SEQUENTIAL);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn advise_sequential(_f: &File, _off: u64, _len: u64) {}

#[cfg(target_os = "linux")]
extern "C" {
    fn posix_fadvise(fd: RawFd, offset: i64, len: i64, advice: i32) -> i32;
}

pub fn fsync_file(f: &File) -> io::Result<()> {
    // SAFETY: fd is valid.
    let rc = unsafe { fsync(f.as_raw_fd()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// fsync a directory so create/rename metadata is durable.
pub fn fsync_dir(path: &Path) {
    // Opening a directory read-only is portable on unix.
    if let Ok(f) = std::fs::File::open(path) {
        // SAFETY: fd is valid.
        unsafe {
            fsync(f.as_raw_fd());
        }
    }
}

/// Preallocate disk blocks (fallocate mode 0); fall back silently —
/// callers treat this as a hint only.
pub fn preallocate(f: &File, size: u64) {
    // SAFETY: fd is valid.
    unsafe {
        fallocate(f.as_raw_fd(), 0, 0, size as i64);
    }
}

/// Same volume? std exposes st_dev on unix — no FFI needed.
pub fn same_filesystem(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev(),
        _ => false,
    }
}
