//! OS clipboard file interop — the bridge that lets KetraCopy integrate with
//! (and take over) the native copy / cut / paste / move flows of Explorer,
//! Finder and the GTK/KDE file managers.
//!
//! ## How the three platforms represent a file clipboard
//!
//! | Platform   | Write format                                     | Cut marker |
//! |---|---|---|
//! | Windows    | `CF_HDROP` (`DROPFILES` struct, UTF-16 list)     | `Preferred DropEffect` = `MOVE` |
//! | Linux/X11  | `text/uri-list` + `text/x-special/gnome-copied-files` | gnome prefix `cut` / `application/x-kde-cutselection` |
//! | macOS      | Finder pasteboard via AppleScript («class furl») | side-channel mode file (Finder's Cmd+X state is not scriptable) |
//!
//! The codecs for all three formats are **pure functions compiled and
//! unit-tested on every platform**; only the backend that talks to the OS is
//! `#[cfg]`-selected.
//!
//! ## Paste takeover
//!
//! `ketra clip paste -d DIR` reads whatever the *native* file manager put on
//! the clipboard (including its cut state) and executes the transfer through
//! the KetraCopy engine — accelerated, verified, resumable.

use crate::args::CommonArgs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone)]
pub struct ClipData {
    pub paths: Vec<PathBuf>,
    pub mode: ClipMode,
}

impl ClipData {
    pub fn mode_str(&self) -> &'static str {
        match self.mode {
            ClipMode::Copy => "copy",
            ClipMode::Cut => "cut",
        }
    }
}

// ---------------------------------------------------------------------------
// Pure codecs (compiled + tested on ALL platforms)
// ---------------------------------------------------------------------------

