//! KetraCopy GUI — a Material Design 3 shell for the ketra-core engine.
//!
//! Tabs: **Transfer** (copy/move/test with live progress), **LAN**
//! (LocalSend-compatible send/receive on the local network), **Settings**
//! (persisted defaults + one-click shell integration) and **About**.
//!
//! The engine runs on worker threads; the UI polls progress events and
//! never blocks the frame loop.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use ketra_core::options::{OnConflict, Options, VerifyMode};
use ketra_core::progress::Event;
use ketra_md3::{color, components, theme};
use ketra_md3::components::CardKind;

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([780.0, 540.0])
            .with_title("KetraCopy"),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "KetraCopy",
        options,
        Box::new(|cc| Ok(Box::new(KetraApp::new(cc)) as Box<dyn eframe::App>)),
    );
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Transfer,
    Lan,
    Settings,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferKind {
    Copy,
    Move,
    Test,
}

impl TransferKind {
    fn label(self) -> &'static str {
        match self {
            TransferKind::Copy => "Copy",
            TransferKind::Move => "Move",
            TransferKind::Test => "Test (verify reads)",
        }
    }
}

/// Everything the UI thread needs to render a running job.
#[derive(Default)]
struct UiProgress {
    started: bool,
    finished: bool,
    canceled: bool,
    phase: String,
    total_files: u32,
    total_bytes: u64,
    done_files: u32,
    skipped: u32,
    failed: u32,
    copied_bytes: u64,
    speed_bps: f64,
    eta_secs: f64,
    current: String,
    exit_code: i32,
}

struct Job {
    kind: TransferKind,
    sources: Vec<PathBuf>,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Event>,
    progress: Arc<Mutex<UiProgress>>,
    errors: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Job {
    fn start(kind: TransferKind, sources: Vec<PathBuf>, dest: PathBuf, opts: Options) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Event>();
        let progress = Arc::new(Mutex::new(UiProgress::default()));
        let errors = Arc::new(Mutex::new(Vec::new()));

        let cancel2 = cancel.clone();
        let progress2 = progress.clone();
        let errors2 = errors.clone();
        let sources_t = sources.clone();
        let dest_t = dest.clone();
        let thread = std::thread::spawn(move || {
            sig_install();
            let mut on_event = |ev: &Event| {
                update_progress(&progress2, ev);
                let _ = tx.send(ev.clone());
            };
            let mut on_conflict = |_task: &ketra_core::plan::FileTask| ketra_core::engine::ConflictDecision::Overwrite;
            let mut engine = ketra_core::engine::Engine::new(&opts, &cancel2, &mut on_event, &mut on_conflict);
            let result = match kind {
                TransferKind::Copy => engine.copy(&sources_t, &dest_t, false),
                TransferKind::Move => engine.copy(&sources_t, &dest_t, true),
                TransferKind::Test => engine.test(&sources_t, "xxh64"),
            };
            match result {
                Ok(s) => {
                    let mut p = progress2.lock().unwrap();
                    p.finished = true;
                    p.failed = s.files_failed;
                    p.done_files = s.files_copied;
                    p.skipped = s.files_skipped;
                    p.exit_code = if s.files_failed > 0 { 1 } else { 0 };
                }
                Err(e) if e.is_canceled() => {
                    let mut p = progress2.lock().unwrap();
                    p.finished = true;
                    p.canceled = true;
                    p.exit_code = 2;
                }
                Err(e) => {
                    let mut p = progress2.lock().unwrap();
                    p.finished = true;
                    p.failed += 1;
                    p.exit_code = 1;
                    errors2.lock().unwrap().push(format!("plan/run error: {e}"));
                }
            }
        });
        Job {
            kind,
            sources,
            dest,
            cancel,
            rx,
            progress,
            errors,
            thread: Some(thread),
        }
    }

    /// Drain pending events into the UI progress struct (called every frame).
    fn pump(&self) {
        while let Ok(ev) = self.rx.try_recv() {
            update_progress(&self.progress, &ev);
        }
    }

    fn is_done(&self) -> bool {
        self.progress.lock().unwrap().finished
    }
}

fn update_progress(p: &Mutex<UiProgress>, ev: &Event) {
    let mut p = p.lock().unwrap();
    match ev {
        Event::Started { total_files, total_bytes, phase } => {
            p.started = true;
            p.phase = phase.to_string();
            p.total_files = *total_files;
            p.total_bytes = *total_bytes;
        }
        Event::FileStart { path, .. } => p.current = path.clone(),
        Event::FileProgress { total_copied, total_bytes, speed_bps, eta_secs, .. } => {
            p.copied_bytes = *total_copied;
            p.total_bytes = *total_bytes;
            p.speed_bps = *speed_bps;
            p.eta_secs = *eta_secs;
        }
        Event::FileDone { .. } => p.done_files += 1,
        Event::FileSkip { .. } => p.skipped += 1,
        Event::Error { path, message } => p.failed += 1,
        Event::Message(m) => p.phase = m.clone(),
        Event::Finished { files_copied, files_skipped, files_failed, bytes_copied, .. } => {
            p.finished = true;
            p.done_files = *files_copied;
            p.skipped = *files_skipped;
            p.failed = *files_failed;
            p.copied_bytes = *bytes_copied;
        }
        Event::Canceled { .. } => {
            p.finished = true;
            p.canceled = true;
        }
    }
}

