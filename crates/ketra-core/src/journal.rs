//! Append-only copy journal for crash-safe resumption.
//!
//! While Phase 2 copies a file, one JSONL record per flush interval is
//! appended to `<dst>/.ketracopy-journal.jsonl`:
//!
//! ```json
//! {"rel":"docs/a.bin","size":123456,"chunk":1048576,"offset":1048576,"status":"partial"}
//! ```
//!
//! `offset` is the number of leading bytes of the destination file that the
//! engine believes are already correct. On resume the engine treats a
//! journal-recorded offset as an upper bound and probes the real copied
//! extent (`resume::probe_resume`) before continuing.

use crate::error::{Error, Result};
use crate::json;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// One journal record.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    /// Destination-relative path.
    pub rel: String,
    /// Full expected size in bytes.
    pub size: u64,
    /// Chunk size used by the interrupted run.
    pub chunk: u64,
    /// Trusted copied extent (upper bound).
    pub offset: u64,
    /// "partial" or "done".
    pub status: String,
}

/// Append-mode journal writer.
pub struct Journal {
    path: PathBuf,
    file: Option<File>,
}

impl Journal {
    /// Open (creating or appending) the journal at `path`.
    pub fn open(path: &Path) -> Result<Journal> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::ioctx(parent, e))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| Error::ioctx(path, e))?;
        Ok(Journal { path: path.to_path_buf(), file: Some(file) })
    }

    /// Path of the journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record and flush it to disk.
    pub fn record(&mut self, e: &JournalEntry) -> Result<()> {
        let line = format!(
            "{{\"rel\":{},\"size\":{},\"chunk\":{},\"offset\":{},\"status\":{}}}\n",
            json::quote(&e.rel),
            e.size,
            e.chunk,
            e.offset,
            json::quote(&e.status)
        );
        if let Some(f) = self.file.as_mut() {
            f.write_all(line.as_bytes()).map_err(|er| Error::ioctx(&self.path, er))?;
            f.flush().map_err(|er| Error::ioctx(&self.path, er))?;
        }
        Ok(())
    }

    /// Delete the journal file (successful completion).
    pub fn finish(self) -> Result<()> {
        drop(self.file);
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::ioctx(&self.path, e)),
        }
    }
}

/// Load all records of a journal file. Newest record per `rel` wins.
pub fn load(path: &Path) -> Result<Vec<JournalEntry>> {
    let f = File::open(path).map_err(|e| Error::ioctx(path, e))?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line.map_err(|e| Error::ioctx(path, e))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_line(line) {
            Ok(e) => out.push(e),
            // A torn final line (crash mid-write) is expected and ignored.
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// Latest offset per rel: (offset, done?).
pub fn latest_offsets(path: &Path) -> Result<std::collections::BTreeMap<String, (u64, bool)>> {
    let mut map = std::collections::BTreeMap::new();
    for e in load(path)? {
        let done = e.status == "done";
        map.insert(e.rel, (e.offset, done));
    }
    Ok(map)
}

fn parse_line(line: &str) -> Result<JournalEntry> {
    let v = json::parse(line).map_err(Error::Config)?;
    let rel = v
        .get("rel")
        .and_then(|j| j.as_str())
        .ok_or_else(|| Error::Config("journal: missing rel".into()))?
        .to_string();
    let size = v
        .get("size")
        .and_then(|j| j.as_u64())
        .ok_or_else(|| Error::Config("journal: missing size".into()))?;
    let chunk = v.get("chunk").and_then(|j| j.as_u64()).unwrap_or(0);
    let offset = v
        .get("offset")
        .and_then(|j| j.as_u64())
        .ok_or_else(|| Error::Config("journal: missing offset".into()))?;
    let status = v.get("status").and_then(|j| j.as_str()).unwrap_or("partial").to_string();
    Ok(JournalEntry { rel, size, chunk, offset, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_record_load_roundtrip_and_latest_wins() {
        let dir = tmp("rt");
        let path = dir.join(".ketracopy-journal.jsonl");
        {
            let mut j = Journal::open(&path).unwrap();
            j.record(&JournalEntry {
                rel: "a.bin".into(),
                size: 100,
                chunk: 10,
                offset: 10,
                status: "partial".into(),
            })
            .unwrap();
            j.record(&JournalEntry {
                rel: "a.bin".into(),
                size: 100,
                chunk: 10,
                offset: 40,
                status: "partial".into(),
            })
            .unwrap();
            j.record(&JournalEntry {
                rel: "b.bin".into(),
                size: 5,
                chunk: 10,
                offset: 5,
                status: "done".into(),
            })
            .unwrap();
        }
        let entries = load(&path).unwrap();
        assert_eq!(entries.len(), 3);
        let latest = latest_offsets(&path).unwrap();
        assert_eq!(latest.get("a.bin"), Some(&(40, false)));
        assert_eq!(latest.get("b.bin"), Some(&(5, true)));

        // finish() deletes the file
        let j = Journal::open(&path).unwrap();
        j.finish().unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_torn_final_line_is_tolerated() {
        let dir = tmp("torn");
        let path = dir.join(".ketracopy-journal.jsonl");
        std::fs::write(
            &path,
            "{\"rel\":\"a\",\"size\":1,\"chunk\":1,\"offset\":1,\"status\":\"partial\"}\n{\"rel\":\"b\",\"size\":2,\"chu",
        )
        .unwrap();
        let entries = load(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rel, "a");
        std::fs::remove_dir_all(&dir).ok();
    }
}