/// Percent-encode a path for a `file://` URI (RFC 8089; keep the safe set).
fn uri_escape(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 8);
    for b in p.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn uri_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `text/uri-list` body: one `file://` URI per line (CRLF tolerated by all
/// file managers; we write CRLF per RFC 2483 and accept LF on read).
pub fn encode_uri_list(paths: &[PathBuf]) -> String {
    let mut s = String::new();
    for p in paths {
        s.push_str(&format!("file://{}\r\n", uri_escape(&p.display().to_string())));
    }
    s
}

pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let p = line.strip_prefix("file://").unwrap_or(line);
        let p = uri_unescape(p);
        if !p.is_empty() {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// GNOME/Nemo/Nautilus flavour: first line is `copy` or `cut`, then one
/// `file://` URI per line (LF separated).
pub const GNOME_COPIED_FILES_TYPE: &str = "text/x-special/gnome-copied-files";
pub const URI_LIST_TYPE: &str = "text/uri-list";
pub const KDE_CUT_TYPE: &str = "application/x-kde-cutselection";

pub fn encode_gnome_files(mode: ClipMode, paths: &[PathBuf]) -> String {
    let mut s = String::new();
    s.push_str(match mode {
        ClipMode::Copy => "copy",
        ClipMode::Cut => "cut",
    });
    for p in paths {
        s.push('\n');
        s.push_str(&format!("file://{}", uri_escape(&p.display().to_string())));
    }
    s
}

pub fn parse_gnome_files(text: &str) -> Option<ClipData> {
    let mut lines = text.lines();
    let head = lines.next()?.trim().to_ascii_lowercase();
    let mode = match head.as_str() {
        "cut" | "move" => ClipMode::Cut,
        "copy" => ClipMode::Copy,
        _ => return None,
    };
    let paths = parse_uri_list(&lines.collect::<Vec<_>>().join("\n"));
    if paths.is_empty() {
        return None;
    }
    Some(ClipData { paths, mode })
}

/// Windows `DROPFILES` blob: 20-byte header followed by a UTF-16LE,
/// NUL-separated, double-NUL-terminated absolute path list.
///
/// Layout: `u32 pFiles` (offset of the list), `POINT pt` (2×i32), `BOOL fNC`,
/// `BOOL fWide`.
pub fn encode_dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    const HEADER: usize = 20;
    let mut w: Vec<u16> = Vec::new();
    for p in paths {
        for u in p.display().to_string().encode_utf16() {
            w.push(u);
        }
        w.push(0);
    }
    w.push(0); // terminating extra NUL
    let mut out = Vec::with_capacity(HEADER + w.len() * 2);
    out.extend_from_slice(&(HEADER as u32).to_le_bytes()); // pFiles = 20
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0u32.to_le_bytes()); // fNC = FALSE
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide = TRUE
    for u in &w {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// Parse a `DROPFILES` blob (without the DropEffect; that lives in its own
/// clipboard format). Tolerates ANSI (non-wide) blobs.
pub fn parse_dropfiles(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.len() < 20 {
        return None;
    }
    let off = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if off < 20 || off > bytes.len() {
        return None;
    }
    let wide = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) == 1;
    let list = &bytes[off..];
    let mut out = Vec::new();
    if wide {
        let u16s: Vec<u16> = list
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut cur: Vec<u16> = Vec::new();
        for u in u16s {
            if u == 0 {
                if cur.is_empty() {
                    break; // double NUL = end
                }
                out.push(String::from_utf16_lossy(&cur));
                cur.clear();
            } else {
                cur.push(u);
            }
        }
    } else {
        let mut cur: Vec<u8> = Vec::new();
        for b in list {
            if *b == 0 {
                if cur.is_empty() {
                    break;
                }
                out.push(String::from_utf8_lossy(&cur).into_owned());
                cur.clear();
            } else {
                cur.push(*b);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Windows `Preferred DropEffect` DWORD: bit0 = copy, bit1 = move.
pub fn encode_drop_effect(mode: ClipMode) -> [u8; 4] {
    match mode {
        ClipMode::Copy => 1u32.to_le_bytes(),
        ClipMode::Cut => 2u32.to_le_bytes(),
    }
}

pub fn parse_drop_effect(bytes: &[u8]) -> Option<ClipMode> {
    if bytes.len() < 4 {
        return None;
    }
    let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    Some(if v & 2 != 0 { ClipMode::Cut } else { ClipMode::Copy })
}

/// Make a path absolute against the process CWD (clipboards want absolute
/// paths; relative paths confuse Explorer/Nemo).
pub fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(p),
            Err(_) => p.to_path_buf(),
        }
    }
}

// ---------------------------------------------------------------------------
// Backend: Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod backend {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    const CF_HDROP: u32 = 15;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: isize) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(fmt: u32, handle: isize) -> isize;
        fn GetClipboardData(fmt: u32) -> isize;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
        fn GlobalLock(handle: isize) -> *mut u8;
        fn GlobalUnlock(handle: isize) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    pub fn put(paths: &[PathBuf], mode: ClipMode) -> Result<(), String> {
        let paths: Vec<PathBuf> = paths.iter().map(|p| absolutize(p)).collect();
        let blob = encode_dropfiles(&paths);
        let effect = encode_drop_effect(mode);

        unsafe {
            if OpenClipboard(0) == 0 {
                return Err("cannot open the clipboard (is another app holding it?)".into());
            }
            let result = (|| {
                EmptyClipboard();

                // CF_HDROP
                let h = GlobalAlloc(GMEM_MOVEABLE, blob.len());
                if h == 0 {
                    return Err("GlobalAlloc failed".into());
                }
                let dst = GlobalLock(h);
                if dst.is_null() {
                    return Err("GlobalLock failed".into());
                }
                std::ptr::copy_nonoverlapping(blob.as_ptr(), dst, blob.len());
                GlobalUnlock(h);
                if SetClipboardData(CF_HDROP, h) == 0 {
                    return Err("SetClipboardData(CF_HDROP) failed".into());
                }

                // Preferred DropEffect (copy vs cut)
                let name = wide("Preferred DropEffect");
                let fmt = RegisterClipboardFormatW(name.as_ptr());
                if fmt != 0 {
                    let h2 = GlobalAlloc(GMEM_MOVEABLE, effect.len());
                    if h2 != 0 {
                        let dst2 = GlobalLock(h2);
                        if !dst2.is_null() {
                            std::ptr::copy_nonoverlapping(effect.as_ptr(), dst2, effect.len());
                            GlobalUnlock(h2);
                            SetClipboardData(fmt, h2);
                        }
                    }
                }
                Ok(())
            })();
            CloseClipboard();
            result
        }
    }

    pub fn get() -> Result<ClipData, String> {
        unsafe {
            if OpenClipboard(0) == 0 {
                return Err("cannot open the clipboard".into());
            }
            let result = (|| {
                let h = GetClipboardData(CF_HDROP);
                if h == 0 {
                    return Err("the clipboard does not contain files (copy some first)".into());
                }
                let p = GlobalLock(h);
                if p.is_null() {
                    return Err("GlobalLock failed".into());
                }
                // DROPFILES is double-NUL terminated; scan until we see it
                // (hard cap 1 MiB, well below any sane clipboard payload).
                let mut bytes = Vec::with_capacity(64 * 1024);
                let mut i = 0usize;
                loop {
                    if i >= 1024 * 1024 {
                        break;
                    }
                    let b = *p.add(i);
                    bytes.push(b);
                    if i >= 8
                        && bytes[i] == 0
                        && bytes[i - 1] == 0
                        && bytes[i - 2] == 0
                        && bytes[i - 3] == 0
                        && bytes[i - 4] == 0
                        && bytes[i - 5] == 0
                        && bytes[i - 6] == 0
                        && bytes[i - 7] == 0
                    {
                        break;
                    }
                    i += 1;
                }
                GlobalUnlock(h);
                let paths = parse_dropfiles(&bytes)
                    .ok_or("clipboard file list is malformed")?
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();

                // Preferred DropEffect: cut or copy?
                let name = wide("Preferred DropEffect");
                let fmt = RegisterClipboardFormatW(name.as_ptr());
                let mut mode = ClipMode::Copy;
                if fmt != 0 {
                    let h2 = GetClipboardData(fmt);
                    if h2 != 0 {
                        let p2 = GlobalLock(h2);
                        if !p2.is_null() {
                            let eff = [*p2, *p2.add(1), *p2.add(2), *p2.add(3)];
                            GlobalUnlock(h2);
                            if let Some(m) = parse_drop_effect(&eff) {
                                mode = m;
                            }
                        }
                    }
                }
                if paths.is_empty() {
                    Err("the clipboard does not contain files".into())
                } else {
                    Ok(ClipData { paths, mode })
                }
            })();
            CloseClipboard();
            result
        }
    }
}

// ---------------------------------------------------------------------------
// Backend: macOS (osascript; Finder «class furl» pasteboard)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod backend {
    use super::*;
    use std::process::Command;

    fn mode_file() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/ketra/clipboard-mode"))
    }

    /// Finder's own Cmd+X state is not scriptable; we persist our cut flag
    /// side-channel (with a 1-hour expiry) for `ketra put --cut`.
    fn store_mode(mode: ClipMode) {
        if let Some(p) = mode_file() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let body = match mode {
                ClipMode::Cut => format!("cut {stamp}"),
                ClipMode::Copy => format!("copy {stamp}"),
            };
            let _ = std::fs::write(p, body);
        }
    }

    fn stored_mode() -> Option<ClipMode> {
        let p = mode_file()?;
        let body = std::fs::read_to_string(p).ok()?;
        let mut it = body.split_whitespace();
        let m = it.next()?;
        let stamp: u64 = it.next().unwrap_or("0").parse().ok()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now.saturating_sub(stamp) > 3600 {
            return None; // stale side-channel: default to copy
        }
        match m {
            "cut" => Some(ClipMode::Cut),
            _ => Some(ClipMode::Copy),
        }
    }

    pub fn put(paths: &[PathBuf], mode: ClipMode) -> Result<(), String> {
        let paths: Vec<String> = paths.iter().map(|p| absolutize(p).display().to_string()).collect();
        let script = "on run argv\n  set fl to {}\n  repeat with p in argv\n    set end of fl to POSIX file (p as text)\n  end repeat\n  set the clipboard to fl\nend run";
        let mut cmd = Command::new("/usr/bin/osascript");
        cmd.arg("-e").arg(script).arg("--").args(&paths);
        let out = cmd.output().map_err(|e| format!("osascript: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "osascript failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        store_mode(mode);
        Ok(())
    }

    pub fn get() -> Result<ClipData, String> {
        let script = "set out to \"\"\ntry\n  set fl to the clipboard as list\non error\n  try\n    set fl to {the clipboard as «class furl»}\n  on error\n    return \"\"\n  end try\nend try\nrepeat with f in fl\n  try\n    set out to out & (POSIX path of f) & linefeed\n  end try\nend repeat\nreturn out";
        let out = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let paths: Vec<PathBuf> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        if paths.is_empty() {
            return Err("the clipboard does not contain files (copy some first)".into());
        }
        let mode = stored_mode().unwrap_or(ClipMode::Copy);
        Ok(ClipData { paths, mode })
    }
}

// ---------------------------------------------------------------------------
// Backend: Linux/X11 (dlopen libX11; detached selection-owner daemon)
// ---------------------------------------------------------------------------

#[cfg(all(unix, not(target_os = "macos")))]
mod backend {
    use super::*;
    use crate::clip::x11;

    /// State file handed to the detached selection-owner daemon.
    fn server_state_file() -> PathBuf {
        std::env::temp_dir().join(format!("ketra-clip-{}.state", std::process::id()))
    }

    pub fn put(paths: &[PathBuf], mode: ClipMode) -> Result<(), String> {
        if std::env::var_os("DISPLAY").is_none() {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                return Err(
                    "no X11 clipboard on this Wayland session: use the file-manager right-click actions instead".into(),
                );
            }
            return Err("no X11 display ($DISPLAY is unset)".into());
        }
        let paths: Vec<PathBuf> = paths.iter().map(|p| absolutize(p)).collect();
        let mut state = String::new();
        state.push_str(match mode {
            ClipMode::Copy => "copy",
            ClipMode::Cut => "cut",
        });
        state.push('\n');
        for p in &paths {
            state.push_str(&p.display().to_string());
            state.push('\n');
        }
        let file = server_state_file();
        std::fs::write(&file, state).map_err(|e| format!("state file: {e}"))?;

        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let mut child = std::process::Command::new(exe)
            .arg("__clip-server")
            
            .arg(&file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn clipboard daemon: {e}"))?;
        // Give it a moment to take the selection, then confirm it is alive.
        std::thread::sleep(Duration::from_millis(150));
        if let Ok(Some(_)) = child.try_wait() {
            let _ = std::fs::remove_file(&file);
            return Err("clipboard daemon exited immediately (no X11 display?)".into());
        }
        Ok(())
    }

    pub fn get() -> Result<ClipData, String> {
        if std::env::var_os("DISPLAY").is_none() {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                return Err(
                    "no X11 clipboard on this Wayland session: use the file-manager right-click actions instead".into(),
                );
            }
            return Err("no X11 display ($DISPLAY is unset)".into());
        }
        x11::read_clipboard_files()
    }
}