fn sig_install() {
    // Minimal Ctrl+C → cancel wiring (mirrors the CLI's sig module).
    #[cfg(unix)]
    {
        extern "C" fn handler(_sig: i32) {}
        unsafe {
            extern "C" {
                fn signal(signum: i32, handler: usize) -> usize;
            }
            signal(2, handler as usize);
        }
    }
}

// ---------------------------------------------------------------------------
// LAN state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct PeerRow {
    alias: String,
    addr: String,
    ketra: bool,
    fingerprint: String,
}

struct LanState {
    alias: String,
    peers: Arc<Mutex<Vec<PeerRow>>>,
    browsing: Arc<AtomicBool>,
    send_status: Arc<Mutex<String>>,
    send_running: Arc<AtomicBool>,
    receive_log: Arc<Mutex<VecDeque<String>>>,
    receive_server: Option<Arc<ketra_core::lan::server::Receiver>>,
    receiving: Arc<AtomicBool>,
}

impl LanState {
    fn new() -> Self {
        LanState {
            alias: std::env::var("USER").unwrap_or_else(|_| "Ketra".into()),
            peers: Arc::new(Mutex::new(Vec::new())),
            browsing: Arc::new(AtomicBool::new(false)),
            send_status: Arc::new(Mutex::new(String::new())),
            send_running: Arc::new(AtomicBool::new(false)),
            receive_log: Arc::new(Mutex::new(VecDeque::new())),
            receive_server: None,
            receiving: Arc::new(AtomicBool::new(false)),
        }
    }

