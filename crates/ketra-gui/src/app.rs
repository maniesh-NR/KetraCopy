//! The KetraCopy app shell: source list → options → confirm → live progress
//! → summary, all styled with the ketra-md3 design system.

use crate::job::{self, AppMsg};
use egui::{Context, RichText, ScrollArea, TextStyle, Ui};
use egui::TextEdit;
use ketra_core::engine;
use ketra_core::options::{ConflictPolicy, TransferOptions, VerifyLevel};
use ketra_core::plan::TransferPlan;
use ketra_core::progress::Event;
use ketra_core::Summary;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

/// Keep the activity log bounded (task spec: last ~200 events).
const LOG_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogKind {
    Info,
    Ok,
    Warn,
    Error,
}

struct LogLine {
    kind: LogKind,
    text: String,
}

enum RunState {
    /// Configuring a new transfer.
    Idle,
    /// The planner thread is scanning the sources.
    Planning,
    /// Plan built; waiting for the user to confirm.
    Confirming {
        plan: TransferPlan,
        opts: TransferOptions,
    },
    /// The engine is running on a worker thread.
    Running { job: job::EngineJob },
    /// Show the summary card.
    Finished { summary: Summary },
}

/// Aggregate, UI-side progress derived purely from engine events.
#[derive(Default)]
struct Live {
    total_bytes: u64,
    bytes_done: u64,
    files_total: u64,
    files_ok: u64,
    files_failed: u64,
    files_skipped: u64,
    /// Most recently started file (for the "current file" line).
    current: Option<String>,
    /// Per-task bytes already accounted for (parallel-safe deltas).
    in_flight: HashMap<usize, (String, u64)>,
    /// Aggregate throughput EMA, bytes/sec.
    bps_ema: f64,
    last_bytes: u64,
    last_t: Option<Instant>,
    eta_secs: f64,
    paused: bool,
}

pub struct KetraApp {
    dark: bool,

    // ---- configuration ----
    sources: Vec<String>,
    new_source: String,
    dest: String,
    verify: VerifyLevel,
    conflict: ConflictPolicy,
    /// Speed limit in MiB/s (0 = unlimited; matches the CLI's semantics).
    limit_mib: f32,
    /// Concurrent files (0 = auto by device class).
    parallel: usize,

    // ---- run state ----
    run: RunState,
    live: Live,
    log: VecDeque<LogLine>,
    status: Option<String>,
    msg_tx: std::sync::mpsc::Sender<AppMsg>,
    msg_rx: Receiver<AppMsg>,
}

impl Default for KetraApp {
    fn default() -> Self {
        Self::new()
    }
}

impl KetraApp {
    pub fn new() -> Self {
        let (msg_tx, msg_rx) = channel();
        Self {
            dark: false,
            sources: Vec::new(),
            new_source: String::new(),
            dest: String::new(),
            // Engine options stay consistent with the CLI defaults:
            // verify inline (the enum default), manifest on, journal on,
            // resume on — all inside `TransferOptions::default()`.
            verify: VerifyLevel::default(),
            conflict: ConflictPolicy::default(),
            limit_mib: 0.0,
            parallel: 0,
            run: RunState::Idle,
            live: Live::default(),
            log: VecDeque::new(),
            status: Some("Ready".into()),
            msg_tx,
            msg_rx,
        }
    }

    // ------------------------------------------------------------------
    // Options / actions
    // ------------------------------------------------------------------

    /// GUI knobs → engine options. Unlisted fields keep the CLI defaults
    /// (verify Inline, manifest + journal + resume on).
    fn build_opts(&self) -> TransferOptions {
        TransferOptions {
            verify: self.verify,
            conflict: self.conflict,
            limit_bps: if self.limit_mib > 0.0 {
                // MiB/s → B/s. Slider tops out at 200; no truncation risk,
                // and 32-bit-safe regardless (u64 math).
                (self.limit_mib as u64) << 20
            } else {
                0
            },
            parallel_files: self.parallel,
            ..TransferOptions::default()
        }
    }

    fn push_source(&mut self, path: String) {
        let path = path.trim().to_string();
        if path.is_empty() {
            return;
        }
        if self.sources.iter().any(|s| s == &path) {
            self.push_log(LogKind::Warn, format!("already added: {path}"));
            return;
        }
        self.push_log(LogKind::Info, format!("source: {path}"));
        self.sources.push(path);
    }