// ---------------------------------------------------------------------------
// Facade
// ---------------------------------------------------------------------------

fn backend_put(paths: &[PathBuf], mode: ClipMode) -> Result<(), String> {
    #[cfg(windows)]
    {
        return backend::put(paths, mode);
    }
    #[cfg(target_os = "macos")]
    {
        return backend::put(paths, mode);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return backend::put(paths, mode);
    }
}

fn backend_get() -> Result<ClipData, String> {
    #[cfg(windows)]
    {
        return backend::get();
    }
    #[cfg(target_os = "macos")]
    {
        return backend::get();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return backend::get();
    }
}

/// `ketra clip put <paths> [--cut]`
pub fn put_cmd(paths: Vec<PathBuf>, cut: bool) -> i32 {
    let mode = if cut { ClipMode::Cut } else { ClipMode::Copy };
    let missing: Vec<String> = paths
        .iter()
        .filter(|p| !p.exists())
        .map(|p| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "ketra: clip put: {} path(s) do not exist: {}",
            missing.len(),
            missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        );
        return 3;
    }
    match backend_put(&paths, mode) {
        Ok(()) => {
            println!(
                "clipboard: {} item(s), mode {}",
                paths.len(),
                if cut { "cut" } else { "copy" }
            );
            0
        }
        Err(e) => {
            eprintln!("ketra: clip put: {e}");
            1
        }
    }
}

