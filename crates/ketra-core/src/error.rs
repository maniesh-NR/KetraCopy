//! Unified engine error type.

use std::fmt;
use std::io;
use std::path::Path;

/// Errors produced by the KetraCopy engine.
#[derive(Debug)]
pub enum Error {
    /// Filesystem error with the path that caused it.
    Io { path: String, source: io::Error },
    /// Cooperative cancellation was requested.
    Canceled,
    /// Inline verification found differing bytes.
    Verify { path: String, offset: u64 },
    /// The destination volume ran out of space.
    NoSpace { path: String },
    /// The source became shorter than planned while it was being read.
    /// The destination is left partial (journalled) for `--resume`.
    Shrank { path: String, copied: u64, planned: u64 },
    /// The current platform or filesystem cannot do what was asked.
    Unsupported(String),
    /// Settings or manifest files are malformed.
    Config(String),
    /// A destination already exists and no conflict policy resolved it.
    Conflict { path: String },
}

impl Error {
    /// Attach a path context to an `io::Error`.
    pub fn ioctx(path: impl AsRef<Path>, e: io::Error) -> Error {
        Error::Io { path: path.as_ref().display().to_string(), source: e }
    }

    /// True when the error is the cooperative-cancellation signal.
    pub fn is_canceled(&self) -> bool {
        matches!(self, Error::Canceled)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => {
                if path.is_empty() {
                    write!(f, "I/O error: {source}")
                } else {
                    write!(f, "{path}: {source}")
                }
            }
            Error::Canceled => write!(f, "canceled by user"),
            Error::Verify { path, offset } => {
                write!(f, "verification failed at byte {offset} of {path}")
            }
            Error::NoSpace { path } => write!(f, "no space left on destination volume ({path})"),
            Error::Shrank { path, copied, planned } => write!(
                f,
                "source shrank mid-copy: {copied} of {planned} bytes read of {path}; the partial destination was kept for --resume"
            ),
            Error::Unsupported(m) => write!(f, "unsupported operation: {m}"),
            Error::Config(m) => write!(f, "configuration error: {m}"),
            Error::Conflict { path } => {
                write!(f, "destination already exists: {path}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io { path: String::new(), source: e }
    }
}

/// Result alias used across the engine.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_with_path() {
        let e = Error::ioctx("/tmp/x", io::Error::from(io::ErrorKind::NotFound));
        assert!(e.to_string().contains("/tmp/x"));
        assert!(e.to_string().contains("entity not found") || e.to_string().contains("NotFound"));
    }

    #[test]
    fn test_canceled_detection() {
        assert!(Error::Canceled.is_canceled());
        assert!(!Error::Unsupported("x".into()).is_canceled());
    }

    #[test]
    fn test_from_io_error_has_empty_path() {
        let e: Error = io::Error::new(io::ErrorKind::Other, "boom").into();
        assert!(matches!(e, Error::Io { ref path, .. } if path.is_empty()));
    }
}