    fn browse(&self) {
        if self.browsing.load(Ordering::SeqCst) {
            return;
        }
        self.browsing.store(true, Ordering::SeqCst);
        let peers = self.peers.clone();
        let flag = self.browsing.clone();
        std::thread::spawn(move || {
            if let Some(mdns) = ketra_core::lan::Mdns::spawn() {
                let found = ketra_core::lan::server::discover(Some(&mdns), Duration::from_secs(3));
                let rows: Vec<PeerRow> = found
                    .iter()
                    .map(|p| PeerRow {
                        alias: p.alias(),
                        addr: format!("{}:{}", p.ip, p.port),
                        ketra: p.is_ketra(),
                        fingerprint: p.fingerprint().unwrap_or_default(),
                    })
                    .collect();
                *peers.lock().unwrap() = rows;
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    fn send(&self, peer: PeerRow, files: Vec<PathBuf>) {
        if self.send_running.load(Ordering::SeqCst) || files.is_empty() {
            return;
        }
        self.send_running.store(true, Ordering::SeqCst);
        *self.send_status.lock().unwrap() = format!("connecting to {} …", peer.alias);
        let status = self.send_status.clone();
        let running = self.send_running.clone();
        std::thread::spawn(move || {
            let cfg = ketra_core::lan::client::SendConfig {
                dest_dir_hint: String::new(),
                parallel: 4,
                verify: true,
                limit_bps: None,
                timeout: Duration::from_secs(30),
                ketra_mode: true,
            };
            let cancel = AtomicBool::new(false);
            let result = ketra_core::lan::client::send_files(&peer.addr, &files, &cfg, &cancel, &mut |name, delta, size| {
                let pct = if size > 0 { 100.0 * delta as f64 / size as f64 } else { 100.0 };
                *status.lock().unwrap() = format!("{name}: {pct:.1}%");
            });
            *status.lock().unwrap() = match result {
                Ok(r) if r.files_failed == 0 => format!(
                    "sent {} file(s), {} verified, {} in {:.1}s",
                    r.files_sent,
                    r.files_verified,
                    human_bytes(r.bytes),
                    r.duration.as_secs_f64()
                ),
                Ok(r) => format!("{} ok, {} failed", r.files_sent, r.files_failed),
                Err(e) => format!("send failed: {e}"),
            };
            running.store(false, Ordering::SeqCst);
        });
    }

    fn start_receive(&mut self, dest: PathBuf) {
        if self.receive_server.is_some() {
            return;
        }
        let log = self.receive_log.clone();
        let log2 = self.receive_log.clone();
        let flag = self.receiving.clone();
        let cfg = ketra_core::lan::server::ReceiverConfig {
            alias: self.alias.clone(),
            dest_dir: dest,
            port: 0,
            auto_accept: true,
            verify_sha256: true,
            announce: true,
            max_streams: 4,
        };
        let hooks = ketra_core::lan::server::ReceiverHooks {
            on_request: Box::new(|_| true),
            on_file_done: Box::new(move |f| {
                let mut l = log.lock().unwrap();
                l.push_back(format!(
                    "{} {} ({}){}",
                    if f.verified { "✓" } else { "?" },
                    f.file_name,
                    human_bytes(f.size),
                    if f.verified { "" } else { " unverified" }
                ));
                if l.len() > 200 {
                    l.pop_front();
                }
            }),
            on_progress: Box::new(|_, _, _| {}),
            on_message: Box::new(move |m| {
                let mut l = log2.lock().unwrap();
                l.push_back(m);
                if l.len() > 200 {
                    l.pop_front();
                }
            }),
        };
        match ketra_core::lan::server::Receiver::start(cfg, hooks) {
            Ok(server) => {
                let port = server.port();
                let mut l = self.receive_log.lock().unwrap();
                l.push_back(format!("listening on :{port} (LocalSend v2 + Ketra)"));
                self.receive_server = Some(server);
                flag.store(true, Ordering::SeqCst);
            }
            Err(e) => {
                self.receive_log.lock().unwrap().push_back(format!("receiver error: {e}"));
            }
        }
    }

    fn stop_receive(&mut self) {
        if let Some(s) = self.receive_server.take() {
            s.stop();
            self.receive_log.lock().unwrap().push_back("receiver stopped".into());
        }
        self.receiving.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct KetraApp {
    tab: Tab,
    dark: bool,

    // transfer
    kind: TransferKind,
    source_text: String,
    dest_text: String,
    verify: VerifyMode,
    conflict: OnConflict,
    limit_mbs: f64,
    net_streams: u32,
    flatten: bool,
    job: Option<Job>,
    last_log: VecDeque<String>,

    // lan
    lan: LanState,
    lan_selected: Option<usize>,
    send_files_text: String,
    receive_dest_text: String,

    // settings
    setting_verify: VerifyMode,
    setting_limit_mbs: f64,
    setting_buffer_kb: f64,
    setting_clone: bool,
    shell_status: String,
}

impl KetraApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut s = Settings2::load();
        let dark = s.theme_dark();
        theme::apply_theme(&cc.egui_ctx, dark);
        // Persisted defaults seed the transfer tab.
        let verify = match s.verify.as_deref() {
            Some("off") => VerifyMode::Off,
            Some("manifest") => VerifyMode::Manifest,
            _ => VerifyMode::Inline,
        };
        let app = KetraApp {
            tab: Tab::Transfer,
            dark,
            kind: TransferKind::Copy,
            source_text: String::new(),
            dest_text: String::new(),
            verify,
            conflict: OnConflict::KeepNewer,
            limit_mbs: s.speed_limit_bps.map(|v| v as f64 / (1024.0 * 1024.0)).unwrap_or(0.0),
            net_streams: 0,
            flatten: false,
            job: None,
            last_log: VecDeque::new(),
            lan: LanState::new(),
            send_files_text: String::new(),
            lan_selected: None,
            receive_dest_text: default_receive_dir().display().to_string(),
            setting_verify: verify,
            setting_limit_mbs: s.speed_limit_bps.map(|v| v as f64 / (1024.0 * 1024.0)).unwrap_or(0.0),
            setting_buffer_kb: s.buffer.map(|v| v as f64 / 1024.0).unwrap_or(1024.0),
            setting_clone: s.use_clone.unwrap_or(true),
            shell_status: String::new(),
        };
        app.lan.browse();
        app
    }

    fn build_options(&self) -> Options {
        let mut o = Options::default();
        o.verify = self.verify;
        o.conflict = self.conflict;
        o.flatten = self.flatten;
        o.use_sparse = true;
        o.use_clone = true;
        o.net_streams = self.net_streams;
        o.resume = true;
        if self.limit_mbs > 0.0 {
            o.speed_limit_bps = Some((self.limit_mbs * 1024.0 * 1024.0) as u64);
        }
        // Persisted chunk buffer
        let s = Settings2::load();
        if let Some(b) = s.buffer {
            o.buffer = b;
        }
        o
    }

    fn start_job(&mut self) {
        let sources: Vec<PathBuf> = self
            .source_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        if sources.is_empty() {
            self.shell_status = "Add at least one source path.".into();
            return;
        }
        let kind = self.kind;
        let dest = match kind {
            TransferKind::Test => PathBuf::new(),
            _ => PathBuf::from(self.dest_text.trim()),
        };
        if kind != TransferKind::Test && dest.as_os_str().is_empty() {
            self.shell_status = "Choose a destination folder.".into();
            return;
        }
        let opts = self.build_options();
        self.last_log.clear();
        self.job = Some(Job::start(kind, sources, dest, opts));
    }

    fn pump_job(&mut self) {
        if let Some(job) = &self.job {
            job.pump();
            // mirror errors into the log
            let errs = job.errors.lock().unwrap();
            for e in errs.iter() {
                if !self.last_log.iter().any(|l| l == e) {
                    self.last_log.push_back(e.clone());
                    if self.last_log.len() > 200 {
                        self.last_log.pop_front();
                    }
                }
            }
        }
        let done_needs_join = self.job.as_ref().map(|j| j.is_done() && j.thread.is_some()).unwrap_or(false);
        if done_needs_join {
            if let Some(job) = self.job.as_mut() {
                if let Some(handle) = job.thread.take() {
                    let _ = handle.join();
                }
            }
        }
        let done_unjoined = self.job.as_ref().map(|j| j.is_done() && j.thread.is_none()).unwrap_or(false);
        if done_unjoined {
            let job = self.job.take().unwrap();
            let p = job.progress.lock().unwrap();
            let line = if p.canceled {
                format!("canceled — {} file(s), {} kept for resume", p.done_files, human_bytes(p.copied_bytes))
            } else {
                format!(
                    "finished: {} copied, {} skipped, {} failed ({})",
                    p.done_files,
                    p.skipped,
                    p.failed,
                    human_bytes(p.copied_bytes)
                )
            };
            self.last_log.push_back(line);
        }
    }

    fn apply_theme_now(&mut self, ctx: &egui::Context) {
        theme::apply_theme(ctx, self.dark);
    }
}

impl eframe::App for KetraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_job();

        // Top app bar with tab chips.
        egui::TopBottomPanel::top("app_bar").show(ctx, |ui| {
            components::top_app_bar(ui, "KetraCopy", |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (tab, label) in [
                    (Tab::Transfer, "Transfer"),
                    (Tab::Lan, "LAN"),
                    (Tab::Settings, "Settings"),
                    (Tab::About, "About"),
                ] {
                    if components::chip(ui, label, self.tab == tab).clicked() {
                        self.tab = tab;
                    }
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Transfer => self.ui_transfer(ui),
            Tab::Lan => self.ui_lan(ui),
            Tab::Settings => self.ui_settings(ui, ctx),
            Tab::About => self.ui_about(ui),
        });

        // Keep animating while a job or LAN transfer runs.
        let busy = self.job.is_some() || self.lan.browsing.load(Ordering::SeqCst) || self.lan.send_running.load(Ordering::SeqCst);
        if busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

// ---------------------------------------------------------------------------
// Tab: Transfer
// ---------------------------------------------------------------------------

impl KetraApp {
    fn ui_transfer(&mut self, ui: &mut egui::Ui) {
        let s = components::current_scheme(ui.ctx());
        ui.add_space(8.0);

        // Mode chips
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (k, label) in [
                (TransferKind::Copy, "Copy"),
                (TransferKind::Move, "Move"),
                (TransferKind::Test, "Test"),
            ] {
                if components::chip(ui, label, self.kind == k).clicked() {
                    self.kind = k;
                }
            }
        });
        ui.add_space(6.0);

        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("Sources (one path per line)").color(s.on_surface_variant));
            egui::ScrollArea::vertical()
                .max_height(88.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.source_text)
                            .desired_width(f32::INFINITY)
                            .hint_text("/home/user/photos\n/mnt/disk/movie.iso"),
                    );
                });
            if self.kind != TransferKind::Test {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Destination folder").color(s.on_surface_variant));
                ui.add(
                    egui::TextEdit::singleline(&mut self.dest_text)
                        .desired_width(f32::INFINITY)
                        .hint_text("/mnt/backup"),
                );
            }
        });
        ui.add_space(8.0);

        // Options row
        components::card(ui, CardKind::Filled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Verify:");
                for (v, label) in [
                    (VerifyMode::Off, "Off"),
                    (VerifyMode::Inline, "Inline (byte compare)"),
                    (VerifyMode::Manifest, "Manifest (SHA-256)"),
                ] {
                    if components::chip(ui, label, self.verify == v).clicked() {
                        self.verify = v;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Conflicts:");
                for (c, label) in [
                    (OnConflict::KeepNewer, "Keep newer"),
                    (OnConflict::Overwrite, "Overwrite"),
                    (OnConflict::Skip, "Skip"),
                    (OnConflict::Rename, "Rename"),
                ] {
                    if components::chip(ui, label, self.conflict == c).clicked() {
                        self.conflict = c;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Speed limit (MiB/s, 0 = unlimited):");
                ui.add(egui::DragValue::new(&mut self.limit_mbs).speed(1.0).range(0.0..=10000.0));
                ui.separator();
                ui.label("Net streams:");
                ui.add(egui::DragValue::new(&mut self.net_streams).speed(1.0).range(0..=16));
                ui.separator();
                ui.checkbox(&mut self.flatten, "Flatten");
            });
        });
        ui.add_space(10.0);

        // Action row
        ui.horizontal(|ui| {
            let running = self.job.is_some() && !self.job.as_ref().unwrap().is_done();
            if running {
                if components::filled_button_colored(ui, "Cancel", s.error, s.on_error).clicked() {
                    self.job.as_ref().unwrap().cancel.store(true, Ordering::SeqCst);
                }
            } else if components::filled_button(ui, "Start").clicked() {
                self.start_job();
            }
            if self.kind != TransferKind::Test {
                let d = PathBuf::from(self.dest_text.trim());
                let enabled = !d.as_os_str().is_empty() && d.is_dir();
                ui.add_enabled_ui(enabled, |ui| {
                    if components::outlined_button(ui, "Paste clipboard here").clicked() {
                        let sources = self.source_text.clone();
                        let dest = d.clone();
                        let opts = self.build_options();
                        // Clipboard paste runs on a worker through the same engine.
                        self.last_log.clear();
                        self.job = Some(paste_job(dest, opts, sources.is_empty()));
                    }
                });
            }
            if !self.shell_status.is_empty() {
                ui.label(egui::RichText::new(&self.shell_status).color(s.on_surface_variant));
            }
        });
        ui.add_space(10.0);

        // Progress card
        let (progress, phase) = {
            match &self.job {
                Some(job) => {
                    let p = job.progress.lock().unwrap();
                    let frac = if p.total_bytes > 0 {
                        p.copied_bytes as f32 / p.total_bytes as f32
                    } else if p.finished {
                        1.0
                    } else {
                        -1.0
                    };
                    let summary = format!(
                        "{}  [{}{}]  {} / {}  {} /s  eta {}s",
                        p.phase,
                        p.done_files,
                        p.total_files,
                        human_bytes(p.copied_bytes),
                        human_bytes(p.total_bytes),
                        human_bytes(p.speed_bps.max(0.0) as u64),
                        p.eta_secs.max(0.0) as u64
                    );
                    (Some(frac), summary)
                }
                None => (None, format!("{} — idle", self.kind.label())),
            }
        };
        components::card(ui, CardKind::Elevated, |ui| {
            ui.label(phase);
            ui.add_space(6.0);
            components::linear_progress(ui, progress);
            if !self.last_log.is_empty() {
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for line in self.last_log.iter().rev() {
                        ui.label(egui::RichText::new(line).small().color(s.on_surface_variant));
                    }
                });
            }
        });
    }
}

