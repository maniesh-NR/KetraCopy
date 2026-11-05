//! copy/move implementation: args → options → engine, with progress view,
//! Ctrl+C handling, conflict prompting and optional JSON report.
//! `clip paste` reuses this module end-to-end.

use crate::args::{CommonArgs, ConflictArg, ProgressArg, VerifyArg};
use crate::sig;
use crate::view::View;
use ketra_core::json::Json;
use ketra_core::options::{OnConflict, Options, VerifyMode};
use ketra_core::progress::Event;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub struct Outcome {
    pub code: i32,
}

/// Options built from persistent settings + CLI overrides.
fn build_options(args: &CommonArgs) -> Options {
    let mut opts: Options = Options::default();

    // Persistent settings (ketracopy.json), applied first so CLI wins.
    {
    let path = ketra_core::settings::Settings::default_path();
    if path.exists() {
        if let Ok(s) = ketra_core::settings::Settings::load(&path) {
            s.apply_to(&mut opts);
        }
    }
}

    opts.verify = match args.verify {
        VerifyArg::Off => VerifyMode::Off,
        VerifyArg::Inline => VerifyMode::Inline,
        VerifyArg::Manifest => VerifyMode::Manifest,
    };
    if args.limit_bps > 0 {
        opts.speed_limit_bps = Some(args.limit_bps);
    } else if args.limit_bps == 0 && args.yes {
        // `-y` does not clear a configured limit; only an explicit --limit does.
    }
    if args.net_streams > 0 {
        opts.net_streams = args.net_streams as u32;
    }
    opts.conflict = match args.conflict {
        ConflictArg::Overwrite => OnConflict::Overwrite,
        ConflictArg::Newer => OnConflict::KeepNewer,
        ConflictArg::Skip => OnConflict::Skip,
        ConflictArg::Rename => OnConflict::Rename,
        ConflictArg::Ask => OnConflict::Ask,
    };
    opts.includes = args.include.clone();
    opts.excludes = args.exclude.clone();
    opts.flatten = args.flatten;
    opts.preserve_mtime = args.preserve_times;
    opts.preserve_perms = args.preserve_attrs;
    opts.use_sparse = args.sparse;
    opts.use_clone = args.clone;
    opts.resume = args.resume;
    opts.dry_run = args.dry_run;
    opts
}

