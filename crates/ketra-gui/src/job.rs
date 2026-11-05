//! Spawning and supervising the ketra-core engine from the GUI.
//!
//! The UI thread never blocks on the engine: planning runs on its own
//! thread, the engine runs on another, and all communication happens over
//! `std::sync::mpsc` channels plus the shared [`Controller`].

use ketra_core::plan::TransferPlan;
use ketra_core::progress::Event;
use ketra_core::{Controller, Op, Summary, TransferOptions};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// App-level messages from background threads to the UI thread.
pub enum AppMsg {
    /// `build_plan` finished a dry-run scan; ask the user to confirm.
    PlanReady(TransferPlan),
    /// `build_plan` failed (bad path, dest-inside-source guard, …).
    PlanFailed(String),
}

/// Live handles for a running transfer.
pub struct EngineJob {
    pub ctrl: Controller,
    pub events: Receiver<Event>,
    /// One-shot summary; `Err(Disconnected)` means the thread died.
    pub done: Receiver<Summary>,
}

/// Build the transfer plan on a worker thread (Phase 1 scan).
pub fn plan_async(
    sources: Vec<PathBuf>,
    dest: PathBuf,
    opts: TransferOptions,
    tx: Sender<AppMsg>,
) {
    std::thread::Builder::new()
        .name("ketra-plan".into())
        .spawn(move || match ketra_core::build_plan(&sources, &dest, Op::Copy, &opts) {
            Ok(plan) => {
                let _ = tx.send(AppMsg::PlanReady(plan));
            }
            Err(e) => {
                let _ = tx.send(AppMsg::PlanFailed(e.to_string()));
            }
        })
        .expect("spawn planner thread");
}

/// Run the engine to completion on a worker thread (Phases 2–3).
///
/// Returns immediately; the UI pumps `events` every frame and reads the
/// final `Summary` from `done`.
pub fn start_engine(plan: TransferPlan, opts: TransferOptions) -> EngineJob {
    let (etx, events) = std::sync::mpsc::channel::<Event>();
    let (dtx, done) = std::sync::mpsc::channel::<Summary>();
    let ctrl = Controller::new();
    let thread_ctrl = ctrl.clone();
    std::thread::Builder::new()
        .name("ketra-engine".into())
        .spawn(move || {
            let summary = ketra_core::engine::run(&plan, &opts, &thread_ctrl, &etx);
            // If the window is closing the receiver is gone; ignore that.
            let _ = dtx.send(summary);
        })
        .expect("spawn engine thread");
    EngineJob { ctrl, events, done }
}