/// `ketra clip get [--json]`
pub fn get_cmd(json: bool) -> i32 {
    match backend_get() {
        Ok(data) => {
            if json {
                let items: Vec<String> = data
                    .paths
                    .iter()
                    .map(|p| format!("\"{}\"", ketra_core::json::quote(&p.display().to_string())))
                    .collect();
                println!(
                    "{{\"mode\":\"{}\",\"paths\":[{}]}}",
                    data.mode_str(),
                    items.join(",")
                );
            } else {
                println!("mode: {}", data.mode_str());
                for p in &data.paths {
                    println!("{}", p.display());
                }
            }
            0
        }
        Err(e) => {
            eprintln!("ketra: clip get: {e}");
            1
        }
    }
}

/// `ketra clip paste -d DIR [options]` — reads the native clipboard
/// (including cut state) and transfers through the KetraCopy engine.
pub fn paste_cmd(dest: PathBuf, common: &CommonArgs) -> i32 {
    let data = match backend_get() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ketra: clip paste: {e}");
            return 1;
        }
    };
    let missing: Vec<String> = data
        .paths
        .iter()
        .filter(|p| !p.exists())
        .map(|p| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "ketra: clip paste: {} clipboard item(s) no longer exist (stale clipboard): {}",
            missing.len(),
            missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        );
        return 1;
    }
    if !dest.is_dir() {
        eprintln!(
            "ketra: clip paste: destination {} does not exist (create it first)",
            dest.display()
        );
        return 3;
    }
    eprintln!(
        "ketra: pasting {} item(s), mode {} → {}",
        data.paths.len(),
        data.mode_str(),
        dest.display()
    );
    let mut common = common.clone();
    common.dest = dest;
    let kind = match data.mode {
        ClipMode::Copy => crate::run_cmd::TransferKind::Copy,
        ClipMode::Cut => crate::run_cmd::TransferKind::Move,
    };
    crate::run_cmd::run(kind, data.paths, &common).code
}

// ---------------------------------------------------------------------------
// X11 plumbing (Linux): selection owner daemon + clipboard reader, std-only
// via hand-declared FFI (dlopen libX11 at runtime — no build-time linkage).
// ---------------------------------------------------------------------------

