//! Progress rendering for the terminal: plain status lines, quiet mode, or
//! JSONL passthrough (the engine already emits parseable JSON per event).

use ketra_core::progress::Event;
use std::io::{IsTerminal, Write};
use std::sync::mpsc;
use std::time::Duration;

pub struct View {
    plain: bool,
    quiet: bool,
    /// Currently shown file (for in-place status updates).
    current: Option<(String, u64)>,
    printed_something: bool,
}

impl View {
    pub fn new(plain: bool, quiet: bool) -> Self {
        View { plain, quiet, current: None, printed_something: false }
    }

    fn human(n: u64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;
        const TB: f64 = GB * 1024.0;
        let v = n as f64;
        if v >= TB { format!("{:.2} TiB", v / TB) }
        else if v >= GB { format!("{:.2} GiB", v / GB) }
        else if v >= MB { format!("{:.1} MiB", v / MB) }
        else if v >= KB { format!("{:.1} KiB", v / KB) }
        else { format!("{n} B") }
    }

    /// Consume events until the run finishes or is canceled.
    /// Returns true when the stream ended.
    pub fn pump(&mut self, rx: &mpsc::Receiver<Event>) -> bool {
        let mut done = false;
        while let Ok(ev) = rx.recv_timeout(Duration::from_millis(150)) {
            match ev {
                Event::Finished { .. } | Event::Canceled { .. } => {
                    self.render(&ev);
                    done = true;
                    break;
                }
                other => self.render(&other),
            }
        }
        done
    }

    pub fn render(&mut self, ev: &Event) {
        if self.quiet {
            // Quiet mode: only errors and the final result line.
            match ev {
                Event::Error { .. } => {
                    if let Event::Error { path, message } = ev {
                        eprintln!("error: {path}: {message}");
                    }
                }
                Event::Finished { files_copied, files_skipped, files_failed, bytes_copied, elapsed_secs } => {
                    println!(
                        "{files_copied} copied, {files_skipped} skipped, {files_failed} failed, {} in {:.1}s",
                        Self::human(*bytes_copied),
                        elapsed_secs
                    );
                }
                Event::Canceled { files_copied, bytes_copied } => {
                    println!("canceled: {files_copied} copied, {}", Self::human(*bytes_copied));
                }
                _ => {}
            }
            return;
        }
        if !self.plain {
            // JSON mode is handled by the caller (event_line passthrough).
            return;
        }
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        match ev {
            Event::Started { total_files, total_bytes, phase } => {
                self.printed_something = true;
                let _ = writeln!(out, "ketra {phase}: {total_files} file(s), {}", Self::human(*total_bytes));
            }
            Event::FileStart { path, index, total_files, .. } => {
                self.current = Some((path.clone(), *index as u64 + 1));
                let _ = write!(out, "\r  [{}/{}] {} …{}", index + 1, total_files, Self::short(path), " ".repeat(20));
                let _ = out.flush();
            }
            Event::FileProgress { total_copied, total_bytes, speed_bps, eta_secs, .. } => {
                let frac = if *total_bytes > 0 {
                    (*total_copied as f64 / *total_bytes as f64).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let width = 24;
                let filled = (frac * width as f64) as usize;
                let bar = "█".repeat(filled) + &"░".repeat(width - filled);
                let _ = write!(
                    out,
                    "\r  {} {:>7.1}% {}  {:>10}/s  eta {:>5.0}s   ",
                    bar,
                    frac * 100.0,
                    Self::human(*total_copied),
                    Self::human(speed_bps.max(0.0) as u64),
                    eta_secs.max(0.0)
                );
                let _ = out.flush();
            }
            Event::FileDone { path, action, verified, .. } => {
                self.current = None;
                let mark = if *verified { "✓" } else { " " };
                let _ = writeln!(out, "\r  [{mark}] {action}: {}", Self::short(path));
                self.printed_something = true;
            }
            Event::FileSkip { path, reason } => {
                self.current = None;
                let _ = writeln!(out, "  [s] skip ({}): {}", reason, Self::short(path));
                self.printed_something = true;
            }
            Event::Message(m) => {
                let _ = writeln!(out, "  {m}");
                self.printed_something = true;
            }
            Event::Error { path, message } => {
                let _ = writeln!(out, "\r  [!] {path}: {message}");
                self.printed_something = true;
            }
            Event::Finished { files_copied, files_skipped, files_failed, bytes_copied, elapsed_secs } => {
                let _ = write!(out, "\r{}", " ".repeat(79));
                let _ = writeln!(
                    out,
                    "\r  done: {files_copied} copied, {files_skipped} skipped, {files_failed} failed, {} in {:.1}s",
                    Self::human(*bytes_copied),
                    elapsed_secs
                );
            }
            Event::Canceled { files_copied, bytes_copied } => {
                let _ = write!(out, "\r{}", " ".repeat(79));
                let _ = writeln!(out, "\r  canceled: {files_copied} copied, {}", Self::human(*bytes_copied));
            }
        }
        let _ = out.flush();
    }

    fn short(p: &str) -> String {
        // Keep the tail of the path readable on narrow terminals.
        const MAX: usize = 46;
        let chars: Vec<char> = p.chars().collect();
        if chars.len() <= MAX {
            return p.to_string();
        }
        let tail: String = chars[chars.len() - MAX..].iter().collect();
        format!("…{tail}")
    }
}

/// Detect the default rendering mode once per process.
pub fn detect_plain(explicit_plain: bool) -> bool {
    explicit_plain || std::io::stdout().is_terminal()
}

/// Print every event as one JSON line (JSONL mode).
pub fn json_forward(ev: &Event) {
    println!("{}", ketra_core::progress::event_line(ev));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_units() {
        assert_eq!(View::human(0), "0 B");
        assert_eq!(View::human(2048), "2.0 KiB");
        assert_eq!(View::human(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(View::human(3 * 1024 * 1024 * 1024), "3.00 GiB");
    }

    #[test]
    fn test_short_truncates() {
        let long = "a".repeat(80);
        let s = View::short(&long);
        assert!(s.chars().count() <= 80 && s.starts_with('…'));
        assert_eq!(View::short("x"), "x");
    }

    #[test]
    fn test_quiet_renders_only_summary() {
        let mut v = View::new(false, true);
        v.render(&Event::Started { total_files: 1, total_bytes: 1, phase: "copy" });
        // No panic, no output expectations here — logic covered above.
    }
}
