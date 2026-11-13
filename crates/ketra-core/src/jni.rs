//! Android JNI bridge — the contract between the Rust engine and the
//! Kotlin/Compose app (`android/app/src/main/java/dev/ketracopy/engine/`).
//!
//! Design (deliberately callback-free, to stay robust across threads):
//!
//! * All native calls are **blocking** and meant to run on Kotlin worker
//!   threads (`Dispatchers.IO`). They return a JSON string.
//! * Progress/verification events are appended to an in-process **event
//!   queue** as JSONL lines (the same lines `ketra --progress json`
//!   prints). The UI polls them with `Engine.nativePollEvents()`.
//! * Only three JNI entry points are used (`NewStringUTF`,
//!   `GetStringUTFChars`, `ReleaseStringUTFChars`) through a hand-declared
//!   `JNINativeInterface` table — no external crates, consistent with the
//!   engine's zero-dependency rule.
//!
//! Kotlin contract (frozen):
//!
//! ```kotlin
//! package dev.ketracopy.engine
//! class Engine {
//!     companion object {
//!         init { System.loadLibrary("ketra_core") }
//!         @JvmStatic external fun nativeVersion(): String
//!         @JvmStatic external fun nativeHashFile(path: String, algo: String): String
//!         @JvmStatic external fun nativeEngineStart(optsJson: String): Long
//!         @JvmStatic external fun nativeEngineRun(handle: Long, srcsJson: String, dst: String, move: Boolean): String
//!         @JvmStatic external fun nativeEngineCancel(handle: Long)
//!         @JvmStatic external fun nativeEngineFree(handle: Long)
//!         @JvmStatic external fun nativePollEvents(): String
//!     }
//! }
//! object Lan {
//!     @JvmStatic external fun nativePeersBrowse(timeoutMs: Int): String
//!     @JvmStatic external fun nativeReceiverStart(port: Int, alias: String, destDir: String, autoAccept: Boolean, verify: Boolean): String
//!     @JvmStatic external fun nativeReceiverStop()
//!     @JvmStatic external fun nativeSend(host: String, port: Int, srcsJson: String, parallel: Int, verify: Boolean): String
//! }
//! ```

use crate::engine::{ConflictDecision, Engine};
use crate::json::{self, Json};
use crate::lan::{self, SendConfig};
use crate::options::{hash_file, Options, OnConflict, VerifyMode};
use crate::progress::{event_line, Event};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Hand-declared JNI (stable ABI since JNI 1.2; Android uses the same table).
// ---------------------------------------------------------------------------

type JString = *mut std::os::raw::c_void;

#[repr(C)]
#[allow(non_snake_case)]
pub struct JniTable {
    /// Function-table slots 0..=166 (reserved + everything up to
    /// `NewStringUTF`), as opaque pointers.
    reserved: [Option<unsafe extern "system" fn()>; 167],
    /// slot 167
    NewStringUTF: unsafe extern "system" fn(
        env: *mut *mut JniTable,
        bytes: *const std::os::raw::c_char,
    ) -> JString,
    /// slot 168
    GetStringUTFLength: Option<unsafe extern "system" fn()>,
    /// slot 169
    GetStringUTFChars: unsafe extern "system" fn(
        env: *mut *mut JniTable,
        str_: JString,
        is_copy: *mut std::os::raw::c_char,
    ) -> *const std::os::raw::c_char,
    /// slot 170
    ReleaseStringUTFChars: unsafe extern "system" fn(
        env: *mut *mut JniTable,
        str_: JString,
        chars: *const std::os::raw::c_char,
    ),
}

/// `JNIEnv` in C terms (pointer to the table pointer).
type JniEnv = *mut *mut JniTable;

unsafe fn jstr_to_string(env: JniEnv, s: JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let table = unsafe { &**env };
    let chars = unsafe { (table.GetStringUTFChars)(env, s, std::ptr::null_mut()) };
    if chars.is_null() {
        return None;
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(chars) };
    let out = bytes.to_string_lossy().into_owned();
    unsafe { (table.ReleaseStringUTFChars)(env, s, chars) };
    Some(out)
}