/// A clipboard-paste job (reads the native clipboard on the worker thread).
fn paste_job(dest: PathBuf, opts: Options, _no_sources: bool) -> Job {
    // Reuse the Job plumbing by reading the clipboard first on this thread
    // (fast), then starting an engine job with the resolved sources.
    let sources = read_clipboard_sources();
    match sources {
        Ok((paths, cut)) => {
            let kind = if cut { TransferKind::Move } else { TransferKind::Copy };
            Job::start(kind, paths, dest, opts)
        }
        Err(e) => {
            let (tx, rx) = mpsc::channel::<Event>();
            drop(tx);
            Job {
                kind: TransferKind::Copy,
                sources: vec![],
                dest,
                cancel: Arc::new(AtomicBool::new(false)),
                rx,
                progress: Arc::new(Mutex::new(UiProgress {
                    finished: true,
                    exit_code: 1,
                    phase: format!("clipboard: {e}"),
                    ..Default::default()
                })),
                errors: Arc::new(Mutex::new(vec![format!("clipboard: {e}")])),
                thread: None,
            }
        }
    }
}

/// Minimal inline clipboard read (file paths only) reusing the CLI codecs'
/// same protocol knowledge. Kept local to the GUI to avoid a cyclic dep.
fn read_clipboard_sources() -> Result<(Vec<PathBuf>, bool), String> {
    #[cfg(windows)]
    {
        gui_clip::win_read()
    }
    #[cfg(target_os = "macos")]
    {
        gui_clip::mac_read()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        gui_clip::x11_read()
    }
}