    fn begin_planning(&mut self) {
        self.push_log(LogKind::Info, "Scanning sources (dry run)…".into());
        self.status = Some("Scanning sources…".into());
        self.run = RunState::Planning;
        let sources: Vec<PathBuf> = self.sources.iter().map(PathBuf::from).collect();
        let dest = PathBuf::from(&self.dest);
        let opts = self.build_opts();
        job::plan_async(sources, dest, opts, self.msg_tx.clone());
    }

    fn start_engine(&mut self, plan: TransferPlan, opts: TransferOptions) {
        self.push_log(
            LogKind::Info,
            format!(
                "Starting: {} files, {}",
                plan.file_count(),
                engine::human(plan.total_bytes)
            ),
        );
        self.live = Live::default();
        self.run = RunState::Running {
            job: job::start_engine(plan, opts),
        };
    }

    fn reset(&mut self) {
        self.run = RunState::Idle;
        self.live = Live::default();
        self.status = Some("Ready".into());
    }

    fn finish(&mut self, summary: Summary) {
        self.push_log(
            if summary.canceled || summary.files_failed > 0 {
                LogKind::Error
            } else {
                LogKind::Ok
            },
            format!(
                "Finished: {} ok, {} failed, {} skipped — {} in {} (canceled={})",
                summary.files_ok,
                summary.files_failed,
                summary.files_skipped,
                engine::human(summary.bytes),
                fmt_duration_ms(summary.ms),
                summary.canceled
            ),
        );
        self.status = Some(
            if summary.canceled {
                "Transfer canceled"
            } else if summary.files_failed > 0 {
                "Transfer finished with errors"
            } else {
                "Transfer finished"
            }
            .to_string(),
        );
        self.run = RunState::Finished { summary };
    }

    // ------------------------------------------------------------------
    // Event pumping (UI thread never blocks: try_recv only)
    // ------------------------------------------------------------------