#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11 {
    use super::{ClipData, ClipMode};
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    type XDisplay = std::ffi::c_void;
    type Window = c_ulong;
    type Atom = c_ulong;
    type Time = c_ulong;

    const SELECTION_CLEAR: c_int = 29;
    const SELECTION_REQUEST: c_int = 30;
    const SELECTION_NOTIFY: c_int = 31;
    const ANY_PROPERTY_TYPE: Atom = 0;
    const PROP_MODE_REPLACE: c_int = 0;
    const NO_EVENT_MASK: c_long = 0;
    const CURRENT_TIME: Time = 0;
    const XA_ATOM: Atom = 4;
    const XA_STRING: Atom = 31;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XSelectionRequestEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut XDisplay,
        owner: Window,
        requestor: Window,
        selection: Atom,
        target: Atom,
        property: Atom,
        time: Time,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XSelectionEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut XDisplay,
        owner: Window,
        requestor: Window,
        selection: Atom,
        target: Atom,
        property: Atom,
        time: Time,
    }

    /// sizeof(XEvent) is 192 bytes on LP64; only the leading fields are read,
    /// so a plain long-array + reinterpret is safe here.
    #[repr(C)]
    struct XEvent {
        pad: [c_long; 24],
    }

    // select() fd_set (1024-bit bitmap on glibc/musl)
    #[repr(C)]
    struct FdSet {
        bits: [u8; 128],
    }

    #[repr(C)]
    struct TimeVal {
        tv_sec: c_long,
        tv_usec: c_long,
    }

    extern "C" {
        fn select(
            nfds: c_int,
            readfds: *mut FdSet,
            writefds: *mut c_void,
            exceptfds: *mut c_void,
            timeout: *mut TimeVal,
        ) -> c_int;
    }

    fn fd_set_bit(set: &mut FdSet, fd: c_int) {
        let f = fd as usize;
        set.bits[f / 8] |= 1 << (f % 8);
    }

    struct X11 {
        XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut XDisplay,
        XCloseDisplay: unsafe extern "C" fn(*mut XDisplay) -> c_int,
        XDefaultRootWindow: unsafe extern "C" fn(*mut XDisplay) -> Window,
        XCreateSimpleWindow: unsafe extern "C" fn(
            *mut XDisplay,
            Window,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_uint,
            c_ulong,
            c_ulong,
        ) -> Window,
        XDestroyWindow: unsafe extern "C" fn(*mut XDisplay, Window) -> c_int,
        XSetSelectionOwner: unsafe extern "C" fn(*mut XDisplay, Atom, Window, Time) -> c_int,
        XGetSelectionOwner: unsafe extern "C" fn(*mut XDisplay, Atom) -> Window,
        XInternAtom: unsafe extern "C" fn(*mut XDisplay, *const c_char, c_int) -> Atom,
        XChangeProperty: unsafe extern "C" fn(
            *mut XDisplay,
            Window,
            Atom,
            Atom,
            c_int,
            c_int,
            *const u8,
            c_int,
        ) -> c_int,
        XSendEvent: unsafe extern "C" fn(*mut XDisplay, Window, c_int, c_long, *mut XEvent) -> c_int,
        XNextEvent: unsafe extern "C" fn(*mut XDisplay, *mut XEvent) -> c_int,
        XPending: unsafe extern "C" fn(*mut XDisplay) -> c_int,
        XFlush: unsafe extern "C" fn(*mut XDisplay) -> c_int,
        XConvertSelection: unsafe extern "C" fn(*mut XDisplay, Atom, Atom, Atom, Window, Time) -> c_int,
        XGetWindowProperty: unsafe extern "C" fn(
            *mut XDisplay,
            Window,
            Atom,
            c_long,
            c_long,
            c_int,
            Atom,
            *mut Atom,
            *mut c_int,
            *mut c_ulong,
            *mut c_ulong,
            *mut *mut u8,
        ) -> c_int,
        XFree: unsafe extern "C" fn(*mut u8) -> c_int,
        ConnectionNumber: unsafe extern "C" fn(*mut XDisplay) -> c_int,
    }

    impl X11 {
        /// dlopen libX11 and resolve every symbol we need. std links libdl on
        /// Linux, so plain `extern dlopen` resolves without extra link flags.
        fn load() -> Result<(Self, *mut XDisplay), String> {
            unsafe extern "C" {
                fn dlopen(filename: *const c_char, flags: c_int) -> *mut u8;
                fn dlsym(handle: *mut u8, symbol: *const c_char) -> *mut u8;
            }
            const RTLD_NOW: c_int = 2;
            unsafe {
                let lib = dlopen(b"libX11.so.6\0".as_ptr() as *const c_char, RTLD_NOW);
                let lib = if lib.is_null() {
                    dlopen(b"libX11.so\0".as_ptr() as *const c_char, RTLD_NOW)
                } else {
                    lib
                };
                if lib.is_null() {
                    return Err(
                        "libX11 not found (install libx11, or use the file-manager right-click actions)"
                            .into(),
                    );
                }
                let sym = |name: &str| -> Result<*mut u8, String> {
                    let s = CString::new(name).unwrap();
                    let p = dlsym(lib, s.as_ptr());
                    if p.is_null() {
                        return Err(format!("libX11 is missing symbol {name}"));
                    }
                    Ok(p)
                };
                fn take<T>(sym: &dyn Fn(&str) -> Result<*mut u8, String>, name: &str) -> Result<T, String> {
                    let p = sym(name)?;
                    Ok(unsafe { std::mem::transmute_copy::<*mut u8, T>(&p) })
                }
                let x = X11 {
                    XOpenDisplay: take(&sym, "XOpenDisplay")?,
                    XCloseDisplay: take(&sym, "XCloseDisplay")?,
                    XDefaultRootWindow: take(&sym, "XDefaultRootWindow")?,
                    XCreateSimpleWindow: take(&sym, "XCreateSimpleWindow")?,
                    XDestroyWindow: take(&sym, "XDestroyWindow")?,
                    XSetSelectionOwner: take(&sym, "XSetSelectionOwner")?,
                    XGetSelectionOwner: take(&sym, "XGetSelectionOwner")?,
                    XInternAtom: take(&sym, "XInternAtom")?,
                    XChangeProperty: take(&sym, "XChangeProperty")?,
                    XSendEvent: take(&sym, "XSendEvent")?,
                    XNextEvent: take(&sym, "XNextEvent")?,
                    XPending: take(&sym, "XPending")?,
                    XFlush: take(&sym, "XFlush")?,
                    XConvertSelection: take(&sym, "XConvertSelection")?,
                    XGetWindowProperty: take(&sym, "XGetWindowProperty")?,
                    XFree: take(&sym, "XFree")?,
                    ConnectionNumber: take(&sym, "ConnectionNumber")?,
                };
                let dpy = (x.XOpenDisplay)(std::ptr::null());
                if dpy.is_null() {
                    return Err("XOpenDisplay(NULL) failed (headless or Wayland-only session)".into());
                }
                // The library handle stays alive for the whole process.
                std::mem::forget(lib);
                Ok((x, dpy))
            }
        }
    }

    fn atom_or(dpy: *mut XDisplay, x: &X11, name: &str, fallback: Atom) -> Atom {
        let c = CString::new(name).unwrap();
        let a = unsafe { (x.XInternAtom)(dpy, c.as_ptr(), 0) };
        if a == 0 {
            fallback
        } else {
            a
        }
    }

    /// The detached selection-owner main loop (`ketra __clip-server`).
    ///
    /// State file: first line `copy`|`cut`, then one absolute path per line.
    /// Exits when selection ownership is lost (another copy or a clipboard
    /// manager takes over) — the payload lives on in whoever took it.
    pub fn serve(file: &PathBuf) -> i32 {
        let body = match std::fs::read_to_string(file) {
            Ok(b) => b,
            Err(_) => return 1,
        };
        let mut lines = body.lines();
        let mode = match lines.next().unwrap_or("copy") {
            "cut" => ClipMode::Cut,
            _ => ClipMode::Copy,
        };
        let paths: Vec<PathBuf> = lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        if paths.is_empty() {
            return 1;
        }

        let (x, dpy) = match X11::load() {
            Ok(v) => v,
            Err(_) => return 1,
        };
        unsafe {
            let clipboard = atom_or(dpy, &x, "CLIPBOARD", 0);
            if clipboard == 0 {
                return 1;
            }
            let targets = atom_or(dpy, &x, "TARGETS", 0);
            let utf8 = atom_or(dpy, &x, "UTF8_STRING", XA_STRING);
            let uri = atom_or(dpy, &x, super::URI_LIST_TYPE, 0);
            let gnome = atom_or(dpy, &x, super::GNOME_COPIED_FILES_TYPE, 0);
            let kde = atom_or(dpy, &x, super::KDE_CUT_TYPE, 0);

            let root = (x.XDefaultRootWindow)(dpy);
            let win = (x.XCreateSimpleWindow)(dpy, root, 0, 0, 1, 1, 0, 0, 0);
            (x.XSetSelectionOwner)(dpy, clipboard, win, CURRENT_TIME);
            if (x.XGetSelectionOwner)(dpy, clipboard) != win {
                return 1;
            }

            // Payloads (paths are files on disk, valid for the daemon's life).
            let uri_list = super::encode_uri_list(&paths);
            let gnome_files = super::encode_gnome_files(mode, &paths);
            let plain_list = paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let kde_flag: &[u8] = match mode {
                ClipMode::Cut => b"1",
                ClipMode::Copy => b"0",
            };

            // TARGETS reply (Xlib format=32 means c_long units)
            let mut target_atoms: Vec<c_ulong> = vec![
                targets as c_ulong,
                XA_ATOM,
                utf8,
                XA_STRING,
                uri,
                gnome,
            ];
            if kde != 0 {
                target_atoms.push(kde);
            }
            let targets_bytes = std::slice::from_raw_parts(
                target_atoms.as_ptr() as *const u8,
                target_atoms.len() * std::mem::size_of::<c_ulong>(),
            );

            let mut ev = XEvent { pad: [0; 24] };
            loop {
                (x.XNextEvent)(dpy, &mut ev);
                let etype = ev.pad[0] as c_int;
                if etype == SELECTION_REQUEST {
                    let req = *(&ev as *const XEvent as *const XSelectionRequestEvent);
                    let (payload, ptype, pfmt): (&[u8], Atom, c_int) = if req.target == targets {
                        (targets_bytes, XA_ATOM, 32)
                    } else if req.target == uri {
                        (uri_list.as_bytes(), uri, 8)
                    } else if req.target == gnome {
                        (gnome_files.as_bytes(), gnome, 8)
                    } else if kde != 0 && req.target == kde {
                        (kde_flag, kde, 8)
                    } else if req.target == utf8 || req.target == XA_STRING {
                        (plain_list.as_bytes(), XA_STRING, 8)
                    } else {
                        // Unknown target: refuse by storing nothing.
                        (&[], 0, 8)
                    };
                    (x.XChangeProperty)(
                        dpy,
                        req.requestor,
                        req.property,
                        ptype,
                        pfmt,
                        PROP_MODE_REPLACE,
                        payload.as_ptr(),
                        payload.len() as c_int,
                    );
                    let mut reply = XSelectionEvent {
                        type_: SELECTION_NOTIFY,
                        serial: 0,
                        send_event: 1,
                        display: dpy,
                        owner: req.owner,
                        requestor: req.requestor,
                        selection: req.selection,
                        target: req.target,
                        property: req.property,
                        time: CURRENT_TIME,
                    };
                    (x.XSendEvent)(
                        dpy,
                        req.requestor,
                        0,
                        NO_EVENT_MASK,
                        &mut reply as *mut XSelectionEvent as *mut XEvent,
                    );
                    (x.XFlush)(dpy);
                } else if etype == SELECTION_CLEAR {
                    // Ownership lost; another clipboard source took over.
                    let _ = std::fs::remove_file(file);
                    (x.XDestroyWindow)(dpy, win);
                    (x.XCloseDisplay)(dpy);
                    return 0;
                }
            }
        }
    }

    /// Read the CLIPBOARD selection as a file list. Preference order:
    /// 1. `text/x-special/gnome-copied-files` (mode inline: copy|cut)
    /// 2. `text/uri-list` (+ optional `application/x-kde-cutselection` = 1)
    /// 3. UTF8 text that parses as a file URI list
    pub fn read_clipboard_files() -> Result<ClipData, String> {
        let (x, dpy) = X11::load()?;
        unsafe {
            let clipboard = atom_or(dpy, &x, "CLIPBOARD", 0);
            if clipboard == 0 {
                return Err("X11: CLIPBOARD atom missing".into());
            }
            let root = (x.XDefaultRootWindow)(dpy);
            let win = (x.XCreateSimpleWindow)(dpy, root, 0, 0, 1, 1, 0, 0, 0);
            let prop = atom_or(dpy, &x, "KETRA_CLIP", 0);

            let request = |target: Atom, timeout_ms: u64| -> Result<Option<Vec<u8>>, String> {
                (x.XConvertSelection)(dpy, clipboard, target, prop, win, CURRENT_TIME);
                (x.XFlush)(dpy);
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                let mut ev = XEvent { pad: [0; 24] };
                loop {
                    if Instant::now() > deadline {
                        return Ok(None);
                    }
                    // Drain pending events; block in select() with a small
                    // timeout when idle so the deadline is honored.
                    while (x.XPending)(dpy) > 0 {
                        (x.XNextEvent)(dpy, &mut ev);
                        if ev.pad[0] as c_int != SELECTION_NOTIFY {
                            continue;
                        }
                        let notify = *(&ev as *const XEvent as *const XSelectionEvent);
                        if notify.property == 0 {
                            return Ok(None); // conversion refused
                        }
                        let mut atype: Atom = 0;
                        let mut afmt: c_int = 0;
                        let mut nitems: c_ulong = 0;
                        let mut bytes_after: c_ulong = 0;
                        let mut data: *mut u8 = std::ptr::null_mut();
                        let rc = (x.XGetWindowProperty)(
                            dpy,
                            win,
                            notify.property,
                            0,
                            (1 << 20) as c_long,
                            1, // delete the property after reading
                            ANY_PROPERTY_TYPE,
                            &mut atype,
                            &mut afmt,
                            &mut nitems,
                            &mut bytes_after,
                            &mut data,
                        );
                        if rc != 0 {
                            return Ok(None);
                        }
                        let len = match afmt {
                            8 => nitems as usize,
                            16 => nitems as usize * 2,
                            32 => nitems as usize * std::mem::size_of::<c_long>(),
                            _ => 0,
                        };
                        let bytes = if data.is_null() || len == 0 {
                            Vec::new()
                        } else {
                            std::slice::from_raw_parts(data, len).to_vec()
                        };
                        if !data.is_null() {
                            (x.XFree)(data);
                        }
                        return Ok(Some(bytes));
                    }
                    let fd = (x.ConnectionNumber)(dpy);
                    if fd >= 0 && (fd as usize) < 1024 {
                        let mut set = FdSet { bits: [0; 128] };
                        fd_set_bit(&mut set, fd);
                        let mut tv = TimeVal { tv_sec: 0, tv_usec: 50_000 };
                        let _ = select(fd + 1, &mut set, std::ptr::null_mut(), std::ptr::null_mut(), &mut tv);
                    } else {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
            };

            let gnome = atom_or(dpy, &x, super::GNOME_COPIED_FILES_TYPE, 0);
            let uri = atom_or(dpy, &x, super::URI_LIST_TYPE, 0);
            let utf8 = atom_or(dpy, &x, "UTF8_STRING", XA_STRING);
            let kde = atom_or(dpy, &x, super::KDE_CUT_TYPE, 0);

            if gnome != 0 {
                if let Some(bytes) = request(gnome, 1500)? {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if let Some(d) = super::parse_gnome_files(&text) {
                        (x.XDestroyWindow)(dpy, win);
                        (x.XCloseDisplay)(dpy);
                        return Ok(d);
                    }
                }
            }
            if uri != 0 {
                if let Some(bytes) = request(uri, 1500)? {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let paths = super::parse_uri_list(&text);
                    if !paths.is_empty() {
                        let mut mode = ClipMode::Copy;
                        if kde != 0 {
                            if let Some(bytes) = request(kde, 600)? {
                                if String::from_utf8_lossy(&bytes).trim() == "1" {
                                    mode = ClipMode::Cut;
                                }
                            }
                        }
                        (x.XDestroyWindow)(dpy, win);
                        (x.XCloseDisplay)(dpy);
                        return Ok(ClipData { paths, mode });
                    }
                }
            }
            if let Some(bytes) = request(utf8, 1200)? {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let paths = super::parse_uri_list(&text);
                if !paths.is_empty() {
                    (x.XDestroyWindow)(dpy, win);
                    (x.XCloseDisplay)(dpy);
                    return Ok(ClipData { paths, mode: ClipMode::Copy });
                }
            }
            (x.XDestroyWindow)(dpy, win);
            (x.XCloseDisplay)(dpy);
            Err("the clipboard does not contain files".into())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — the pure codecs must behave identically on every platform.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn test_uri_list_roundtrip() {
        let paths = vec![p("/tmp/a b.txt"), p("/tmp/unicode#1.png")];
        let s = encode_uri_list(&paths);
        assert!(s.contains("file:///tmp/a%20b.txt"));
        assert!(s.ends_with("\r\n"));
        let back = parse_uri_list(&s);
        assert_eq!(back, paths);
    }

    #[test]
    fn test_uri_list_ignores_comments_and_blanks() {
        let back = parse_uri_list("# comment\n\nfile:///tmp/x\r\n");
        assert_eq!(back, vec![p("/tmp/x")]);
    }

    #[test]
    fn test_gnome_files_roundtrip_both_modes() {
        let paths = vec![p("/home/u/photo.jpg"), p("/home/u/dir")];
        for mode in [ClipMode::Copy, ClipMode::Cut] {
            let s = encode_gnome_files(mode, &paths);
            let d = parse_gnome_files(&s).expect("parsed");
            assert_eq!(d.paths, paths);
            assert_eq!(d.mode, mode);
        }
        assert!(parse_gnome_files("copy\n").is_none());
        assert!(parse_gnome_files("garbage").is_none());
    }

    #[test]
    fn test_dropfiles_roundtrip() {
        let paths = vec![p("C:\\Users\\a b\\f.txt"), p("C:\\x.bin")];
        let blob = encode_dropfiles(&paths);
        // header + UTF-16 of both paths (+1 NUL each) + final NUL
        assert_eq!(blob.len(), 20 + 2 * (19 + 9 + 1));
        let back = parse_dropfiles(&blob).expect("parsed");
        assert_eq!(back[0], "C:\\Users\\a b\\f.txt");
        assert_eq!(back[1], "C:\\x.bin");
    }

    #[test]
    fn test_dropfiles_ansi_fallback() {
        // header (pFiles=20, fWide=0) + "abc\0" + "\0"
        let mut blob = Vec::new();
        blob.extend_from_slice(&20u32.to_le_bytes());
        blob.extend_from_slice(&0i32.to_le_bytes());
        blob.extend_from_slice(&0i32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(b"abc\0\0");
        let back = parse_dropfiles(&blob).expect("parsed");
        assert_eq!(back, vec!["abc".to_string()]);
    }

    #[test]
    fn test_dropfiles_garbage() {
        assert!(parse_dropfiles(&[0u8; 8]).is_none());
        assert!(parse_dropfiles(&[]).is_none());
        let mut bad = vec![0u8; 20];
        bad[..4].copy_from_slice(&999u32.to_le_bytes()); // offset beyond blob
        assert!(parse_dropfiles(&bad).is_none());
    }

    #[test]
    fn test_drop_effect() {
        let c = parse_drop_effect(&encode_drop_effect(ClipMode::Copy)).unwrap();
        let x = parse_drop_effect(&encode_drop_effect(ClipMode::Cut)).unwrap();
        assert_eq!(c, ClipMode::Copy);
        assert_eq!(x, ClipMode::Cut);
        // Explorer sets 5 (COPY|LINK) for plain copy: still copy.
        assert_eq!(parse_drop_effect(&5u32.to_le_bytes()).unwrap(), ClipMode::Copy);
    }

    #[test]
    fn test_absolutize_relative() {
        let cwd = std::env::current_dir().unwrap();
        let got = absolutize(Path::new("relative/file"));
        assert!(got.is_absolute());
        assert_eq!(got.strip_prefix(&cwd).is_ok(), true);
    }
}
