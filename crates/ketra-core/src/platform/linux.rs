//! Linux implementation: reflink ioctl, `copy_file_range`, sparse scans,
//! `fallocate`, `statvfs`. All reached via hand-declared libc FFI.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

// --- FFI declarations (glibc/musl, x86_64 & aarch64 layouts) ---

extern "C" {
    fn copy_file_range(
        fd_in: c_int,
        off_in: *mut i64,
        fd_out: c_int,
        off_out: *mut i64,
        len: usize,
        flags: c_uint,
    ) -> isize;
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn fallocate(fd: c_int, mode: c_int, offset: i64, len: i64) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
    fn posix_fadvise(fd: c_int, offset: i64, len: i64, advice: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
}

type c_int = std::os::raw::c_int;
type c_uint = std::os::raw::c_uint;
type c_ulong = std::os::raw::c_ulong;
type c_char = std::os::raw::c_char;

const SEEK_DATA: c_int = 3;
const SEEK_HOLE: c_int = 4;
const FICLONE: c_ulong = 0x40049409; // _IOW('f', 0, int)
const FALLOC_FL_KEEP_SIZE: c_int = 0x01;
const FALLOC_FL_PUNCH_HOLE: c_int = 0x02;
const POSIX_FADV_WILLNEED: c_int = 3;
const POSIX_FADV_DONTNEED: c_int = 4;
const POSIX_FADV_SEQUENTIAL: c_int = 2;
const O_DIRECT: i32 = 0o40000;

/// glibc `struct statvfs` (x86_64/aarch64 linux-gnu layout).
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
    __spare: [c_int; 6],
}

fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

/// Free bytes available to unprivileged users on the volume of `path`.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let c = cstr(&long_path(path))?;
    let mut st: StatVfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st.f_bavail.wrapping_mul(st.f_frsize))
}

/// Try `FICLONE` reflink; `dst` must not exist.
pub fn clone_file(src: &Path, dst: &Path) -> io::Result<super::CloneOutcome> {
    let s = File::open(src)?;
    let d = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    let rc = unsafe { ioctl(d.as_raw_fd(), FICLONE, s.as_raw_fd() as c_int) };
    if rc == 0 {
        Ok(super::CloneOutcome::Cloned)
    } else {
        let err = io::Error::last_os_error();
        drop(d);
        let _ = std::fs::remove_file(dst); // clean the empty target up
        if matches!(
            err.raw_os_error(),
            Some(22) /* EINVAL */
                | Some(95) /* EOPNOTSUPP */
                | Some(1) /* EPERM */
                | Some(18) /* EXDEV: cross-device — FICLONE cannot span
                              filesystems; fall back to a regular copy
                              (mirrors the macOS mapping) */
                | Some(38) /* ENOSYS */
        ) {
            Ok(super::CloneOutcome::NotSupported)
        } else {
            Err(err)
        }
    }
}

/// `copy_file_range` one region; returns bytes copied (0 = EOF).
/// `EINTR` is retried in place with the *same* offsets.
pub fn copy_range(src: &File, dst: &File, off: u64, max: u64) -> io::Result<u64> {
    let mut off_in = off as i64;
    let mut off_out = off as i64;
    loop {
        let n = unsafe {
            copy_file_range(
                src.as_raw_fd(),
                &mut off_in,
                dst.as_raw_fd(),
                &mut off_out,
                max as usize,
                0,
            )
        };
        if n >= 0 {
            return Ok(n as u64);
        }
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            continue; // retry same region, same offsets
        }
        return Err(e);
    }
}

/// Seek to the next region boundary: `Some((offset, is_data))`, `None` if
/// the filesystem does not support sparse scans (legacy EINVAL / ENOTSUP).
pub fn seek_region(file: &File, from: u64) -> Option<(u64, bool)> {
    let fd = file.as_raw_fd();
    unsafe {
        let d = lseek(fd, from as i64, SEEK_DATA);
        if d < 0 {
            // EOF inside a hole: data ends at file end.
            let e = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if e == 6 {
                /* ENXIO: from is inside the final hole */
                return Some((from, false));
            }
            return None;
        }
        let h = lseek(fd, d, SEEK_HOLE);
        if h < 0 {
            return None;
        }
        Some((d as u64, true))
    }
}

/// Find the next hole (offset of hole start) at or after `from`, bounded by
/// the next data region (not EOF — files may have several data/hole extents).
pub fn next_hole(file: &File, from: u64, end: u64) -> Option<(u64, u64)> {
    let fd = file.as_raw_fd();
    unsafe {
        let d = lseek(fd, from as i64, SEEK_DATA);
        let h = if d < 0 {
            // Everything from `from` to `end` is a hole.
            if from < end {
                return Some((from, end - from));
            } else {
                return None;
            }
        } else {
            lseek(fd, d, SEEK_HOLE)
        };
        if h < 0 || h as u64 >= end {
            return None;
        }
        let hstart = h as u64;
        if std::env::var_os("KETRA_DEBUG_NH").is_some() {
            eprintln!("debug next_hole: from={from} d={d} h={h} end={end}");
        }
        // The hole may end before `end`: find the next data region after it.
        let dnext = lseek(fd, h, SEEK_DATA);
        let hlen = if dnext < 0 {
            // ENXIO: no more data after this hole — it runs to `end`.
            end - hstart
        } else if (dnext as u64) >= end {
            end - hstart
        } else {
            dnext as u64 - hstart
        };
        if hlen == 0 {
            return None;
        }
        Some((hstart, hlen))
    }
}

