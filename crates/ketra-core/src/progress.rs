//! Progress events (streamed to the CLI as JSONL) and the aggregate
//! progress tracker (speed EMA, ETA, percentages).

use crate::json::quote;
use std::time::{Duration, Instant};

/// One progress event emitted by the engine.
#[derive(Debug, Clone)]
pub enum Event {
    /// The run started.
    Started {
        total_files: u32,
        total_bytes: u64,
        phase: &'static str,
    },
    /// A file began transferring.
    FileStart {
        path: String,
        size: u64,
        index: u32,
        total_files: u32,
    },
    /// Periodic progress inside a file.
    FileProgress {
        path: String,
        copied: u64,
        size: u64,
        file_index: u32,
        total_copied: u64,
        total_bytes: u64,
        speed_bps: f64,
        eta_secs: f64,
    },
    /// A file finished successfully.
    FileDone {
        path: String,
        bytes: u64,
        action: &'static str,
        verified: bool,
    },
    /// A file was skipped (up to date / conflict policy).
    FileSkip {
        path: String,
        reason: &'static str,
    },
    /// A human-readable note.
    Message(String),
    /// A per-file error (the run continues with other files).
    Error {
        path: String,
        message: String,
    },
    /// The run finished.
    Finished {
        files_copied: u32,
        files_skipped: u32,
        files_failed: u32,
        bytes_copied: u64,
        elapsed_secs: f64,
    },
    /// The run was canceled.
    Canceled {
        files_copied: u32,
        bytes_copied: u64,
    },
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "-1.000".to_string()
    }
}

/// Serialize an event as one JSON object line (without newline).
pub fn event_line(e: &Event) -> String {
    match e {
        Event::Started { total_files, total_bytes, phase } => format!(
            "{{\"event\":\"started\",\"total_files\":{total_files},\"total_bytes\":{total_bytes},\"phase\":\"{phase}\"}}"
        ),
        Event::FileStart { path, size, index, total_files } => format!(
            "{{\"event\":\"file_start\",\"path\":{},\"size\":{size},\"index\":{index},\"total_files\":{total_files}}}",
            quote(path)
        ),
        Event::FileProgress { path, copied, size, file_index, total_copied, total_bytes, speed_bps, eta_secs } => format!(
            "{{\"event\":\"file_progress\",\"path\":{},\"copied\":{copied},\"size\":{size},\"file_index\":{file_index},\"total_copied\":{total_copied},\"total_bytes\":{total_bytes},\"speed_bps\":{},\"eta_secs\":{}}}",
            quote(path),
            num(*speed_bps),
            num(*eta_secs)
        ),
        Event::FileDone { path, bytes, action, verified } => format!(
            "{{\"event\":\"file_done\",\"path\":{},\"bytes\":{bytes},\"action\":\"{action}\",\"verified\":{}}}",
            quote(path),
            verified
        ),
        Event::FileSkip { path, reason } => format!(
            "{{\"event\":\"file_skip\",\"path\":{},\"reason\":\"{reason}\"}}",
            quote(path)
        ),
        Event::Message(m) => format!("{{\"event\":\"message\",\"text\":{}}}", quote(m)),
        Event::Error { path, message } => format!(
            "{{\"event\":\"error\",\"path\":{},\"message\":{}}}",
            quote(path),
            quote(message)
        ),
        Event::Finished { files_copied, files_skipped, files_failed, bytes_copied, elapsed_secs } => format!(
            "{{\"event\":\"finished\",\"files_copied\":{files_copied},\"files_skipped\":{files_skipped},\"files_failed\":{files_failed},\"bytes_copied\":{bytes_copied},\"elapsed_secs\":{}}}",
            num(*elapsed_secs)
        ),
        Event::Canceled { files_copied, bytes_copied } => format!(
            "{{\"event\":\"canceled\",\"files_copied\":{files_copied},\"bytes_copied\":{bytes_copied}}}"
        ),
    }
}

/// Aggregate run-level progress with speed smoothing.
#[derive(Clone)]
pub struct Progress {
    pub total_files: u32,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub files_done: u32,
    start: Instant,
    last_sample: Instant,
    last_bytes: u64,
    speed_ema: f64,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    /// Start tracking at now.
    pub fn new() -> Self {
        let now = Instant::now();
        Progress {
            total_files: 0,
            total_bytes: 0,
            copied_bytes: 0,
            files_done: 0,
            start: now,
            last_sample: now,
            last_bytes: 0,
            speed_ema: 0.0,
        }
    }

