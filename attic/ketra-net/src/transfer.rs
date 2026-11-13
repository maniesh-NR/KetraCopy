//! High-level transfer orchestration shared by the receive server and the
//! send client. The `ProgressSink` trait is how callers (CLI, Android UI)
//! receive streaming progress events; `TransferJob` drives a session and
//! produces a `TransferOutcome`.

use std::sync::Arc;

use crate::protocol::FileMetadata;

/// Streaming progress events. Mirrors `ketra_core::progress::Event` but
/// adds LAN-specific state (session id, peer, per-file chunk verify).
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Session established with a peer (their alias + session id).
    Started { peer_alias: String, session_id: String, file_count: usize, total_bytes: u64 },
    /// About to send/receive file `i` of `n`.
    FileStart { index: usize, total: usize, meta: FileMetadata },
    /// Bytes transferred for the current file.
    FileProgress { index: usize, bytes_done: u64, bytes_total: u64 },
    /// Current file finished; `verified` is true when inline verify passed.
    FileDone { index: usize, verified: bool, sha256: Option<String> },
    /// Whole session finished.
    Completed { ok: usize, failed: usize, bytes: u64, manifest_exchanged: bool },
    /// A failure aborted the session.
    Failed { message: String },
}

/// Sink for progress events. Implementations: CLI prints to stderr (plain or
/// JSON), Android bridges to a Kotlin callback via UniFFI.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, ev: ProgressEvent);
}

/// A no-op sink for tests / quiet runs.
pub struct NullSink;
impl ProgressSink for NullSink {
    fn emit(&self, _ev: ProgressEvent) {}
}

/// A sink that fans out to multiple sinks.
pub struct FanoutSink {
    sinks: Vec<Arc<dyn ProgressSink>>,
}
impl FanoutSink {
    pub fn new(sinks: Vec<Arc<dyn ProgressSink>>) -> Self {
        Self { sinks }
    }
}
impl ProgressSink for FanoutSink {
    fn emit(&self, ev: ProgressEvent) {
        for s in &self.sinks {
            s.emit(ev.clone());
        }
    }
}

/// Final outcome of a transfer session.
#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub ok: usize,
    pub failed: usize,
    pub bytes: u64,
    pub manifest_exchanged: bool,
    pub error: Option<String>,
}

impl TransferOutcome {
    pub fn is_ok(&self) -> bool {
        self.failed == 0 && self.error.is_none()
    }
}

/// A transfer job: the list of files to move and the negotiated session id.
/// `TransferJob` is a passive descriptor; the actual send/receive is driven
/// by `SendClient` / `ReceiveServer`.
#[derive(Debug, Clone)]
pub struct TransferJob {
    pub session_id: String,
    pub files: Vec<FileMetadata>,
    pub parallel_streams: u8,
    pub chunk_size: u64,
    pub checksum_algo: String,
}