/// Run a copy or move. `move_files` removes sources after a verified copy.
pub fn run(kind: TransferKind, sources: Vec<PathBuf>, args: &CommonArgs) -> Outcome {
    let mut opts = build_options(args);
    opts.manifest_path = None;

    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    sig::install();

    let (tx, rx) = mpsc::channel::<Event>();

    // Renderer thread: plain terminal UI / JSONL / quiet.
    let plain = match args.progress {
        ProgressArg::Quiet => false,
        ProgressArg::Json => false,
        ProgressArg::Plain => true,
        ProgressArg::Auto => std::io::stdout().is_terminal(),
    };
    let quiet = args.progress == ProgressArg::Quiet;
    let json_mode = args.progress == ProgressArg::Json;

    let cancel2 = cancel.clone();
    let end_tx = tx.clone();
    let render = std::thread::spawn(move || {
        let mut view = View::new(plain, quiet);
        loop {
            if sig::was_interrupted() {
                cancel2.store(true, Ordering::SeqCst);
            }
            if json_mode {
                let mut finished = false;
                while let Ok(ev) = rx.try_recv() {
                    let end = matches!(ev, Event::Finished { .. } | Event::Canceled { .. });
                    crate::view::json_forward(&ev);
                    if end {
                        finished = true;
                    }
                }
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                if finished {
                    return;
                }
            } else if view.pump(&rx) {
                return;
            }
            if sig::was_interrupted() {
                cancel2.store(true, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    });

    // Non-interactive conflict resolution: Ask becomes Overwrite with a note.
    let mut interactive = args.conflict == ConflictArg::Ask && !args.yes && std::io::stdin().is_terminal();
    if args.conflict == ConflictArg::Ask && !interactive {
        eprintln!("ketra: --conflict ask requested but stdin is not a terminal; overwriting");
        interactive = false;
    }

    let mut errors: Vec<(String, String)> = Vec::new();
    let events_tx = tx.clone();
    let mut on_event = |ev: &Event| {
        if let Event::Error { path, message } = ev {
            errors.push((path.clone(), message.clone()));
        }
        let _ = events_tx.send(ev.clone());
    };
    let mut on_conflict = |task: &ketra_core::plan::FileTask| {
        if interactive {
            prompt_conflict(&task.dst.display().to_string(), task.size)
        } else {
            ketra_core::engine::ConflictDecision::Overwrite
        }
    };
    let mut engine = ketra_core::engine::Engine::new(&opts, &cancel, &mut on_event, &mut on_conflict);

    let result = match kind {
        TransferKind::Copy => engine.copy(&sources, &args.dest, false),
        TransferKind::Move => engine.copy(&sources, &args.dest, true),
        TransferKind::Test(ref algo) => engine.test(&sources, algo),
    };

    // Early-return paths (planning errors, cancel before the first event) do
    // not emit a terminal event; nudge the renderer so it never deadlocks.
    if result.is_err() {
        let _ = end_tx.send(Event::Canceled { files_copied: 0, bytes_copied: 0 });
    }
    drop(tx);
    let _ = render.join().unwrap_or(());

    if let Some(path) = &args.report {
        write_report(path, &result, &errors);
    }

    match result {
        Ok(summary) => {
            if summary.files_failed > 0 {
                Outcome { code: 1 }
            } else {
                Outcome { code: 0 }
            }
        }
        Err(e) if e.is_canceled() => Outcome { code: 2 },
        Err(e) => {
            eprintln!("ketra: {e}");
            Outcome { code: 1 }
        }
    }
}

#[derive(Debug, Clone)]
pub enum TransferKind {
    Copy,
    Move,
    /// Read-test the sources through the given hash algorithm.
    Test(String),
}

/// Interactive conflict prompt: [o]verwrite [s]kip [r]ename, with
/// all-of-them shortcuts. Falls back to Overwrite when stdin is closed.
fn prompt_conflict(dst: &str, src_size: u64) -> ketra_core::engine::ConflictDecision {
    use std::io::Write as _;
    static MEMO: std::sync::Mutex<Option<ketra_core::engine::ConflictDecision>> = std::sync::Mutex::new(None);
    {
        let m = MEMO.lock().unwrap();
        if let Some(d) = *m {
            return d;
        }
    }
    loop {
        print!(
            "\nconflict: {dst} exists ({src_size} bytes to transfer)\n  [o]verwrite [s]kip [r]ename [O]verwrite-all [S]kip-all [c]ancel > "
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return ketra_core::engine::ConflictDecision::Overwrite;
        }
        // Case matters: lowercase = this file, UPPERCASE = all remaining.
        let d = match line.trim() {
            "o" => ketra_core::engine::ConflictDecision::Overwrite,
            "O" | "oa" | "OA" => ketra_core::engine::ConflictDecision::OverwriteAll,
            "s" => ketra_core::engine::ConflictDecision::Skip,
            "S" | "sa" | "SA" => ketra_core::engine::ConflictDecision::SkipAll,
            "r" => ketra_core::engine::ConflictDecision::Rename,
            "c" => ketra_core::engine::ConflictDecision::Cancel,
            _ => continue,
        };
        match d {
            ketra_core::engine::ConflictDecision::OverwriteAll | ketra_core::engine::ConflictDecision::SkipAll => {
                if let Ok(mut m) = MEMO.lock() {
                    *m = Some(d);
                }
            }
            ketra_core::engine::ConflictDecision::Cancel => {
                // Cancel is handled by the engine (returns Error::Canceled).
            }
            _ => {}
        }
        return d;
    }
}

fn write_report(path: &std::path::Path, result: &Result<ketra_core::engine::Summary, ketra_core::error::Error>, errors: &[(String, String)]) {
    use std::collections::BTreeMap;
    let mut files_ok = 0u32;
    let mut files_skipped = 0u32;
    let mut files_failed = 0u32;
    let mut bytes = 0u64;
    let mut canceled = false;
    if let Ok(s) = result {
        files_ok = s.files_copied;
        files_skipped = s.files_skipped;
        files_failed = s.files_failed;
        bytes = s.bytes_copied;
    } else if matches!(result, Err(e) if e.is_canceled()) {
        canceled = true;
    }
    let err_list: Vec<Json> = errors
        .iter()
        .map(|(p, m)| {
            let mut o = BTreeMap::new();
            o.insert("src".to_string(), Json::Str(p.clone()));
            o.insert("message".to_string(), Json::Str(m.clone()));
            Json::Obj(o)
        })
        .collect();
    let mut root = BTreeMap::new();
    root.insert("files_ok".to_string(), Json::Num(files_ok as f64));
    root.insert("files_skipped".to_string(), Json::Num(files_skipped as f64));
    root.insert("files_failed".to_string(), Json::Num(files_failed as f64));
    root.insert("bytes".to_string(), Json::Num(bytes as f64));
    root.insert("canceled".to_string(), Json::Bool(canceled));
    root.insert("errors".to_string(), Json::Arr(err_list));
    let v = Json::Obj(root);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let _ = std::fs::write(path, json_compact(&v));
}

/// Minimal JSON serializer (the core crate has no public pretty-printer).
fn json_compact(v: &Json) -> String {
    use ketra_core::json::quote;
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => format!("{n}"),
        Json::Str(s) => quote(s),
        Json::Arr(items) => {
            let inner: Vec<String> = items.iter().map(json_compact).collect();
            format!("[{}]", inner.join(","))
        }
        Json::Obj(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, val)| format!("{}:{}", quote(k), json_compact(val)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}
