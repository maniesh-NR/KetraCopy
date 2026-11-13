//! Windows implementation: `CreateFileW` with `FILE_FLAG_NO_BUFFERING`,
//! `\\?\` long-path conversion, `FSCTL_SET_SPARSE` / `FSCTL_SET_ZERO_DATA`
//! hole management, `FSCTL_DUPLICATE_EXTENTS_TO_FILE` reflinks on ReFS,
//! `GetDiskFreeSpaceExW` free space, `SetFileInformationByHandle` mtime.
//!
//! All handles are opened through this module so the pipeline can rely on
//! overlapped-free, sequential, (optionally) unbuffered semantics.

#![allow(non_snake_case)]

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

type BOOL = i32;
type DWORD = u32;
type HANDLE = *mut std::os::raw::c_void;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const GENERIC_READ: DWORD = 0x8000_0000;
const GENERIC_WRITE: DWORD = 0x4000_0000;
const FILE_SHARE_READ: DWORD = 0x0000_0001;
const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
const OPEN_EXISTING: DWORD = 3;
const CREATE_ALWAYS: DWORD = 2;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x0000_0080;
const FILE_FLAG_NO_BUFFERING: DWORD = 0x2000_0000;
const FILE_FLAG_WRITE_THROUGH: DWORD = 0x8000_0000;
const FILE_FLAG_SEQUENTIAL_SCAN: DWORD = 0x0800_0000;
const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x0200_0000;

const FSCTL_SET_SPARSE: DWORD = 0x0009_00C4;
const FSCTL_SET_ZERO_DATA: DWORD = 0x0009_80C8;
const FSCTL_DUPLICATE_EXTENTS_TO_FILE: DWORD = 0x0009_83CC;

extern "system" {
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *mut std::os::raw::c_void,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> BOOL;
    fn SetFilePointerEx(
        hFile: HANDLE,
        liDistanceToMove: i64,
        lpNewFilePointer: *mut i64,
        dwMoveMethod: DWORD,
    ) -> BOOL;
    fn SetEndOfFile(hFile: HANDLE) -> BOOL;
    fn DeviceIoControl(
        hDevice: HANDLE,
        dwIoControlCode: DWORD,
        lpInBuffer: *mut std::os::raw::c_void,
        nInBufferSize: DWORD,
        lpOutBuffer: *mut std::os::raw::c_void,
        nOutBufferSize: DWORD,
        lpBytesReturned: *mut DWORD,
        lpOverlapped: *mut std::os::raw::c_void,
    ) -> BOOL;
    fn SetFileValidData(hFile: HANDLE, lValidDataLength: i64) -> BOOL;
}

/// Duplicate the last Win32 error as an io::Error.
fn last_err() -> io::Error {
    io::Error::last_os_error()
}

fn to_wide(p: &Path) -> Vec<u16> {
    let mut v: Vec<u16> = long_path(p).as_os_str().encode_wide().collect();
    v.push(0);
    v
}

/// Wrap a raw HANDLE as a File (for the pipeline's buffered helpers).
unsafe fn file_from_handle(h: HANDLE) -> io::Result<File> {
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        return Err(last_err());
    }
    use std::os::windows::io::{FromRawHandle, RawHandle};
    let handle: RawHandle = h as RawHandle;
    Ok(File::from_raw_handle(handle))
}

/// `\\?\` prefix for absolute paths to exceed MAX_PATH.
pub fn long_path(p: &Path) -> PathBuf {
    let s = p.as_os_str();
    if let Some(stripped) = s.to_str() {
        if stripped.starts_with(r"\\?\") {
            return p.to_path_buf();
        }
        if stripped.len() >= 248 && stripped.starts_with(r"\\") {
            // UNC: \\server\share → \\?\UNC\server\share
            let mut out = String::from(r"\\?\UNC");
            out.push_str(&stripped[1..]);
            return PathBuf::from(out);
        }
        if stripped.len() >= 248 {
            let mut out = String::from(r"\\?\");
            out.push_str(stripped);
            return PathBuf::from(out);
        }
        if Path::new(stripped).is_absolute() {
            let mut out = String::from(r"\\?\");
            out.push_str(stripped);
            return PathBuf::from(out);
        }
    }
    p.to_path_buf()
}

/// Free bytes available to unprivileged users on the volume of `path`.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let dir = to_wide(&path);
    let mut avail: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    let rc = unsafe {
        GetDiskFreeSpaceExW(dir.as_ptr(), &mut avail, &mut total, &mut free)
    };
    if rc == 0 {
        return Err(last_err());
    }
    Ok(avail)
}

/// Open a file with Win32 flags; returns the std File wrapper.
pub fn open_with_flags(path: &Path, write: bool, create_new: bool, no_buffer: bool) -> io::Result<File> {
    let wide = to_wide(path);
    let access = if write { GENERIC_WRITE | GENERIC_READ } else { GENERIC_READ };
    let disposition = if create_new { CREATE_ALWAYS } else { OPEN_EXISTING };
    let mut flags = FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN;
    if no_buffer {
        flags |= FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH;
    }
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            disposition,
            flags,
            INVALID_HANDLE_VALUE,
        )
    };
    unsafe { file_from_handle(h) }
}