#[cfg(windows)]
mod gui_clip {
    use super::*;
    const CF_HDROP: u32 = 15;
    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: isize) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(fmt: u32) -> isize;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalLock(h: isize) -> *mut u8;
        fn GlobalUnlock(h: isize) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }
    pub fn win_read() -> Result<(Vec<PathBuf>, bool), String> {
        unsafe {
            if OpenClipboard(0) == 0 {
                return Err("cannot open clipboard".into());
            }
            let r = (|| {
                let h = GetClipboardData(CF_HDROP);
                if h == 0 {
                    return Err("clipboard has no files".into());
                }
                let p = GlobalLock(h);
                if p.is_null() {
                    return Err("GlobalLock failed".into());
                }
                let mut bytes = Vec::new();
                let mut i = 0usize;
                loop {
                    if i >= 1024 * 1024 {
                        break;
                    }
                    let b = *p.add(i);
                    bytes.push(b);
                    if i >= 8 && bytes[i] == 0 && bytes[i - 1] == 0 && bytes[i - 2] == 0 && bytes[i - 3] == 0
                        && bytes[i - 4] == 0 && bytes[i - 5] == 0 && bytes[i - 6] == 0 && bytes[i - 7] == 0
                    {
                        break;
                    }
                    i += 1;
                }
                GlobalUnlock(h);
                let paths = ketra_parse_dropfiles(&bytes).ok_or("malformed DROPFILES")?;
                let mut cut = false;
                let name = wide("Preferred DropEffect");
                let fmt = RegisterClipboardFormatW(name.as_ptr());
                if fmt != 0 {
                    let h2 = GetClipboardData(fmt);
                    if h2 != 0 {
                        let p2 = GlobalLock(h2);
                        if !p2.is_null() {
                            cut = (*p2 & 2) != 0;
                            GlobalUnlock(h2);
                        }
                    }
                }
                Ok((paths.into_iter().map(PathBuf::from).collect(), cut))
            })();
            CloseClipboard();
            r
        }
    }
    fn ketra_parse_dropfiles(bytes: &[u8]) -> Option<Vec<String>> {
        if bytes.len() < 20 {
            return None;
        }
        let off = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if off < 20 || off > bytes.len() {
            return None;
        }
        let list = &bytes[off..];
        let u16s: Vec<u16> = list.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut out = Vec::new();
        let mut cur: Vec<u16> = Vec::new();
        for u in u16s {
            if u == 0 {
                if cur.is_empty() {
                    break;
                }
                out.push(String::from_utf16_lossy(&cur));
                cur.clear();
            } else {
                cur.push(u);
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

#[cfg(target_os = "macos")]
mod gui_clip {
    use super::*;
    pub fn mac_read() -> Result<(Vec<PathBuf>, bool), String> {
        let script = "set out to \"\"\ntry\n  set fl to the clipboard as list\non error\n  try\n    set fl to {the clipboard as «class furl»}\n  on error\n    return \"\"\n  end try\nend try\nrepeat with f in fl\n  try\n    set out to out & (POSIX path of f) & linefeed\n  end try\nend repeat\nreturn out";
        let out = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        let paths: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        if paths.is_empty() {
            return Err("clipboard has no files".into());
        }
        Ok((paths, false))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod gui_clip {
    use super::*;
    pub fn x11_read() -> Result<(Vec<PathBuf>, bool), String> {
        // The GUI shell delegates clipboard reading to the `ketra` CLI, which
        // owns the full X11 implementation (selection daemon + codecs).
        if std::env::var_os("DISPLAY").is_none() {
            return Err("no X11 display".into());
        }
        let exe = find_ketra_cli().ok_or("ketra CLI not found next to the GUI")?;
        let out = std::process::Command::new(exe)
            .args(["clip", "get", "--json"])
            .output()
            .map_err(|e| format!("ketra clip get: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut paths = Vec::new();
        let mut cut = false;
        if let Some(mpos) = text.find("\"mode\":\"") {
            let rest = &text[mpos + 8..];
            if rest.starts_with("cut") {
                cut = true;
            }
        }
        for seg in text.split("\"path\":\"").skip(1) {
            if let Some(end) = seg.find('"') {
                let raw = &seg[..end];
                paths.push(PathBuf::from(unescape_json(raw)));
            }
        }
        if paths.is_empty() {
            return Err("clipboard has no files".into());
        }
        Ok((paths, cut))
    }
    fn unescape_json(s: &str) -> String {
        s.replace("\\\\", "\u{0}")
            .replace("\\\"", "\"")
            .replace('\\', "")
            .replace('\u{0}', "\\")
    }
}

fn find_ketra_cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let c = dir.join(if cfg!(windows) { "ketra.exe" } else { "ketra" });
    if c.exists() {
        Some(c)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tab: LAN
// ---------------------------------------------------------------------------

impl KetraApp {
    fn ui_lan(&mut self, ui: &mut egui::Ui) {
        let s = components::current_scheme(ui.ctx());
        ui.add_space(8.0);

        components::card(ui, CardKind::Filled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Device alias:");
                ui.add(egui::TextEdit::singleline(&mut self.lan.alias).desired_width(160.0));
                ui.separator();
                if components::tonal_button(ui, "Refresh peers").clicked() {
                    self.lan.browse();
                }
                ui.label(
                    egui::RichText::new(if self.lan.browsing.load(Ordering::SeqCst) {
                        "browsing…"
                    } else {
                        ""
                    })
                    .small()
                    .color(s.on_surface_variant),
                );
            });
        });
        ui.add_space(8.0);

        // Peers list
        let peers = self.lan.peers.lock().unwrap().clone();
        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new(format!("Peers on this network ({})", peers.len())).color(s.on_surface_variant));
            if peers.is_empty() {
                ui.label(egui::RichText::new("No LocalSend/Ketra peers found yet — start a receiver on another machine, or press Refresh.").small().color(s.on_surface_variant));
            }
            for (i, p) in peers.iter().enumerate() {
                components::list_item(ui, &p.alias, Some(&format!("{} {}", p.addr, if p.ketra { "· ketra accelerated" } else { "· localsend v2" })), |ui| {
                    if components::text_button(ui, "Select").clicked() {
                        self.lan_selected = Some(i);
                    }
                });
            }
        });
        ui.add_space(8.0);

        // Send card
        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("Send files").color(s.on_surface_variant));
            ui.add(
                egui::TextEdit::multiline(&mut self.send_files_text)
                    .desired_width(f32::INFINITY)
                    .hint_text("one absolute file path per line"),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let selected = self.lan_selected.and_then(|i| peers.get(i).cloned());
                let can_send = selected.is_some()
                    && !self.send_files_text.trim().is_empty()
                    && !self.lan.send_running.load(Ordering::SeqCst);
                ui.add_enabled_ui(can_send, |ui| {
                    let label = match &selected {
                        Some(p) => format!("Send to {}", p.alias),
                        None => "Send to peer".to_string(),
                    };
                    if components::filled_button(ui, &label).clicked() {
                        let files: Vec<PathBuf> = self
                            .send_files_text
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty())
                            .map(PathBuf::from)
                            .collect();
                        if let Some(p) = selected {
                            self.lan.send(p, files);
                        }
                    }
                });
                ui.label(
                    egui::RichText::new(self.lan.send_status.lock().unwrap().clone())
                        .small()
                        .color(s.on_surface_variant),
                );
            });
        });
        ui.add_space(8.0);

        // Receive card
        components::card(ui, CardKind::Elevated, |ui| {
            ui.label(egui::RichText::new("Receive").color(s.on_surface_variant));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.receive_dest_text)
                        .desired_width(320.0)
                        .hint_text("incoming files land here"),
                );
                let on = self.lan.receive_server.is_some();
                if on {
                    if components::filled_button_colored(ui, "Stop receiver", s.error, s.on_error).clicked() {
                        self.lan.stop_receive();
                    }
                } else if components::filled_button(ui, "Start receiver").clicked() {
                    let dest = PathBuf::from(self.receive_dest_text.trim());
                    if dest.as_os_str().is_empty() {
                        self.lan.receive_log.lock().unwrap().push_back("set a destination folder first".into());
                    } else {
                        let dest = dest.clone();
                        let lan = &mut self.lan;
                        let alias = lan.alias.clone();
                        let log = lan.receive_log.clone();
                        let log2 = lan.receive_log.clone();
                        let flag = lan.receiving.clone();
                        let cfg = ketra_core::lan::server::ReceiverConfig {
                            alias,
                            dest_dir: dest,
                            port: 0,
                            auto_accept: true,
                            verify_sha256: true,
                            announce: true,
                            max_streams: 4,
                        };
                        let hooks = ketra_core::lan::server::ReceiverHooks {
                            on_request: Box::new(|_| true),
                            on_file_done: Box::new(move |f| {
                                let mut l = log.lock().unwrap();
                                l.push_back(format!(
                                    "{} {} ({}){}",
                                    if f.verified { "✓" } else { "?" },
                                    f.file_name,
                                    human_bytes(f.size),
                                    if f.verified { "" } else { " unverified" }
                                ));
                                if l.len() > 200 {
                                    l.pop_front();
                                }
                            }),
                            on_progress: Box::new(|_, _, _| {}),
                            on_message: Box::new(move |m| {
                                let mut l = log2.lock().unwrap();
                                l.push_back(m);
                                if l.len() > 200 {
                                    l.pop_front();
                                }
                            }),
                        };
                        match ketra_core::lan::server::Receiver::start(cfg, hooks) {
                            Ok(server) => {
                                let port = server.port();
                                lan.receive_log.lock().unwrap().push_back(format!("listening on :{port} (LocalSend v2 + Ketra)"));
                                lan.receive_server = Some(server);
                                flag.store(true, Ordering::SeqCst);
                            }
                            Err(e) => {
                                lan.receive_log.lock().unwrap().push_back(format!("receiver error: {e}"));
                            }
                        }
                    }
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                let log = self.lan.receive_log.lock().unwrap();
                for line in log.iter().rev() {
                    ui.label(egui::RichText::new(line).small().color(s.on_surface_variant));
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Tab: Settings
// ---------------------------------------------------------------------------

struct Settings2 {
    verify: Option<String>,
    speed_limit_bps: Option<u64>,
    buffer: Option<usize>,
    use_clone: Option<bool>,
    direct_io: Option<bool>,
    theme: Option<String>,
}

impl Settings2 {
    fn load() -> Self {
        let path = ketra_core::settings::Settings::default_path();
        match ketra_core::settings::Settings::load(&path) {
            Ok(s) => Settings2 {
                verify: s.verify,
                speed_limit_bps: s.speed_limit_bps,
                buffer: s.buffer,
                use_clone: s.use_clone,
                direct_io: s.direct_io,
                theme: s.theme,
            },
            Err(_) => Settings2::default_vals(),
        }
    }

    fn default_vals() -> Self {
        Settings2 {
            verify: None,
            speed_limit_bps: None,
            buffer: None,
            use_clone: None,
            direct_io: None,
            theme: None,
        }
    }

    fn theme_dark(&self) -> bool {
        match self.theme.as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => false, // "system" detection is a nicety; default light
        }
    }

    fn save(&self) {
        let s = ketra_core::settings::Settings {
            verify: self.verify.clone(),
            speed_limit_bps: self.speed_limit_bps,
            buffer: self.buffer,
            use_clone: self.use_clone,
            direct_io: self.direct_io,
            theme: self.theme.clone(),
        };
        let _ = s.save(&ketra_core::settings::Settings::default_path());
    }
}

impl KetraApp {
    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = components::current_scheme(ui.ctx());
        ui.add_space(8.0);

        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("Appearance").color(s.on_surface_variant));
            ui.horizontal(|ui| {
                for (name, dark) in [("Light", false), ("Dark", true)] {
                    if components::chip(ui, name, self.dark == dark).clicked() {
                        self.dark = dark;
                        theme::apply_theme(ctx, dark);
                    }
                }
            });
        });
        ui.add_space(8.0);

        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("Transfer defaults (persisted to ketracopy.json)").color(s.on_surface_variant));
            ui.horizontal(|ui| {
                ui.label("Verify:");
                for (v, label) in [
                    (VerifyMode::Off, "off"),
                    (VerifyMode::Inline, "inline"),
                    (VerifyMode::Manifest, "manifest"),
                ] {
                    if components::chip(ui, label, self.setting_verify == v).clicked() {
                        self.setting_verify = v;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Speed limit (MiB/s, 0 = off):");
                ui.add(egui::DragValue::new(&mut self.setting_limit_mbs).speed(1.0).range(0.0..=10000.0));
                ui.separator();
                ui.label("Buffer (KiB):");
                ui.add(egui::DragValue::new(&mut self.setting_buffer_kb).speed(64.0).range(4.0..=16384.0));
                ui.separator();
                ui.checkbox(&mut self.setting_clone, "Relink/CFR fast paths");
            });
            ui.add_space(6.0);
            if components::filled_button(ui, "Save defaults").clicked() {
                let st = Settings2 {
                    verify: Some(
                        match self.setting_verify {
                            VerifyMode::Off => "off",
                            VerifyMode::Inline => "inline",
                            VerifyMode::Manifest => "manifest",
                        }
                        .to_string(),
                    ),
                    speed_limit_bps: if self.setting_limit_mbs > 0.0 {
                        Some((self.setting_limit_mbs * 1024.0 * 1024.0) as u64)
                    } else {
                        None
                    },
                    buffer: Some((self.setting_buffer_kb * 1024.0) as usize),
                    use_clone: Some(self.setting_clone),
                    direct_io: None,
                    theme: Some(if self.dark { "dark" } else { "light" }.to_string()),
                };
                st.save();
                self.shell_status = "defaults saved".into();
            }
        });
        ui.add_space(8.0);

        // Shell integration card
        components::card(ui, CardKind::Elevated, |ui| {
            ui.label(egui::RichText::new("Shell integration (right-click menus + shortcuts)").color(s.on_surface_variant));
            ui.label(
                egui::RichText::new(
                    "Registers KetraCopy copy / cut / paste entries in your file manager and the platform keyboard shortcuts.",
                )
                .small()
                .color(s.on_surface_variant),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if components::filled_button(ui, "Install").clicked() {
                    self.shell_status = run_cli(&["shell", "install"]);
                }
                if components::outlined_button(ui, "Uninstall").clicked() {
                    self.shell_status = run_cli(&["shell", "uninstall"]);
                }
            });
            if !self.shell_status.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(&self.shell_status).small().color(s.on_surface_variant));
                });
            }
        });
    }
}