unsafe fn string_to_jstr(env: JniEnv, s: &str) -> JString {
    let table = unsafe { &**env };
    let mut bytes = s.as_bytes().to_vec();
    // Modified UTF-8 needs embedded NULs re-encoded and a terminator.
    if bytes.contains(&0) {
        let mut fixed = Vec::with_capacity(bytes.len() + 2);
        for b in &bytes {
            if *b == 0 {
                fixed.extend_from_slice(&[0xC0, 0x80]);
            } else {
                fixed.push(*b);
            }
        }
        bytes = fixed;
    }
    bytes.push(0);
    unsafe { (table.NewStringUTF)(env, bytes.as_ptr() as *const std::os::raw::c_char) }
}

// ---------------------------------------------------------------------------
// Event queue (JSONL lines, polled by the UI thread)
// ---------------------------------------------------------------------------

fn events() -> &'static Mutex<VecDeque<String>> {
    static Q: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn push_event(line: String) {
    let mut q = events().lock().unwrap();
    if q.len() > 4096 {
        q.pop_front(); // bounded, drop-oldest
    }
    q.push_back(line);
}

fn push(e: &Event) {
    push_event(event_line(e));
}

// ---------------------------------------------------------------------------
// Engine handles
// ---------------------------------------------------------------------------

struct EngineEntry {
    opts: Options,
    cancel: Arc<AtomicBool>,
}

