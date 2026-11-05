//! macOS implementation: `clonefile(2)` reflinks, `fcntl(F_NOCACHE)`,
//! `fcntl(F_RDADVISE)`, `fcntl(F_PREALLOCATE)`, `statvfs` free space.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

extern "C" {
    fn clonefile(src: *const c_char, dst: *const c_char, flags: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
}

type c_int = std::os::raw::c_int;
type c_char = std::os::raw::c_char;

const F_NOCACHE: c_int = 48;
const F_RDADVISE: c_int = 51;
const F_PREALLOCATE: c_int = 42;
const F_ALLOCATECONTIG: c_int = 0x00000002;
const F_ALLOCATEALL: c_int = 0x00000004;
const F_PUNCHHOLE: c_int = 35; // available on APFS; ignored when unsupported

/// Darwin `struct radvisory`.
#[repr(C)]
struct Radvisory {
    ra_offset: i64,
    ra_count: c_int,
    _pad: [u8; 4], // alignment to 16 bytes like the C struct
}

/// Darwin `struct fstore` for F_PREALLOCATE.
#[repr(C)]
struct FStore {
    fst_flags: c_int,
    fst_posmode: c_int,
    fst_offset: i64,
    fst_length: i64,
    fst_bytesalloc: i64,
}

/// Darwin `struct statvfs` (64-bit layout).
#[repr(C)]
struct StatVfs {
    f_bsize: c_ulong,
    f_frsize: c_ulong,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: c_ulong,
    f_flag: c_ulong,
    f_namemax: c_ulong,
}

type c_ulong = std::os::raw::c_ulong;

fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

/// Free bytes available to unprivileged users on the volume of `path`.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let c = cstr(path)?;
    let mut st: StatVfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st.f_bavail.wrapping_mul(st.f_frsize))
}

/// Try `clonefile(2)` (APFS reflink); `dst` must not exist.
pub fn clone_file(src: &Path, dst: &Path) -> io::Result<super::CloneOutcome> {
    let s = cstr(src)?;
    let d = cstr(dst)?;
    let rc = unsafe { clonefile(s.as_ptr(), d.as_ptr(), 0) };
    if rc == 0 {
        Ok(super::CloneOutcome::Cloned)
    } else {
        let err = io::Error::last_os_error();
        let _ = std::fs::remove_file(dst);
        match err.raw_os_error() {
            Some(45) /* ENOTSUP */ | Some(92) /* EOPNOTSUPP */ | Some(22) /* EINVAL */
            | Some(1) /* EPERM */ | Some(18) /* EXDEV */ => Ok(super::CloneOutcome::NotSupported),
            _ => Err(err),
        }
    }
}

/// macOS has no `copy_file_range` reachable without libc crates; the
/// pipeline falls back to its own streaming loop.
pub fn copy_range(_src: &File, _dst: &File, _off: u64, _max: u64) -> io::Result<u64> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "copy_file_range unavailable on macOS"))
}

/// No SEEK_DATA/SEEK_HOLE on Darwin — sparse scans unsupported.
pub fn seek_region(_file: &File, _from: u64) -> Option<(u64, bool)> {
    None
}

/// Hole detection unavailable on Darwin; the pipeline falls back to
/// runtime zero-run skipping.
pub fn next_hole(_file: &File, _from: u64, _end: u64) -> Option<(u64, u64)> {
    None
}

/// Punch a hole on APFS via `fcntl(F_PUNCHHOLE)` when available.
pub fn punch_hole(file: &File, off: u64, len: u64) -> io::Result<()> {
    #[repr(C)]
    struct PunchHole {
        pf_offset: i64,
        pf_length: i64,
    }
    let ph = PunchHole { pf_offset: off as i64, pf_length: len as i64 };
    let rc = unsafe { fcntl(file.as_raw_fd(), F_PUNCHHOLE, &ph as *const PunchHole) };
    if rc == 0 {
        Ok(())
    } else {
        let _ = io::Error::last_os_error();
        Ok(()) // unsupported filesystem: keep the zeros dense
    }
}

/// `fcntl(F_PREALLOCATE)` contiguous-or-all.
pub fn prealloc(file: &File, len: u64) -> io::Result<()> {
    let mut fs = FStore {
        fst_flags: F_ALLOCATECONTIG | F_ALLOCATEALL,
        fst_posmode: 0,
        fst_offset: 0,
        fst_length: len as i64,
        fst_bytesalloc: 0,
    };
    let rc = unsafe { fcntl(file.as_raw_fd(), F_PREALLOCATE, &mut fs as *mut FStore) };
    if rc == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(45) | Some(22) | Some(25) => Ok(()), // ENOTSUP/EINVAL/ENOTTY: best effort
        _ => Err(e),
    }
}

/// Windows parity no-op.
pub fn set_sparse(_file: &File) -> io::Result<()> {
    Ok(())
}

/// `fcntl(F_NOCACHE)` — the macOS equivalent of O_DIRECT.
pub fn bypass_cache(file: &File) {
    unsafe {
        fcntl(file.as_raw_fd(), F_NOCACHE, 1);
    }
}

/// `fcntl(F_RDADVISE)` for `[off, off+len)`.
pub fn read_advise(file: &File, off: u64, len: u64) {
    let ra = Radvisory { ra_offset: off as i64, ra_count: len.min(i32::MAX as u64) as c_int, _pad: [0; 4] };
    unsafe {
        fcntl(file.as_raw_fd(), F_RDADVISE, &ra as *const Radvisory);
    }
}

/// Direct I/O is emulated by opening normally then enabling F_NOCACHE.
pub fn open_direct(path: &Path, write: bool) -> Option<File> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(write)
        .create(write)
        .truncate(false)
        .open(path)
        .ok()?;
    bypass_cache(&f);
    Some(f)
}

/// Darwin paths have no extended-length prefix.
pub fn long_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// Positional read (pread semantics; works through `FileExt::read_at`).
pub fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    f.read_at(buf, off)
}

/// Positional write (pwrite semantics; works through `FileExt::write_at`).
pub fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    f.write_at(buf, off)
}

/// Device id of the file volume (same-volume move detection).
pub fn st_dev(md: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.dev()
}

/// Unix mode bits straight from stat.
pub fn st_mode(md: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    md.mode()
}