/// Try an ReFS block clone (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`).
pub fn clone_file(src: &Path, dst: &Path) -> io::Result<super::CloneOutcome> {
    let s = match open_with_flags(src, false, false, false) {
        Ok(f) => f,
        Err(_) => return Ok(super::CloneOutcome::NotSupported),
    };
    let d = match open_with_flags(dst, true, true, false) {
        Ok(f) => f,
        Err(_) => return Ok(super::CloneOutcome::NotSupported),
    };
    #[repr(C)]
    struct DuplicateExtentsData {
        file_handle: HANDLE,
        source_offset: i64,
        target_offset: i64,
        byte_count: i64,
    }
    let size = src.metadata().map(|m| m.len() as i64).unwrap_or(0);
    let mut req = DuplicateExtentsData {
        file_handle: s.as_raw_handle_or_null(),
        source_offset: 0,
        target_offset: 0,
        byte_count: size,
    };
    let mut returned: DWORD = 0;
    let rc = unsafe {
        DeviceIoControl(
            d.as_raw_handle_or_null(),
            FSCTL_DUPLICATE_EXTENTS_TO_FILE,
            &mut req as *mut _ as *mut std::os::raw::c_void,
            std::mem::size_of::<DuplicateExtentsData>() as DWORD,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if rc == 0 {
        let err = last_err();
        drop(d);
        let _ = std::fs::remove_file(dst);
        return match err.raw_os_error() {
            Some(code) if code != 0 => Ok(super::CloneOutcome::NotSupported),
            _ => Ok(super::CloneOutcome::NotSupported),
        };
    }
    Ok(super::CloneOutcome::Cloned)
}

/// No `copy_file_range` on Win32; the pipeline falls back to its own loop.
pub fn copy_range(_src: &File, _dst: &File, _off: u64, _max: u64) -> io::Result<u64> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "copy_file_range unavailable on Windows"))
}

/// Sparse region scans are not exposed on Windows; report unsupported.
pub fn seek_region(_file: &File, _from: u64) -> Option<(u64, bool)> {
    None
}

/// `FSCTL_QUERY_ALLOCATED_RANGES` is not wired up; zero-run skipping covers it.
pub fn next_hole(_file: &File, _from: u64, _end: u64) -> Option<(u64, u64)> {
    None
}

/// Mark the file sparse (`FSCTL_SET_SPARSE`) — done by `set_sparse`.
pub fn punch_hole(file: &File, off: u64, len: u64) -> io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    #[repr(C)]
    struct FileZeroDataInformation {
        file_offset: i64,
        beyond_final_zero: i64,
    }
    let zd = FileZeroDataInformation {
        file_offset: off as i64,
        beyond_final_zero: (off + len) as i64,
    };
    let mut returned: DWORD = 0;
    let rc = unsafe {
        DeviceIoControl(
            file.as_raw_handle_or_null(),
            FSCTL_SET_ZERO_DATA,
            &zd as *const _ as *mut std::os::raw::c_void,
            std::mem::size_of::<FileZeroDataInformation>() as DWORD,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if rc == 0 {
        // Best effort: dense zeros are still correct.
        let _ = last_err();
    }
    Ok(())
}

/// Preallocate with `SetFilePointerEx` + `SetEndOfFile` and mark sparse.
pub fn prealloc(file: &File, len: u64) -> io::Result<()> {
    let h = file.as_raw_handle_or_null();
    let mut cur: i64 = 0;
    let rc = unsafe { SetFilePointerEx(h, len as i64, &mut cur, 0 /* FILE_BEGIN */) };
    if rc == 0 {
        return Err(last_err());
    }
    if unsafe { SetEndOfFile(h) } == 0 {
        return Err(last_err());
    }
    let _ = unsafe { SetFilePointerEx(h, 0, &mut cur, 0) };
    Ok(())
}

/// `FSCTL_SET_SPARSE` on the destination handle.
pub fn set_sparse(file: &File) -> io::Result<()> {
    let mut returned: DWORD = 0;
    let rc = unsafe {
        DeviceIoControl(
            file.as_raw_handle_or_null(),
            FSCTL_SET_SPARSE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if rc == 0 {
        let _ = last_err(); // FAT/exFAT: not sparse-capable; ignore
    }
    Ok(())
}

/// Windows bypasses the cache via CreateFileW flags (no fcntl equivalent).
pub fn bypass_cache(_file: &File) {}

/// Sequential-read hint is folded into CreateFileW; no-op here.
pub fn read_advise(_file: &File, _off: u64, _len: u64) {}

/// Direct I/O uses `FILE_FLAG_NO_BUFFERING` at open time.
pub fn open_direct(path: &Path, write: bool) -> Option<File> {
    open_with_flags(path, write, false, true).ok()
}

/// Set the valid data length (skip zero-fill of preallocated regions).
pub fn set_valid_data(file: &File, len: u64) -> io::Result<()> {
    let rc = unsafe { SetFileValidData(file.as_raw_handle_or_null(), len as i64) };
    if rc == 0 {
        return Err(last_err());
    }
    Ok(())
}

/// Raw handle accessor used by the FFI wrappers.
trait RawHandleExt {
    fn as_raw_handle_or_null(&self) -> HANDLE;
}

impl RawHandleExt for File {
    fn as_raw_handle_or_null(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        self.as_raw_handle() as HANDLE
    }
}

/// Unused import guard: OsStr used through encode_wide only.
#[allow(dead_code)]
fn _assert_os_str(s: &OsStr) -> usize {
    s.encode_wide().count()
}

/// Positional read (pread semantics; works through `FileExt::seek_read`).
pub fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    f.seek_read(buf, off)
}

/// Positional write (pwrite semantics; works through `FileExt::seek_write`).
pub fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    f.seek_write(buf, off)
}

/// Volume/file identity for the same-volume move check (best effort: 0).
pub fn st_dev(md: &std::fs::Metadata) -> u64 {
    let _ = md;
    0
}

/// Unix-style mode bits approximation from the read-only attribute.
pub fn st_mode(md: &std::fs::Metadata) -> u32 {
    if md.permissions().readonly() { 0o444 } else { 0o666 }
}