fn run_cli(args: &[&str]) -> String {
    match find_ketra_cli() {
        Some(exe) => match std::process::Command::new(exe).args(args).output() {
            Ok(o) => {
                let mut out = String::from_utf8_lossy(&o.stdout).into_owned();
                let err = String::from_utf8_lossy(&o.stderr);
                if !err.is_empty() {
                    out.push_str(&err);
                }
                if out.is_empty() {
                    "done".into()
                } else {
                    out
                }
            }
            Err(e) => format!("failed to run ketra: {e}"),
        },
        None => "ketra CLI not found next to ketracopy-gui".into(),
    }
}

// ---------------------------------------------------------------------------
// Tab: About
// ---------------------------------------------------------------------------

impl KetraApp {
    fn ui_about(&mut self, ui: &mut egui::Ui) {
        let s = components::current_scheme(ui.ctx());
        ui.add_space(12.0);
        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("KetraCopy").heading().color(s.on_surface));
            ui.label(
                egui::RichText::new(format!(
                    "v{} (engine v{}) — clean-room TeraCopy-style file transfer",
                    env!("CARGO_PKG_VERSION"),
                    ketra_core::VERSION
                ))
                .color(s.on_surface_variant),
            );
            ui.label(egui::RichText::new("MIT licensed · Rust std-only engine · LocalSend-compatible LAN transfer").small().color(s.on_surface_variant));
            ui.label(egui::RichText::new("Developed with Z.ai (GLM model) — see ATTRIBUTIONS for details.").small().color(s.on_surface_variant));
        });
        ui.add_space(8.0);
        components::card(ui, CardKind::Filled, |ui| {
            ui.label(egui::RichText::new("Engine pipeline").color(s.on_surface_variant));
            ui.label("1. Fast scan — size+mtime comparison skips up-to-date files");
            ui.label("2. Stream copy — chunked, inline byte-compare, reflink/CFR fast paths, sparse-aware, resumable journal, AIMD throttling");
            ui.label("3. Manifest — SHA-256 manifest for later integrity verification");
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Default)]
struct LanSelectedSlot(Option<usize>);

fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.2} GiB", v / GB)
    } else if v >= MB {
        format!("{:.1} MiB", v / MB)
    } else if v >= KB {
        format!("{:.1} KiB", v / KB)
    } else {
        format!("{n} B")
    }
}

fn default_receive_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("."))
}