    fn pump(&mut self) {
        // 1. app-level messages (planner results).
        loop {
            match self.msg_rx.try_recv() {
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                Ok(AppMsg::PlanReady(plan)) => {
                    if matches!(self.run, RunState::Planning) {
                        self.push_log(
                            LogKind::Info,
                            format!(
                                "Plan ready: {} tasks ({} files), {}",
                                plan.tasks.len(),
                                plan.file_count(),
                                engine::human(plan.total_bytes)
                            ),
                        );
                        self.status = Some("Confirm the transfer to start".into());
                        let opts = self.build_opts();
                        self.run = RunState::Confirming { plan, opts };
                    }
                }
                Ok(AppMsg::PlanFailed(e)) => {
                    self.push_log(LogKind::Error, format!("plan failed: {e}"));
                    self.status = Some(format!("Planning failed: {e}"));
                    if matches!(self.run, RunState::Planning) {
                        self.run = RunState::Idle;
                    }
                }
            }
        }

        // 2. engine events + one-shot summary.
        let mut events = Vec::new();
        let mut done_summary = None;
        let mut engine_died = false;
        if let RunState::Running { job } = &mut self.run {
            while let Ok(ev) = job.events.try_recv() {
                events.push(ev);
            }
            match job.done.try_recv() {
                Ok(s) => done_summary = Some(s),
                Err(TryRecvError::Disconnected) => engine_died = true,
                Err(TryRecvError::Empty) => {}
            }
        }
        for ev in events {
            self.on_event(ev);
        }
        if engine_died {
            self.push_log(LogKind::Error, "engine thread ended unexpectedly".into());
            self.status = Some("Engine thread ended unexpectedly".into());
            self.run = RunState::Idle;
        }
        if let Some(s) = done_summary {
            self.finish(s);
        }
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Started { files, bytes } => {
                self.live.files_total = files;
                self.live.total_bytes = bytes;
                self.push_log(
                    LogKind::Info,
                    format!("started: {files} files, {}", engine::human(bytes)),
                );
            }
            Event::Warning { message } => {
                // Surface warnings (e.g. low free space) prominently.
                self.status = Some(format!("⚠ {message}"));
                self.push_log(LogKind::Warn, message);
            }
            Event::TaskStart { idx, src, size, .. } => {
                let name = file_name_of(&src);
                self.live.current = Some(name.clone());
                self.live.in_flight.insert(idx, (name.clone(), 0));
                self.push_log(
                    LogKind::Info,
                    format!("▸ {name} ({})", engine::human(size)),
                );
            }
            Event::TaskPhase { phase, .. } => {
                if let Some((_, _)) = self.live.in_flight.values().next() {
                    // Phase note rides along in the current-file line.
                    if let Some(cur) = &mut self.live.current {
                        let base = cur.split(" — ").next().unwrap_or(cur).to_string();
                        *cur = format!("{base} — {}", phase.as_str());
                    }
                }
            }
            Event::TaskProgress { idx, done, .. } => {
                let entry = self.live.in_flight.entry(idx).or_insert((String::new(), 0));
                let prev = entry.1;
                if done > prev {
                    self.live.bytes_done += done - prev;
                    entry.1 = done;
                }
            }
            Event::TaskDone { idx, bytes, bps, .. } => {
                let (name, prev) = self
                    .live
                    .in_flight
                    .remove(&idx)
                    .unwrap_or_else(|| (String::new(), 0));
                if bytes > prev {
                    self.live.bytes_done += bytes - prev;
                }
                self.live.files_ok += 1;
                self.live.current = None;
                self.push_log(
                    LogKind::Ok,
                    format!(
                        "✓ {} ({}) — {}/s",
                        if name.is_empty() { "?" } else { &name },
                        engine::human(bytes),
                        engine::human(bps)
                    ),
                );
            }
            Event::TaskSkipped { reason, .. } => {
                self.live.files_skipped += 1;
                self.push_log(LogKind::Warn, format!("↷ {reason}"));
            }
            Event::TaskError { message, .. } => {
                self.live.files_failed += 1;
                self.push_log(LogKind::Error, format!("✗ {message}"));
            }
            Event::ConflictResolved { action, .. } => {
                self.push_log(
                    LogKind::Info,
                    format!("conflict resolved: {}", action.as_str()),
                );
            }
            Event::Paused => {
                self.live.paused = true;
                self.status = Some("Paused".into());
                self.push_log(LogKind::Info, "paused".into());
            }
            Event::Resumed => {
                self.live.paused = false;
                self.status = Some("Resumed".into());
                self.push_log(LogKind::Info, "resumed".into());
            }
            Event::Finished { .. } => {
                // The authoritative Summary arrives via the done channel.
            }
        }
    }

    /// Frame-time throughput/ETA update (aggregate EMA over UI frames).
    fn tick_throughput(&mut self) {
        let running = matches!(self.run, RunState::Running { .. });
        let now = Instant::now();
        match self.live.last_t {
            Some(last) => {
                let dt = now.duration_since(last).as_secs_f64();
                if dt >= 0.25 {
                    let delta = self.live.bytes_done.saturating_sub(self.live.last_bytes);
                    let inst = delta as f64 / dt;
                    self.live.bps_ema = if self.live.bps_ema == 0.0 {
                        inst
                    } else {
                        self.live.bps_ema * 0.7 + inst * 0.3
                    };
                    self.live.last_bytes = self.live.bytes_done;
                    self.live.last_t = Some(now);
                }
            }
            None if running => {
                self.live.last_t = Some(now);
                self.live.last_bytes = self.live.bytes_done;
            }
            None => {}
        }
        if !running {
            self.live.bps_ema = 0.0;
        }
        let remaining = self.live.total_bytes.saturating_sub(self.live.bytes_done) as f64;
        self.live.eta_secs = if running && self.live.bps_ema > 1.0 {
            remaining / self.live.bps_ema
        } else {
            0.0
        };
    }

    fn push_log(&mut self, kind: LogKind, text: String) {
        self.log.push_back(LogLine { kind, text });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
    }

    fn handle_drops(&mut self, ctx: &Context) {
        if !matches!(self.run, RunState::Idle) {
            return;
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            if let Some(p) = f.path {
                self.push_source(p.display().to_string());
            }
        }
    }

    // ------------------------------------------------------------------
    // Painting
    // ------------------------------------------------------------------

    fn ui(&mut self, ctx: &Context) {
        let scheme = ketra_md3::current_scheme(ctx);

        egui::TopBottomPanel::top("ketra_app_bar").show(ctx, |ui| {
            ketra_md3::top_app_bar(ui, "KetraCopy", |ui| {
                let label = if self.dark { "Light mode" } else { "Dark mode" };
                if ketra_md3::chip(ui, label, false).clicked() {
                    self.dark = !self.dark;
                    ketra_md3::apply_theme(ui.ctx(), self.dark);
                }
            });
        });

        // Bottom status line (MD3 snackbar style). Warnings from the engine
        // land here via Event::Warning.
        egui::TopBottomPanel::bottom("ketra_status").show(ctx, |ui| {
            ui.add_space(4.0);
            match &self.status {
                Some(s) => {
                    ketra_md3::snackbar(ui, s);
                }
                None => {
                    ui.add_space(4.0);
                }
            }
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(scheme.surface))
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .id_source("ketra_main")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        // Sequential matches: each state check ends its
                        // borrow before the view method takes `&mut self`.
                        if matches!(
                            self.run,
                            RunState::Idle | RunState::Planning | RunState::Confirming { .. }
                        ) {
                            if matches!(self.run, RunState::Confirming { .. }) {
                                ui.disable(); // dim the form behind the modal
                            }
                            self.config_ui(ui);
                        }
                        if matches!(self.run, RunState::Running { .. }) {
                            self.live_ui(ui);
                        }
                        if matches!(self.run, RunState::Finished { .. }) {
                            self.finished_ui(ui);
                        }
                    });
                if matches!(self.run, RunState::Confirming { .. }) {
                    self.confirm_modal(ui);
                }
            });
    }

    /// Source list, destination, options and the big Start button.
    fn config_ui(&mut self, ui: &mut Ui) {
        let scheme = ketra_md3::current_scheme(ui.ctx());
        let configuring = matches!(self.run, RunState::Idle);

        // ---- sources ----
        ketra_md3::card(ui, ketra_md3::CardKind::Elevated, |ui| {
            ui.label(
                RichText::new("Sources")
                    .size(ketra_md3::typography::TITLE_MEDIUM)
                    .color(scheme.on_surface),
            );
            ui.add_space(4.0);

            if self.sources.is_empty() {
                ui.label(
                    RichText::new("No sources yet — add files or folders, or drop them here.")
                        .weak(),
                );
            }
            for i in 0..self.sources.len() {
                let path = self.sources[i].clone();
                let removed = ketra_md3::list_item(ui, &path, None, |ui| {
                    let r = ketra_md3::text_button(ui, "Remove");
                    r.clicked()
                })
                .inner;
                if removed && configuring {
                    self.sources.remove(i);
                    break; // indices shifted; repaint handles the rest
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let add_path = ui.add_enabled(
                    configuring,
                    TextEdit::singleline(&mut self.new_source)
                        .hint_text("Path to a file or folder…")
                        .desired_width(f32::INFINITY),
                );
                if ketra_md3::filled_button(ui, "Add…").clicked() && configuring {
                    if let Some(paths) = rfd::FileDialog::new()
                        .set_title("Add files or folders")
                        .pick_files()
                    {
                        for p in paths {
                            self.push_source(p.display().to_string());
                        }
                    }
                }
                if add_path.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && configuring
                {
                    let v = self.new_source.clone();
                    self.push_source(v);
                    self.new_source.clear();
                }
                if ketra_md3::tonal_button(ui, "Add path").clicked() && configuring {
                    let v = self.new_source.clone();
                    self.push_source(v);
                    self.new_source.clear();
                }
            });
        });

        // ---- destination ----
        ketra_md3::card(ui, ketra_md3::CardKind::Elevated, |ui| {
            ui.label(
                RichText::new("Destination")
                    .size(ketra_md3::typography::TITLE_MEDIUM)
                    .color(scheme.on_surface),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled(
                    configuring,
                    TextEdit::singleline(&mut self.dest)
                        .hint_text("Destination folder…")
                        .desired_width(f32::INFINITY),
                );
                if ketra_md3::tonal_button(ui, "Browse").clicked() && configuring {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_title("Choose destination folder")
                        .pick_folder()
                    {
                        self.dest = p.display().to_string();
                    }
                }
            });
        });

        // ---- options ----
        ketra_md3::card(ui, ketra_md3::CardKind::Elevated, |ui| {
            ui.label(
                RichText::new("Options")
                    .size(ketra_md3::typography::TITLE_MEDIUM)
                    .color(scheme.on_surface),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Verify").color(scheme.on_surface_variant));
                for (lvl, label) in [
                    (VerifyLevel::Off, "Off"),
                    (VerifyLevel::QuickScan, "Quick"),
                    (VerifyLevel::Inline, "Inline"),
                    (VerifyLevel::FullHash, "Full"),
                ] {
                    if ketra_md3::chip(ui, label, self.verify == lvl).clicked() {
                        self.verify = lvl;
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("Conflicts").color(scheme.on_surface_variant));
                egui::ComboBox::from_id_source("ketra_conflict")
                    .selected_text(conflict_label(self.conflict))
                    .show_ui(ui, |ui| {
                        ui.set_min_width(180.0);
                        ui.selectable_value(
                            &mut self.conflict,
                            ConflictPolicy::Overwrite,
                            conflict_label(ConflictPolicy::Overwrite),
                        );
                        ui.selectable_value(
                            &mut self.conflict,
                            ConflictPolicy::OverwriteIfNewer,
                            conflict_label(ConflictPolicy::OverwriteIfNewer),
                        );
                        ui.selectable_value(
                            &mut self.conflict,
                            ConflictPolicy::Skip,
                            conflict_label(ConflictPolicy::Skip),
                        );
                        ui.selectable_value(
                            &mut self.conflict,
                            ConflictPolicy::Rename,
                            conflict_label(ConflictPolicy::Rename),
                        );
                    });
            });

            let limit_text = if self.limit_mib <= 0.0 {
                "Speed limit: unlimited".to_string()
            } else {
                format!("Speed limit: {} MiB/s", self.limit_mib as u64)
            };
            ui.add(
                egui::Slider::new(&mut self.limit_mib, 0.0..=200.0)
                    .text(limit_text)
                    .integer(),
            );

            let par_text = if self.parallel == 0 {
                "Parallel files: auto".to_string()
            } else {
                let n = self.parallel;
                format!("Parallel files: {n}")
            };
            ui.add(egui::Slider::new(&mut self.parallel, 0..=8).text(par_text));
        });

        // ---- start ----
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            let valid = configuring && !self.sources.is_empty() && !self.dest.trim().is_empty();
            let mut start_clicked = false;
            ui.add_enabled_ui(valid, |ui| {
                start_clicked = ketra_md3::filled_button(ui, "Start").clicked();
            });
            if start_clicked {
                self.begin_planning();
            }
            if matches!(self.run, RunState::Planning) {
                ui.add_space(4.0);
                ketra_md3::linear_progress(ui, None); // indeterminate scan
                ui.label(RichText::new("Scanning sources…").weak());
            }
            ui.add_space(8.0);
        });
    }

    /// Live progress + controller buttons + activity log.
    fn live_ui(&mut self, ui: &mut Ui) {
        let scheme = ketra_md3::current_scheme(ui.ctx());

        // Snapshot the state we display (copies only), so the painting
        // closure can still log via `&mut self`.
        let ctrl = match &self.run {
            RunState::Running { job } => job.ctrl.clone(),
            _ => return,
        };
        let total_bytes = self.live.total_bytes;
        let bytes_done = self.live.bytes_done;
        let files_total = self.live.files_total;
        let files_ok = self.live.files_ok;
        let files_failed = self.live.files_failed;
        let files_skipped = self.live.files_skipped;
        let current = self.live.current.clone();
        let bps = self.live.bps_ema;
        let eta = self.live.eta_secs;

        let fraction = if total_bytes > 0 {
            // f64 division of u64s: no truncation, 32-bit safe.
            Some((bytes_done as f64 / total_bytes as f64).clamp(0.0, 1.0) as f32)
        } else {
            None
        };
        let pct = if total_bytes > 0 {
            (bytes_done as f64 / total_bytes as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        ketra_md3::card(ui, ketra_md3::CardKind::Elevated, |ui| {
            ketra_md3::linear_progress(ui, fraction);
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{pct:.1}% — {} of {}",
                    engine::human(bytes_done),
                    engine::human(total_bytes)
                ))
                .text_style(TextStyle::Heading)
                .color(scheme.on_surface),
            );
            ui.label(
                RichText::new(match &current {
                    Some(name) => format!("Current: {name}"),
                    None => "Current: —".to_string(),
                })
                .color(scheme.on_surface_variant),
            );
            ui.label(format!(
                "{:.1} MiB/s   ETA {}",
                bps / (1024.0 * 1024.0),
                fmt_eta(eta)
            ));
            ui.label(format!(
                "Files: {files_ok} ok · {files_failed} failed · {files_skipped} skipped (of {files_total})"
            ));

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ctrl.is_paused() {
                    if ketra_md3::tonal_button(ui, "Resume").clicked() {
                        ctrl.resume();
                    }
                } else if ketra_md3::tonal_button(ui, "Pause").clicked() {
                    ctrl.pause();
                }
                if ketra_md3::text_button(ui, "Skip current").clicked() {
                    ctrl.skip_current();
                    self.push_log(LogKind::Info, "skip requested".into());
                }
                if ketra_md3::filled_button_colored(ui, "Cancel", scheme.error, scheme.on_error)
                    .clicked()
                {
                    ctrl.cancel();
                    self.status = Some("Canceling…".into());
                    self.push_log(LogKind::Warn, "cancel requested".into());
                }
            });
        });

        self.log_ui(ui);
    }

    /// Summary card after the engine finishes.
    fn finished_ui(&mut self, ui: &mut Ui) {
        let scheme = ketra_md3::current_scheme(ui.ctx());
        let (ok, failed, skipped, bytes, ms, bps, canceled, manifest) = match &self.run {
            RunState::Finished { summary } => (
                summary.files_ok,
                summary.files_failed,
                summary.files_skipped,
                summary.bytes,
                summary.ms,
                summary.bps,
                summary.canceled,
                summary.manifest_path.clone(),
            ),
            _ => return,
        };

        ketra_md3::card(ui, ketra_md3::CardKind::Elevated, |ui| {
            let title = if canceled {
                "Transfer canceled"
            } else if failed > 0 {
                "Transfer finished with errors"
            } else {
                "Transfer complete"
            };
            ui.label(
                RichText::new(title)
                    .text_style(TextStyle::Heading)
                    .color(if failed > 0 { scheme.error } else { scheme.on_surface }),
            );
            ui.add_space(4.0);
            ui.label(format!(
                "{} ok · {failed} failed · {skipped} skipped",
                ok
            ));
            ui.label(format!(
                "{} in {} — average {}/s",
                engine::human(bytes),
                fmt_duration_ms(ms),
                engine::human(bps)
            ));
            match &manifest {
                Some(p) => ui.label(
                    RichText::new(format!("Manifest written to {}", p.display()))
                        .color(scheme.on_surface_variant),
                ),
                None => ui.label(
                    RichText::new("No manifest written (canceled or nothing copied).")
                        .weak(),
                ),
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ketra_md3::tonal_button(ui, "Reveal folder").clicked() {
                    reveal(&self.dest);
                }
                if ketra_md3::text_button(ui, "New transfer").clicked() {
                    self.reset();
                }
            });
        });

        self.log_ui(ui);
    }

    fn log_ui(&mut self, ui: &mut Ui) {
        let scheme = ketra_md3::current_scheme(ui.ctx());
        ketra_md3::card(ui, ketra_md3::CardKind::Filled, |ui| {
            ui.label(
                RichText::new("Activity")
                    .size(ketra_md3::typography::TITLE_MEDIUM)
                    .color(scheme.on_surface),
            );
            ScrollArea::vertical()
                .id_source("ketra_log")
                .max_height(220.0)
                .auto_shrink(false)
                .show(ui, |ui| {
                    for line in &self.log {
                        let color = match line.kind {
                            LogKind::Info => scheme.on_surface_variant,
                            LogKind::Ok => scheme.primary,
                            LogKind::Warn => scheme.tertiary,
                            LogKind::Error => scheme.error,
                        };
                        ui.label(
                            RichText::new(&line.text)
                                .monospace()
                                .size(12.0)
                                .color(color),
                        );
                    }
                });
        });
    }

    /// Dry-run confirmation dialog: total files/bytes, Start or Cancel.
    fn confirm_modal(&mut self, ui: &mut Ui) {
        let scheme = ketra_md3::current_scheme(ui.ctx());
        let (plan, opts) = match &self.run {
            RunState::Confirming { plan, opts } => (plan.clone(), opts.clone()),
            _ => return,
        };
        let files = plan.file_count();
        let tasks = plan.tasks.len();
        let bytes_total = plan.total_bytes;
        let dest = plan.dest_dir.display().to_string();

        let mut go = false;
        let mut abort = false;
        ketra_md3::modal(ui, |ui| {
            ui.set_min_width(440.0);
            ui.label(
                RichText::new("Start transfer?")
                    .text_style(TextStyle::Heading)
                    .color(scheme.on_surface),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{files} files ({tasks} tasks), {}",
                    engine::human(bytes_total)
                ))
                .color(scheme.on_surface),
            );
            ui.label(
                RichText::new(format!("to {dest}")).color(scheme.on_surface_variant),
            );
            ui.label(
                RichText::new(format!(
                    "Verify: {} · Conflicts: {} · Limit: {}",
                    verify_label(opts.verify),
                    conflict_label(opts.conflict),
                    if opts.limit_bps > 0 {
                        format!("{} MiB/s", opts.limit_bps >> 20)
                    } else {
                        "unlimited".into()
                    }
                ))
                .small()
                .color(scheme.on_surface_variant),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ketra_md3::filled_button(ui, "Start transfer").clicked() {
                    go = true;
                }
                if ketra_md3::text_button(ui, "Cancel").clicked() {
                    abort = true;
                }
            });
        });

        if go {
            self.start_engine(plan, opts);
        } else if abort {
            self.push_log(LogKind::Info, "canceled at confirm".into());
            self.run = RunState::Idle;
            self.status = Some("Ready".into());
        }
    }
}