/// Preallocate `len` bytes without changing the file size
/// (`fallocate` KEEP_SIZE); falls back to full `fallocate` on `len==size`.
pub fn prealloc(file: &File, len: u64) -> io::Result<()> {
    let rc = unsafe { fallocate(file.as_raw_fd(), 0, 0, len as i64) };
    if rc == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(95) | Some(22) | Some(38) => Ok(()), // EOPNOTSUPP/EINVAL/ENOSYS: best effort
        _ => Err(e),
    }
}

/// Punch a hole in `[off, off+len)` keeping the size.
pub fn punch_hole(file: &File, off: u64, len: u64) -> io::Result<()> {
    let rc = unsafe {
        fallocate(
            file.as_raw_fd(),
            FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
            off as i64,
            len as i64,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(95) | Some(22) | Some(38) => Ok(()), // unsupported: keep dense
            _ => Err(e),
        }
    }
}

/// Windows parity no-op: Linux files are sparse-capable by default on
/// modern filesystems.
pub fn set_sparse(_file: &File) -> io::Result<()> {
    Ok(())
}

/// `posix_fadvise(SEQUENTIAL | DONTNEED)` to keep the page cache clean.
pub fn bypass_cache(file: &File) {
    unsafe {
        posix_fadvise(file.as_raw_fd(), 0, 0, POSIX_FADV_SEQUENTIAL);
    }
}

/// `posix_fadvise(WILLNEED)` for `[off, off+len)`.
pub fn read_advise(file: &File, off: u64, len: u64) {
    unsafe {
        posix_fadvise(file.as_raw_fd(), off as i64, len as i64, POSIX_FADV_WILLNEED);
    }
}

/// Open with `O_DIRECT` (block-aligned buffers required by the caller).
pub fn open_direct(path: &Path, write: bool) -> Option<File> {
    let mut oo = std::fs::OpenOptions::new();
    oo.read(true).custom_flags(O_DIRECT);
    if write {
        oo.write(true).create(true).truncate(false);
    }
    oo.open(long_path(path)).ok()
}

/// `fcntl(F_SETFL, O_DIRECT)` on an existing descriptor.
pub fn set_direct(fd: RawFd) {
    unsafe {
        let fl = fcntl(fd, 4 /* F_GETFL */);
        if fl >= 0 {
            fcntl(fd, 4 /* F_SETFL */, fl | O_DIRECT);
        }
    }
}

/// Linux paths are already long-safe (no `\\?\` concept).
pub fn long_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn test_statvfs_free_space() {
        let free = free_space(&std::env::temp_dir()).unwrap();
        assert!(free > 0);
    }

    #[test]
    fn test_sparse_scan_on_sparse_file() {
        let dir = std::env::temp_dir().join(format!("ketra-linux-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("sparse.bin");
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&p)
            .unwrap();
        // 1 MiB hole, 64 KiB data, 1 MiB hole.
        f.seek(std::io::SeekFrom::Start(1024 * 1024)).unwrap();
        f.write_all(&vec![0xABu8; 64 * 1024]).unwrap();
        f.set_len(2 * 1024 * 1024 + 64 * 1024).unwrap();
        f.sync_all().unwrap();

        let region = seek_region(&f, 0).expect("SEEK_DATA supported");
        assert_eq!(region, (1024 * 1024, true));

        let allocated = std::fs::metadata(&p).unwrap().blocks() * 512;
        // Data + overhead must be far below the logical size.
        assert!(allocated < 2 * 1024 * 1024, "allocated={allocated}");

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod hole_tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    extern "C" {
        fn lseek(fd: i32, off: i64, whence: i32) -> i64;
    }
    const SEEK_DATA: i32 = 3;
    const SEEK_HOLE: i32 = 4;

    #[test]
    fn test_next_hole_multi_extents() {
        let dir = std::env::temp_dir().join(format!("ketra-holes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.bin");
        let f = File::create(&p).unwrap();
        f.set_len(32 * 1024 * 1024).unwrap();
        use std::io::{Seek, SeekFrom, Write};
        let mut f = f;
        f.seek(SeekFrom::Start(8 * 1024 * 1024)).unwrap();
        f.write_all(b"A".repeat(4096).as_slice()).unwrap();
        f.seek(SeekFrom::Start(24 * 1024 * 1024)).unwrap();
        f.write_all(b"B".repeat(4096).as_slice()).unwrap();

        let end = 32 * 1024 * 1024u64;
        // hole between first and second data extents
        let h = next_hole(&f, 8 * 1024 * 1024, end).expect("hole after first data extent");
        assert_eq!(h.0, 8 * 1024 * 1024 + 4096, "hole starts after the A block");
        assert!(h.1 <= 16 * 1024 * 1024, "hole must end before the B block, got len {}", h.1);
        // hole after the second data extent runs to EOF
        let h2 = next_hole(&f, 24 * 1024 * 1024, end).expect("trailing hole");
        assert_eq!(h2.0, 24 * 1024 * 1024 + 4096);
        assert_eq!(h2.0 + h2.1, end);
        std::fs::remove_dir_all(&dir).ok();
    }
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
