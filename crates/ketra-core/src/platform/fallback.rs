//! Portable fallback for platforms without the special fast paths
//! (compiles anywhere; used by `cargo check` for exotic targets).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

pub fn free_space(_path: &Path) -> io::Result<u64> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "free_space unavailable"))
}

pub fn clone_file(_src: &Path, _dst: &Path) -> io::Result<super::CloneOutcome> {
    Ok(super::CloneOutcome::NotSupported)
}

pub fn copy_range(_src: &File, _dst: &File, _off: u64, _max: u64) -> io::Result<u64> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "copy_range unavailable"))
}

pub fn seek_region(_file: &File, _from: u64) -> Option<(u64, bool)> {
    None
}

pub fn next_hole(_file: &File, _from: u64, _end: u64) -> Option<(u64, u64)> {
    None
}

pub fn punch_hole(_file: &File, _off: u64, _len: u64) -> io::Result<()> {
    Ok(())
}

pub fn prealloc(_file: &File, _len: u64) -> io::Result<()> {
    Ok(())
}

pub fn set_sparse(_file: &File) -> io::Result<()> {
    Ok(())
}

pub fn bypass_cache(_file: &File) {}

pub fn read_advise(_file: &File, _off: u64, _len: u64) {}

pub fn open_direct(_path: &Path, _write: bool) -> Option<File> {
    None
}

pub fn long_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// Positional read via seek + read (portable fallback).
pub fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = f;
    let prev = f.stream_position()?;
    f.seek(SeekFrom::Start(off))?;
    let n = f.read(buf)?;
    f.seek(SeekFrom::Start(prev))?;
    Ok(n)
}

/// Positional write via seek + write (portable fallback).
pub fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<usize> {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = f;
    let prev = f.stream_position()?;
    f.seek(SeekFrom::Start(off))?;
    let n = f.write(buf)?;
    f.seek(SeekFrom::Start(prev))?;
    Ok(n)
}

/// Device id of the file volume (same-volume move detection).
pub fn st_dev(md: &std::fs::Metadata) -> u64 {
    let _ = md;
    0
}

pub fn st_mode(md: &std::fs::Metadata) -> u32 {
    if md.permissions().readonly() { 0o444 } else { 0o666 }
}
