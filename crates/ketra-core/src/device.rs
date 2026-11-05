//! Volume / device queries: free space, same-device detection.

use crate::error::{Error, Result};
use std::path::Path;

/// Free bytes available to unprivileged users on the volume of `path`.
pub fn free_space(path: &Path) -> Result<u64> {
    platform_free_space(path)
}

/// Device id (st_dev) of the volume containing `path`.
pub fn device_id(path: &Path) -> Result<u64> {
    let md = std::fs::metadata(path).map_err(|e| Error::ioctx(path, e))?;
    Ok(crate::platform::st_dev(&md))
}

/// True when both paths live on the same filesystem (enables the rename
/// shortcut for moves).
pub fn same_device(a: &Path, b: &Path) -> bool {
    match (device_id(a), device_id(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(unix)]
fn platform_free_space(path: &Path) -> Result<u64> {
    crate::platform::free_space(path).map_err(Error::from)
}

#[cfg(not(unix))]
fn platform_free_space(_path: &Path) -> Result<u64> {
    Err(Error::Unsupported("free_space on this platform".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_space_on_temp_dir() {
        let free = free_space(&std::env::temp_dir()).expect("statvfs on tmp");
        // A healthy tmp has more than 1 MiB free.
        assert!(free > 1024 * 1024, "free={free}");
    }

    #[test]
    fn test_same_device_trivially_true() {
        let a = std::env::temp_dir();
        let b = std::env::temp_dir().join("ketra-same-device-probe");
        std::fs::create_dir_all(&b).unwrap();
        assert!(same_device(&a, &b));
        std::fs::remove_dir(&b).ok();
    }
}