impl eframe::App for KetraApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.handle_drops(ctx);
        self.pump();
        self.tick_throughput();

        // Keep frames flowing while anything can change (engine events,
        // indeterminate progress, EMA updates).
        let busy = !matches!(self.run, RunState::Idle);
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        self.ui(ctx);
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn file_name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn verify_label(v: VerifyLevel) -> &'static str {
    match v {
        VerifyLevel::Off => "off",
        VerifyLevel::QuickScan => "quick",
        VerifyLevel::Inline => "inline",
        VerifyLevel::FullHash => "full",
    }
}

fn conflict_label(c: ConflictPolicy) -> &'static str {
    match c {
        ConflictPolicy::Overwrite => "Overwrite",
        ConflictPolicy::OverwriteIfNewer => "Overwrite if newer",
        ConflictPolicy::Skip => "Skip existing",
        ConflictPolicy::Rename => "Auto-rename",
        ConflictPolicy::Ask => "Ask (CLI only)",
    }
}

fn fmt_eta(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "—".into();
    }
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn fmt_duration_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}h{:02}m{:02}s", s / 3600, (s % 3600) / 60, s % 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}.{:02}s", ms % 1000 / 10)
    }
}

/// Open the OS file manager on `path` (folder), with zero extra dependencies.
fn reveal(path: &str) {
    let p = path.trim();
    if p.is_empty() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

// ---------------------------------------------------------------------------
// tests (headless-safe logic)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_formatting() {
        assert_eq!(fmt_eta(0.0), "—");
        assert_eq!(fmt_eta(-5.0), "—");
        assert_eq!(fmt_eta(65.0), "1:05");
        assert_eq!(fmt_eta(3661.0), "1:01:01");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(fmt_duration_ms(0), "0.00s");
        assert_eq!(fmt_duration_ms(1250), "1.25s");
        assert_eq!(fmt_duration_ms(61_000), "1m01s");
        assert_eq!(fmt_duration_ms(3_661_000), "1h01m01s");
    }

    #[test]
    fn file_name_helper() {
        assert_eq!(file_name_of(PathBuf::from("/a/b/c.txt").as_path()), "c.txt");
        assert_eq!(file_name_of(PathBuf::from("/").as_path()), "/");
    }
}