    /// Account for newly copied bytes.
    pub fn add_bytes(&mut self, n: u64) {
        self.copied_bytes = self.copied_bytes.saturating_add(n);
        let now = Instant::now();
        let dt = now.duration_since(self.last_sample).as_secs_f64();
        if dt >= 0.2 {
            let db = (self.copied_bytes - self.last_bytes) as f64;
            let inst = db / dt;
            self.speed_ema = if self.speed_ema == 0.0 { inst } else { self.speed_ema * 0.7 + inst * 0.3 };
            self.last_sample = now;
            self.last_bytes = self.copied_bytes;
        }
    }

    /// Mark one file as finished.
    pub fn file_done(&mut self) {
        self.files_done = self.files_done.saturating_add(1);
    }

    /// Smoothed throughput estimate in bytes/sec.
    pub fn speed_bps(&self) -> f64 {
        if self.speed_ema > 0.0 {
            self.speed_ema
        } else {
            let el = self.start.elapsed().as_secs_f64();
            if el > 0.05 { self.copied_bytes as f64 / el } else { 0.0 }
        }
    }

    /// Estimated seconds to completion (based on the smoothed speed).
    pub fn eta_secs(&self) -> f64 {
        let s = self.speed_bps();
        if s <= 0.0 || self.total_bytes == 0 {
            return -1.0;
        }
        let remaining = self.total_bytes.saturating_sub(self.copied_bytes) as f64;
        remaining / s
    }

    /// Fraction completed, 0.0..=1.0.
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            if self.total_files == 0 {
                return 1.0;
            }
            return if self.files_done >= self.total_files { 1.0 } else { 0.0 };
        }
        (self.copied_bytes as f64 / self.total_bytes as f64).min(1.0)
    }

    /// Seconds elapsed since tracking started.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_lines_are_valid_json_objects() {
        let evs = vec![
            Event::Started { total_files: 3, total_bytes: 100, phase: "copy" },
            Event::FileStart { path: "a b/c\"d.txt".into(), size: 10, index: 0, total_files: 3 },
            Event::FileProgress {
                path: "x".into(),
                copied: 5,
                size: 10,
                file_index: 0,
                total_copied: 5,
                total_bytes: 100,
                speed_bps: 1234.5,
                eta_secs: -1.0,
            },
            Event::FileDone { path: "x".into(), bytes: 10, action: "copied", verified: true },
            Event::FileSkip { path: "y".into(), reason: "up_to_date" },
            Event::Message("hello".into()),
            Event::Error { path: "z".into(), message: "denied".into() },
            Event::Finished {
                files_copied: 2,
                files_skipped: 1,
                files_failed: 0,
                bytes_copied: 100,
                elapsed_secs: 1.25,
            },
            Event::Canceled { files_copied: 1, bytes_copied: 50 },
        ];
        for e in &evs {
            let line = event_line(e);
            assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
            assert!(crate::json::parse(&line).is_ok(), "bad json: {line}");
        }
    }

    #[test]
    fn test_quotes_inside_paths_are_escaped() {
        let line = event_line(&Event::FileStart {
            path: "we\"ird\\path\n".into(),
            size: 1,
            index: 0,
            total_files: 1,
        });
        assert!(crate::json::parse(&line).is_ok());
        assert!(line.contains("\\n"));
    }

    #[test]
    fn test_progress_math() {
        let mut p = Progress::new();
        p.total_bytes = 1000;
        p.add_bytes(250);
        assert_eq!(p.copied_bytes, 250);
        assert!((p.fraction() - 0.25).abs() < 1e-9);
        p.add_bytes(750);
        assert!((p.fraction() - 1.0).abs() < 1e-9);
        // eta is either -1 (no speed yet) or a positive finite number
        let eta = p.eta_secs();
        assert!(eta < 0.0 || eta.is_finite());
    }

    #[test]
    fn test_progress_file_counting_saturates() {
        let mut p = Progress::new();
        for _ in 0..5 {
            p.file_done();
        }
        assert_eq!(p.files_done, 5);
    }
}