fn engines() -> &'static Mutex<BTreeMap<i64, EngineEntry>> {
    static E: OnceLock<Mutex<BTreeMap<i64, EngineEntry>>> = OnceLock::new();
    E.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn next_handle() -> i64 {
    static N: AtomicI64 = AtomicI64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Parse an options JSON object (subset of the CLI flags).
fn opts_from_json(v: &Json) -> Options {
    let mut o = Options::default();
    match v.get("verify").and_then(|j| j.as_str()) {
        Some("off") => o.verify = VerifyMode::Off,
        Some("manifest") => o.verify = VerifyMode::Manifest,
        Some(_) => o.verify = VerifyMode::Inline,
        None => {}
    }
    match v.get("conflict").and_then(|j| j.as_str()) {
        Some("skip") => o.conflict = OnConflict::Skip,
        Some("overwrite") => o.conflict = OnConflict::Overwrite,
        Some("rename") => o.conflict = OnConflict::Rename,
        Some("newer") => o.conflict = OnConflict::KeepNewer,
        _ => o.conflict = OnConflict::Overwrite, // non-interactive default
    }
    if let Some(limit) = v.get("limit_bps").and_then(|j| j.as_u64()) {
        o.speed_limit_bps = Some(limit);
    }
    if let Some(n) = v.get("net_streams").and_then(|j| j.as_u64()) {
        o.net_streams = n as u32;
    }
    if let Some(b) = v.get("buffer").and_then(|j| j.as_u64()) {
        o.buffer = b as usize;
    }
    if let Some(b) = v.get("flatten").and_then(|j| j.as_bool()) {
        o.flatten = b;
    }
    if let Some(b) = v.get("resume").and_then(|j| j.as_bool()) {
        o.resume = b;
    }
    if let Some(b) = v.get("preserve_mtime").and_then(|j| j.as_bool()) {
        o.preserve_mtime = b;
    }
    if let Some(b) = v.get("preserve_perms").and_then(|j| j.as_bool()) {
        o.preserve_perms = b;
    }
    if let Some(b) = v.get("sparse").and_then(|j| j.as_bool()) {
        o.use_sparse = b;
    }
    if let Some(list) = v.get("include") {
        if let Json::Arr(a) = list {
            o.includes = a.iter().filter_map(|j| j.as_str().map(str::to_string)).collect();
        }
    }
    if let Some(list) = v.get("exclude") {
        if let Json::Arr(a) = list {
            o.excludes = a.iter().filter_map(|j| j.as_str().map(str::to_string)).collect();
        }
    }
    o
}

fn summary_json(s: &crate::engine::Summary, err: Option<&str>) -> String {
    format!(
        "{{\"ok\":{},\"files_total\":{},\"files_copied\":{},\"files_skipped\":{},\"files_failed\":{},\"bytes_total\":{},\"bytes_copied\":{},\"elapsed_secs\":{:.3}{}{}}}",
        err.is_none(),
        s.files_total,
        s.files_copied,
        s.files_skipped,
        s.files_failed,
        s.bytes_total,
        s.bytes_copied,
        s.duration.as_secs_f64(),
        if s.manifest.is_some() { ",\"manifest\":" } else { "" },
        if s.manifest.is_some() {
            json::quote(&s.manifest.as_ref().unwrap().display().to_string())
        } else {
            String::new()
        },
    )
    .replace("}}", "}") // keep the optional manifest field tidy
}

fn error_json(msg: &str) -> String {
    format!("{{\"ok\":false,\"error\":{}}}", json::quote(msg))
}

// ---------------------------------------------------------------------------
// exported JNI functions
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeVersion(
    env: JniEnv,
    _class: JString,
) -> JString {
    string_to_jstr(env, &format!("ketra-core {}", crate::VERSION))
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeHashFile(
    env: JniEnv,
    _class: JString,
    path: JString,
    algo: JString,
) -> JString {
    let Some(path) = jstr_to_string(env, path) else {
        return string_to_jstr(env, &error_json("null path"));
    };
    let algo = jstr_to_string(env, algo).unwrap_or_else(|| "sha256".into());
    let cancel = AtomicBool::new(false);
    match hash_file(&PathBuf::from(&path), &algo, 1024 * 1024, &cancel) {
        Ok(hex) => string_to_jstr(env, &format!("{{\"ok\":true,\"algo\":\"{algo}\",\"digest\":\"{hex}\"}}")),
        Err(e) => string_to_jstr(env, &error_json(&e.to_string())),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeEngineStart(
    env: JniEnv,
    _class: JString,
    opts_json: JString,
) -> i64 {
    let text = jstr_to_string(env, opts_json).unwrap_or_else(|| "{}".into());
    let opts = match json::parse(&text).ok().map(|v| opts_from_json(&v)) {
        Some(o) => o,
        None => Options::default(),
    };
    let h = next_handle();
    engines().lock().unwrap().insert(
        h,
        EngineEntry { opts, cancel: Arc::new(AtomicBool::new(false)) },
    );
    h
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeEngineRun(
    env: JniEnv,
    _class: JString,
    handle: i64,
    srcs_json: JString,
    dst: JString,
    move_files: std::os::raw::c_uchar,
) -> JString {
    let Some(entry) = engines().lock().unwrap().get(&handle).cloned_entry() else {
        return string_to_jstr(env, &error_json("unknown engine handle"));
    };
    let Some(srcs_text) = jstr_to_string(env, srcs_json) else {
        return string_to_jstr(env, &error_json("null sources"));
    };
    let srcs: Vec<PathBuf> = match json::parse(&srcs_text).ok().and_then(|v| match v {
        Json::Arr(a) => Some(
            a.into_iter()
                .filter_map(|j| j.as_str().map(str::to_string))
                .map(PathBuf::from)
                .collect(),
        ),
        _ => None,
    }) {
        Some(s) if !s.is_empty() => s,
        _ => return string_to_jstr(env, &error_json("sources must be a non-empty JSON array")),
    };
    let Some(dst) = jstr_to_string(env, dst) else {
        return string_to_jstr(env, &error_json("null destination"));
    };
    let dst = PathBuf::from(dst);
    let moving = move_files != 0;

    let cancel = entry.cancel.clone();
    let mut on_event = |e: &Event| push(e);
    let mut on_conflict = |_t: &crate::plan::FileTask| -> ConflictDecision {
        // Non-interactive: mirror the engine default chosen at start.
        ConflictDecision::Overwrite
    };
    let opts = entry.opts;
    let mut engine = Engine::new(&opts, &cancel, &mut on_event, &mut on_conflict);
    match engine.copy(&srcs, &dst, moving) {
        Ok(sum) => string_to_jstr(env, &summary_json(&sum, None)),
        Err(e) => {
            push(&Event::Message(format!("engine error: {e}")));
            string_to_jstr(env, &error_json(&e.to_string()))
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeEngineCancel(
    _env: JniEnv,
    _class: JString,
    handle: i64,
) {
    if let Some(e) = engines().lock().unwrap().get(&handle) {
        e.cancel.store(true, Ordering::Relaxed);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativeEngineFree(
    _env: JniEnv,
    _class: JString,
    handle: i64,
) {
    engines().lock().unwrap().remove(&handle);
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Engine_nativePollEvents(
    env: JniEnv,
    _class: JString,
) -> JString {
    let drained: Vec<String> = {
        let mut q = events().lock().unwrap();
        q.drain(..).collect()
    };
    let items: Vec<String> = drained.iter().map(|l| json::quote(l)).collect();
    string_to_jstr(env, &format!("[{}]", items.join(",")))
}

// ---------------------------------------------------------------------------
// LAN
// ---------------------------------------------------------------------------

fn receiver_slot() -> &'static Mutex<Option<Arc<lan::Receiver>>> {
    static R: OnceLock<Mutex<Option<Arc<lan::Receiver>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

fn mdns_slot() -> &'static Mutex<Option<Arc<lan::Mdns>>> {
    static M: OnceLock<Mutex<Option<Arc<lan::Mdns>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(None))
}

fn peer_json(p: &lan::Discovered) -> String {
    let alias = p.alias();
    format!(
        "{{\"alias\":{},\"ip\":{},\"port\":{},\"device\":{},\"fingerprint\":{},\"ketra\":{},\"service\":{}}}",
        json::quote(&alias),
        json::quote(&p.ip),
        p.port,
        json::quote(p.txt.get("deviceModel").map(String::as_str).unwrap_or("")),
        json::quote(p.fingerprint().as_deref().unwrap_or("")),
        p.is_ketra(),
        json::quote(&p.service),
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Lan_nativePeersBrowse(
    env: JniEnv,
    _class: JString,
    timeout_ms: i32,
) -> JString {
    let timeout = Duration::from_millis(timeout_ms.max(100) as u64);
    let Some(mdns) = mdns_slot().lock().unwrap().clone() else {
        return string_to_jstr(env, "[]");
    };
    let peers: Vec<String> = lan::discover(Some(&mdns), timeout)
        .iter()
        .map(peer_json)
        .collect();
    string_to_jstr(env, &format!("[{}]", peers.join(",")))
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Lan_nativeReceiverStart(
    env: JniEnv,
    _class: JString,
    port: i32,
    alias: JString,
    dest_dir: JString,
    auto_accept: std::os::raw::c_uchar,
    verify: std::os::raw::c_uchar,
) -> JString {
    // stop a previous receiver first
    if let Some(old) = receiver_slot().lock().unwrap().take() {
        old.stop();
    }
    let alias = jstr_to_string(env, alias).unwrap_or_else(|| "Android".into());
    let Some(dir) = jstr_to_string(env, dest_dir) else {
        return string_to_jstr(env, &error_json("null destDir"));
    };
    let cfg = lan::ReceiverConfig {
        alias,
        dest_dir: PathBuf::from(dir),
        port: port.max(0) as u16,
        auto_accept: auto_accept != 0,
        verify_sha256: verify != 0,
        announce: true,
        max_streams: 4,
    };
    let hooks = lan::ReceiverHooks {
        on_request: Box::new(|p: &crate::lan::protocol::PrepareUpload| {
            push_event(format!(
                "{{\"event\":\"lan_request\",\"alias\":{},\"files\":{}}}",
                json::quote(&p.info.alias),
                p.files.len()
            ));
            // Non-interactive accept; the UI can stop the receiver to refuse.
            true
        }),
        on_file_done: Box::new(|f: &lan::ReceivedFile| {
            push_event(format!(
                "{{\"event\":\"lan_received\",\"file\":{},\"size\":{},\"verified\":{}}}",
                json::quote(&f.file_name),
                f.size,
                f.verified
            ));
        }),
        on_progress: Box::new(|name: &str, delta: u64, size: u64| {
            push_event(format!(
                "{{\"event\":\"lan_progress\",\"file\":{},\"delta\":{delta},\"size\":{size}}}",
                json::quote(name)
            ));
        }),
        on_message: Box::new(|m: String| {
            push_event(format!(
                "{{\"event\":\"lan_message\",\"text\":{}}}",
                json::quote(&m)
            ));
        }),
    };
    match lan::Receiver::start(cfg, hooks) {
        Ok(r) => {
            let port = r.port();
            *receiver_slot().lock().unwrap() = Some(r);
            string_to_jstr(env, &format!("{{\"ok\":true,\"port\":{port}}}"))
        }
        Err(e) => string_to_jstr(env, &error_json(&e)),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Lan_nativeReceiverStop(
    _env: JniEnv,
    _class: JString,
) {
    if let Some(r) = receiver_slot().lock().unwrap().take() {
        r.stop();
    }
    if let Some(m) = mdns_slot().lock().unwrap().take() {
        m.withdraw();
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Lan_nativeSend(
    env: JniEnv,
    _class: JString,
    host: JString,
    port: i32,
    srcs_json: JString,
    parallel: i32,
    verify: std::os::raw::c_uchar,
) -> JString {
    let Some(host) = jstr_to_string(env, host) else {
        return string_to_jstr(env, &error_json("null host"));
    };
    let port = if port > 0 { port as u16 } else { lan::LAN_PORT };
    let Some(srcs_text) = jstr_to_string(env, srcs_json) else {
        return string_to_jstr(env, &error_json("null sources"));
    };
    let srcs: Vec<PathBuf> = match json::parse(&srcs_text).ok().and_then(|v| match v {
        Json::Arr(a) => Some(
            a.into_iter()
                .filter_map(|j| j.as_str().map(str::to_string))
                .map(PathBuf::from)
                .collect(),
        ),
        _ => None,
    }) {
        Some(s) if !s.is_empty() => s,
        _ => return string_to_jstr(env, &error_json("sources must be a non-empty JSON array")),
    };
    let cfg = SendConfig {
        dest_dir_hint: String::new(),
        parallel: parallel.max(1) as u32,
        verify: verify != 0,
        limit_bps: None,
        timeout: Duration::from_secs(3600),
        ketra_mode: true,
    };
    let cancel = AtomicBool::new(false);
    let addr = format!("{host}:{port}");
    let result = lan::send_files(&addr, &srcs, &cfg, &cancel, &mut |name, delta, size| {
        push_event(format!(
            "{{\"event\":\"lan_progress\",\"file\":{},\"delta\":{delta},\"size\":{size}}}",
            json::quote(name)
        ));
    });
    match result {
        Ok(report) => {
            let files: Vec<String> = report
                .files
                .iter()
                .map(|(n, st, b)| {
                    format!(
                        "{{\"name\":{},\"status\":{},\"bytes\":{b}}}",
                        json::quote(n),
                        json::quote(st)
                    )
                })
                .collect();
            string_to_jstr(
                env,
                &format!(
                    "{{\"ok\":true,\"files_total\":{},\"files_sent\":{},\"files_verified\":{},\"files_failed\":{},\"bytes\":{},\"streams\":{},\"files\":[{}]}}",
                    report.files_total,
                    report.files_sent,
                    report.files_verified,
                    report.files_failed,
                    report.bytes,
                    report.max_streams_used,
                    files.join(",")
                ),
            )
        }
        Err(e) => string_to_jstr(env, &error_json(&e)),
    }
}

/// Lazily spawn the shared mDNS socket (called on first `nativePeersBrowse`).
#[no_mangle]
pub unsafe extern "system" fn Java_dev_ketracopy_engine_Lan_nativeInit(
    env: JniEnv,
    _class: JString,
) -> JString {
    let mut slot = mdns_slot().lock().unwrap();
    if slot.is_none() {
        *slot = lan::Mdns::spawn();
    }
    string_to_jstr(
        env,
        &format!("{{\"ok\":{},\"mdns\":{}}}", slot.is_some(), slot.is_some()),
    )
}

// The Android app calls `Lan.nativeInit` during startup; the counter keeps
// this translation unit honest about unused state on non-Android builds.
#[allow(dead_code)]
fn _touch() -> u64 {
    static C: AtomicU64 = AtomicU64::new(0);
    C.fetch_add(1, Ordering::Relaxed)
}
