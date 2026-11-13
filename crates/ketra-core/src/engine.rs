//! The engine: planning + the three-phase pipeline + move/test/verify.
//!
//! ```text
//! plan ──► Phase 1 fast scan (size+mtime) ──► Phase 2 stream copy
//!          (skip identical files)             (+inline verify, journal,
//!                                              clone/cfr fast paths)
//!                                             ──► Phase 3 SHA-256 manifest
//! ```

use crate::device;
use crate::error::{Error, Result};
use crate::journal::{self, Journal, JournalEntry, JournalHandle};
use crate::manifest;
use crate::netmode;
use crate::options::{Options, VerifyMode};
use crate::pipeline;
use crate::plan::{self, FileTask};
use crate::progress::{Event, Progress};
use crate::resume;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Upper bound for `Options::parallel` file workers.
const MAX_PARALLEL: u32 = 16;

/// What per-task preparation decided before the copy executes.
enum Prep {
    Skip,
    Exec { start_offset: u64 },
}

/// Sticky "all" answers from interactive conflict prompts.
#[derive(Default)]
struct StickyFlags {
    overwrite_all: bool,
    skip_all: bool,
}

/// A task ready for execution (conflicts resolved, resume offset known).
#[derive(Clone)]
struct PreparedTask {
    index: usize,
    src: PathBuf,
    dst: PathBuf,
    rel: String,
    size: u64,
    start_offset: u64,
}

/// Result of one finished file.
enum Accounted {
    Copied,
    Canceled,
    Failed,
}

/// What the interactive conflict callback decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictDecision {
    Skip,
    Overwrite,
    Rename,
    OverwriteAll,
    SkipAll,
    Cancel,
}

/// Aggregate result of a run.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub files_total: u32,
    pub files_copied: u32,
    pub files_skipped: u32,
    pub files_failed: u32,
    pub bytes_total: u64,
    pub bytes_copied: u64,
    pub files_cloned: u32,
    pub sparse_bytes: u64,
    pub used_cfr: bool,
    pub manifest: Option<PathBuf>,
    pub duration: Duration,
}

/// Result of `ketra verify`.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub total: u32,
    pub ok: u32,
    pub missing: u32,
    pub size_mismatch: u32,
    pub checksum_mismatch: u32,
}

/// Callback bundle for a run.
pub struct Engine<'a> {
    pub opts: &'a Options,
    pub cancel: &'a AtomicBool,
    pub on_event: &'a mut dyn FnMut(&Event),
    pub on_conflict: &'a mut dyn FnMut(&FileTask) -> ConflictDecision,
}

impl<'a> Engine<'a> {
    pub fn new(
        opts: &'a Options,
        cancel: &'a AtomicBool,
        on_event: &'a mut dyn FnMut(&Event),
        on_conflict: &'a mut dyn FnMut(&FileTask) -> ConflictDecision,
    ) -> Self {
        Engine { opts, cancel, on_event, on_conflict }
    }

    /// Copy (or move) `sources` into `dst`.
    pub fn copy(&mut self, sources: &[PathBuf], dst: &Path, move_files: bool) -> Result<Summary> {
        let t0 = Instant::now();
        let mut plan = plan::build(sources, dst, self.opts)?;
        let mut sum = Summary {
            files_total: plan.tasks.len() as u32,
            bytes_total: plan.total_bytes,
            ..Default::default()
        };

        (self.on_event)(&Event::Started {
            total_files: sum.files_total,
            total_bytes: plan.total_bytes,
            phase: if move_files { "move" } else { "copy" },
        });

        if self.opts.dry_run {
            for (i, t) in plan.tasks.iter().enumerate() {
                (self.on_event)(&Event::FileStart {
                    path: t.rel.clone(),
                    size: t.size,
                    index: i as u32,
                    total_files: sum.files_total,
                });
                (self.on_event)(&Event::FileDone {
                    path: t.rel.clone(),
                    bytes: t.size,
                    action: "planned",
                    verified: false,
                });
            }
            (self.on_event)(&Event::Message(format!(
                "dry-run: {} file(s), {} bytes planned",
                plan.tasks.len(),
                plan.total_bytes
            )));
            sum.duration = t0.elapsed();
            (self.on_event)(&Event::Finished {
                files_copied: 0,
                files_skipped: 0,
                files_failed: 0,
                bytes_copied: 0,
                elapsed_secs: sum.duration.as_secs_f64(),
            });
            return Ok(sum);
        }

        // Volume capacity check (best effort).
        if plan.total_bytes > 0 {
            if let Ok(free) = device::free_space(dst) {
                if free < plan.total_bytes {
                    (self.on_event)(&Event::Message(format!(
                        "warning: destination volume has {} free bytes, plan needs {}",
                        free, plan.total_bytes
                    )));
                }
            }
        }

        // Create the mirrored directory tree up front.
        for d in &plan.dirs {
            let p = dst.join(d);
            std::fs::create_dir_all(&p).map_err(|e| Error::ioctx(&p, e))?;
        }

        // Resume journal (loaded once, appended during the run). Parallel
        // runs share one mutex-guarded writer; sequential runs own it.
        let parallel = self.opts.parallel.max(1).min(MAX_PARALLEL) as usize;
        let journal_path = dst.join(".ketracopy-journal.jsonl");
        let prior: std::collections::BTreeMap<String, (u64, bool)> =
            if self.opts.resume && journal_path.exists() {
                journal::latest_offsets(&journal_path).unwrap_or_default()
            } else {
                Default::default()
            };
        let jrn = JournalHandle::new(Journal::open(&journal_path)?);

        let mut prog = Progress::new();
        prog.total_files = sum.files_total;
        prog.total_bytes = plan.total_bytes;

        let mut sticky = StickyFlags::default();
        let mut copied_rels: Vec<(usize, String, Option<String>)> = Vec::new();

        if parallel == 1 {
            // -----------------------------------------------------------
            // Classic sequential pipeline (semantics unchanged since 1.0).
            // -----------------------------------------------------------
            'tasks: for (i, task) in plan.tasks.iter_mut().enumerate() {
                if self.cancel.load(Ordering::Relaxed) {
                    (self.on_event)(&Event::Canceled {
                        files_copied: sum.files_copied,
                        bytes_copied: sum.bytes_copied,
                    });
                    sum.duration = t0.elapsed();
                    return Err(Error::Canceled);
                }

                let start_offset = match self.prepare(task, dst, &mut sticky, &prior, &sum)? {
                    Prep::Skip => {
                        sum.files_skipped += 1;
                        prog.file_done();
                        continue;
                    }
                    Prep::Exec { start_offset } => start_offset,
                };

                // ---- Phase 2: stream copy ----
                (self.on_event)(&Event::FileStart {
                    path: task.rel.clone(),
                    size: task.size,
                    index: i as u32,
                    total_files: sum.files_total,
                });

                let mut last_emit = Instant::now();
                let file_index = i as u32;
                let rel_path = task.rel.clone();
                let mut progress = |delta: u64| {
                    prog.add_bytes(delta);
                    if last_emit.elapsed() >= Duration::from_millis(100) {
                        last_emit = Instant::now();
                        (self.on_event)(&Event::FileProgress {
                            path: rel_path.clone(),
                            copied: 0, // filled by the engine aggregate below
                            size: task.size,
                            file_index,
                            total_copied: prog.copied_bytes,
                            total_bytes: prog.total_bytes,
                            speed_bps: prog.speed_bps(),
                            eta_secs: prog.eta_secs(),
                        });
                    }
                };

                let mut ctl = crate::controller::SpeedController::new(self.opts.speed_limit_bps);
                // Inline SHA-256 during the transfer: for fresh full copies
                // the manifest digest is computed on the fly and Phase 3
                // reuses it instead of re-reading every file (one full read
                // less per hashed file). Resumed files, reflink clones and
                // netmode ranges still hash via a Phase 3 read.
                let hash_inline = self.opts.verify == VerifyMode::Manifest
                    && start_offset == 0
                    && self.opts.net_streams < 2;
                let result = if hash_inline {
                    match crate::hash::by_name(crate::options::MANIFEST_ALGO) {
                        Some(mut h) => {
                            let mut r = pipeline::copy_file(
                                &task.src,
                                &task.dst,
                                self.opts,
                                &mut ctl,
                                self.cancel,
                                Some(&jrn),
                                &task.rel,
                                start_offset,
                                &mut progress,
                                Some(&mut *h),
                            );
                            // A cloned destination never passed its bytes
                            // through the hasher: its digest would be the
                            // empty-input hash — leave it unset and let
                            // Phase 3 hash the file.
                            if r.as_ref().map(|s| !s.cloned).unwrap_or(false) {
                                let d = h.finish_hex();
                                if let Ok(s) = &mut r {
                                    s.hash_digest = Some(d);
                                }
                            }
                            r
                        }
                        None => pipeline::copy_file(
                            &task.src,
                            &task.dst,
                            self.opts,
                            &mut ctl,
                            self.cancel,
                            Some(&jrn),
                            &task.rel,
                            start_offset,
                            &mut progress,
                            None,
                        ),
                    }
                } else if self.opts.net_streams >= 2 {
                    netmode::copy_parallel(&task.src, &task.dst, self.opts, self.cancel, &mut progress)
                } else {
                    pipeline::copy_file(
                        &task.src,
                        &task.dst,
                        self.opts,
                        &mut ctl,
                        self.cancel,
                        Some(&jrn),
                        &task.rel,
                        start_offset,
                        &mut progress,
                        None,
                    )
                };
                let meta = PreparedTask {
                    index: i,
                    src: task.src.clone(),
                    dst: task.dst.clone(),
                    rel: task.rel.clone(),
                    size: task.size,
                    start_offset,
                };
                match self.account_result(result, &meta, &mut sum, move_files) {
                    (Accounted::Copied, digest) => {
                        prog.file_done();
                        copied_rels.push((i, task.rel.clone(), digest));
                    }
                    (Accounted::Failed, _) => {}
                    (Accounted::Canceled, _) => {
                        (self.on_event)(&Event::Canceled {
                            files_copied: sum.files_copied,
                            bytes_copied: sum.bytes_copied,
                        });
                        sum.duration = t0.elapsed();
                        return Err(Error::Canceled);
                    }
                }
            }
        } else {
            // -----------------------------------------------------------
            // Parallel file-level pipeline: N workers pull prepared tasks,
            // so the per-file fsync of one file overlaps the transfer of
            // the next — the dominant win on many-small-file workloads.
            // Conflicts, resume probing and UI callbacks stay on the main
            // thread; workers only execute already-resolved tasks.
            // -----------------------------------------------------------
            let per_limit = self.opts.speed_limit_bps.map(|l| (l / parallel as u64).max(1));
            let shared_prog = Arc::new(AtomicU64::new(0));
            let (task_tx, task_rx) = mpsc::channel::<PreparedTask>();
            let (res_tx, res_rx) = mpsc::channel::<(usize, Result<crate::pipeline::CopyStats>)>();
            let task_rx = Mutex::new(task_rx);

            let mut meta: Vec<Option<PreparedTask>> =
                (0..plan.tasks.len()).map(|_| None).collect();
            let mut in_flight = 0usize;
            let mut canceled_seen = false;
            let mut prep_err: Option<Error> = None;

            std::thread::scope(|scope| {
                for _ in 0..parallel {
                    let task_rx = &task_rx;
                    let res_tx = res_tx.clone();
                    let jrn_ref = &jrn;
                    let opts = self.opts;
                    let cancel = self.cancel;
                    let shared_prog = Arc::clone(&shared_prog);
                    scope.spawn(move || {
                        loop {
                            let prep = match task_rx.lock().unwrap().recv() {
                                Ok(p) => p,
                                Err(_) => break, // all task senders dropped
                            };
                            let mut ctl =
                                crate::controller::SpeedController::new(per_limit);
                            // Same inline-hash rule as the sequential path.
                            let hash_inline = opts.verify == VerifyMode::Manifest
                                && prep.start_offset == 0
                                && opts.net_streams < 2;
                            let acc = Arc::clone(&shared_prog);
                            let mut progress = move |d: u64| {
                                acc.fetch_add(d, Ordering::Relaxed);
                            };
                            let r = if hash_inline {
                                match crate::hash::by_name(crate::options::MANIFEST_ALGO) {
                                    Some(mut h) => {
                                        let mut rr = pipeline::copy_file(
                                            &prep.src,
                                            &prep.dst,
                                            opts,
                                            &mut ctl,
                                            cancel,
                                            Some(jrn_ref),
                                            &prep.rel,
                                            prep.start_offset,
                                            &mut progress,
                                            Some(&mut *h),
                                        );
                                        if rr.as_ref().map(|s| !s.cloned).unwrap_or(false) {
                                            let d = h.finish_hex();
                                            if let Ok(s) = &mut rr {
                                                s.hash_digest = Some(d);
                                            }
                                        }
                                        rr
                                    }
                                    None => pipeline::copy_file(
                                        &prep.src,
                                        &prep.dst,
                                        opts,
                                        &mut ctl,
                                        cancel,
                                        Some(jrn_ref),
                                        &prep.rel,
                                        prep.start_offset,
                                        &mut progress,
                                        None,
                                    ),
                                }
                            } else if opts.net_streams >= 2 {
                                netmode::copy_parallel(
                                    &prep.src, &prep.dst, opts, cancel, &mut progress,
                                )
                            } else {
                                pipeline::copy_file(
                                    &prep.src,
                                    &prep.dst,
                                    opts,
                                    &mut ctl,
                                    cancel,
                                    Some(jrn_ref),
                                    &prep.rel,
                                    prep.start_offset,
                                    &mut progress,
                                    None,
                                )
                            };
                            let _ = res_tx.send((prep.index, r));
                        }
                    });
                }
                drop(res_tx); // the main thread holds no result sender

                'dispatch: for (i, task) in plan.tasks.iter_mut().enumerate() {
                    if self.cancel.load(Ordering::Relaxed) {
                        break 'dispatch;
                    }
                    let start_offset = match self.prepare(task, dst, &mut sticky, &prior, &sum) {
                        Ok(Prep::Exec { start_offset }) => start_offset,
                        Ok(Prep::Skip) => {
                            sum.files_skipped += 1;
                            prog.file_done();
                            continue;
                        }
                        Err(e) => {
                            prep_err = Some(e);
                            break 'dispatch;
                        }
                    };

                    let pt = PreparedTask {
                        index: i,
                        src: task.src.clone(),
                        dst: task.dst.clone(),
                        rel: task.rel.clone(),
                        size: task.size,
                        start_offset,
                    };
                    (self.on_event)(&Event::FileStart {
                        path: task.rel.clone(),
                        size: task.size,
                        index: i as u32,
                        total_files: sum.files_total,
                    });
                    meta[i] = Some(pt);
                    if task_tx.send(PreparedTask {
                        index: i,
                        src: meta[i].as_ref().unwrap().src.clone(),
                        dst: meta[i].as_ref().unwrap().dst.clone(),
                        rel: meta[i].as_ref().unwrap().rel.clone(),
                        size: meta[i].as_ref().unwrap().size,
                        start_offset: meta[i].as_ref().unwrap().start_offset,
                    }).is_err() {
                        break 'dispatch;
                    }
                    in_flight += 1;

                    // Bounded backlog: never let queued tasks outrun the
                    // workers by more than a small constant.
                    while in_flight >= parallel * 4 {
                        match res_rx.recv_timeout(Duration::from_millis(50)) {
                            Ok((idx, r)) => {
                                in_flight -= 1;
                                if let Some(m) = meta[idx].take() {
                                    match self.account_result(r, &m, &mut sum, move_files) {
                                        (Accounted::Copied, digest) => {
                                            prog.file_done();
                                            copied_rels.push((m.index, m.rel.clone(), digest));
                                        }
                                        (Accounted::Failed, _) => {}
                                        (Accounted::Canceled, _) => canceled_seen = true,
                                    }
                                }
                            }
                            Err(_) => {
                                // Timeout: nothing finished this tick; keep
                                // the dispatch loop responsive to cancel.
                            }
                        }
                        if canceled_seen {
                            break 'dispatch;
                        }
                    }
                }

                // Stop accepting new work; workers finish the backlog.
                drop(task_tx);
                while in_flight > 0 {
                    match res_rx.recv() {
                        Ok((idx, r)) => {
                            in_flight -= 1;
                            if let Some(m) = meta[idx].take() {
                                match self.account_result(r, &m, &mut sum, move_files) {
                                    (Accounted::Copied, digest) => {
                                        prog.file_done();
                                        copied_rels.push((m.index, m.rel.clone(), digest));
                                    }
                                    (Accounted::Failed, _) => {}
                                    (Accounted::Canceled, _) => canceled_seen = true,
                                }
                            }
                        }
                        Err(_) => break, // workers gone without results
                    }
                }
            });

            if let Some(e) = prep_err {
                return Err(e);
            }
            if canceled_seen || self.cancel.load(Ordering::Relaxed) {
                (self.on_event)(&Event::Error {
                    path: String::new(),
                    message: "canceled \u{2014} partial files kept for resume".into(),
                });
                (self.on_event)(&Event::Canceled {
                    files_copied: sum.files_copied,
                    bytes_copied: sum.bytes_copied,
                });
                sum.duration = t0.elapsed();
                return Err(Error::Canceled);
            }
        }


        // ---- Phase 3: archive manifest ----
        if self.opts.verify == VerifyMode::Manifest && !copied_rels.is_empty() {
            (self.on_event)(&Event::Message("phase 3: building SHA-256 manifest".into()));
            // Deterministic manifest order regardless of worker completion
            // order (parallel mode finishes files out of plan order).
            copied_rels.sort_by_key(|(i, _, _)| *i);
            let files: Vec<PathBuf> = copied_rels.iter().map(|(_, r, _)| dst.join(r)).collect();
            // Digests computed inline during the transfer are reused here;
            // files without one (resumed, cloned, netmode) are re-read.
            let mut digests = std::collections::BTreeMap::new();
            for (_, rel, d) in &copied_rels {
                if let Some(d) = d {
                    digests.insert(rel.clone(), d.clone());
                }
            }
            let entries = manifest::scan_with_digests(dst, Some(&files), &digests, self.cancel)?;
            let mpath = match &self.opts.manifest_path {
                Some(p) => PathBuf::from(p),
                None => dst.join(".ketra-manifest.json"),
            };
            let man = manifest::Manifest {
                algorithm: crate::options::MANIFEST_ALGO.into(),
                root: dst.display().to_string(),
                files: entries,
            };
            manifest::write_file(&mpath, &man)?;
            sum.manifest = Some(mpath.clone());
            (self.on_event)(&Event::Message(format!(
                "manifest written: {} ({} entries)",
                mpath.display(),
                man.files.len()
            )));
        }

        // Clean journal only on fully successful, uncanceled runs.
        if sum.files_failed == 0 && !self.cancel.load(Ordering::Relaxed) {
            jrn.finish()?;
        }

        sum.duration = t0.elapsed();
        (self.on_event)(&Event::Finished {
            files_copied: sum.files_copied,
            files_skipped: sum.files_skipped,
            files_failed: sum.files_failed,
            bytes_copied: sum.bytes_copied,
            elapsed_secs: sum.duration.as_secs_f64(),
        });
        Ok(sum)
    }

    /// Resolve one task up to the point where it can be executed: fast
    /// scan, resume probing, conflict policy. All UI callbacks stay here so
    /// parallel workers only ever execute fully-resolved tasks.
    fn prepare(
        &mut self,
        task: &mut FileTask,
        dst: &Path,
        sticky: &mut StickyFlags,
        prior: &std::collections::BTreeMap<String, (u64, bool)>,
        sum: &Summary,
    ) -> Result<Prep> {
        let dst_path = task.dst.clone();
        let exists = dst_path.symlink_metadata().is_ok();

        // ---- Phase 1: fast scan (size + mtime) ----
        if exists {
            if let Ok(dmd) = std::fs::metadata(&dst_path) {
                let same_size = dmd.len() == task.size;
                // Without mtime preservation the destination mtime is
                // copy-time and carries no source information: a size match
                // alone must never shortcut the copy (a same-size MODIFIED
                // source would be silently skipped). Only a genuine mtime
                // match may fast-skip; everything else falls through to the
                // conflict policy.
                let same_mtime = self.opts.preserve_mtime && mtime_secs(&dmd) == task.mtime;
                if same_size && same_mtime && task.size > 0 {
                    (self.on_event)(&Event::FileSkip {
                        path: task.rel.clone(),
                        reason: "up_to_date",
                    });
                    return Ok(Prep::Skip);
                }
            }
        }

        // ---- resume offset (bypasses conflict skipping when > 0) ----
        let mut start_offset: u64 = 0;
        let mut resuming = false;
        if self.opts.resume {
            if let Some((joff, jdone)) = prior.get(&task.rel).copied() {
                if jdone && joff >= task.size {
                    if exists && std::fs::metadata(&dst_path).map(|m| m.len()).unwrap_or(0) == task.size {
                        (self.on_event)(&Event::FileSkip {
                            path: task.rel.clone(),
                            reason: "already_done",
                        });
                        return Ok(Prep::Skip);
                    }
                }
                // Journal says work was done: probe the real extent.
                // (offset > 0 bypasses the conflict policy by design)
                start_offset = resume::probe_resume(
                    &task.src,
                    &dst_path,
                    Some(joff),
                    self.opts.chunk(),
                    self.cancel,
                )?;
                resuming = start_offset > 0 || joff > 0;
            }
        }

        // ---- conflict policy ----
        if exists && !resuming {
            let raw = if sticky.overwrite_all {
                ConflictDecision::Overwrite
            } else if sticky.skip_all {
                ConflictDecision::Skip
            } else {
                match self.opts.conflict {
                    crate::options::OnConflict::Ask => (self.on_conflict)(task),
                    crate::options::OnConflict::Skip => ConflictDecision::Skip,
                    crate::options::OnConflict::Overwrite => ConflictDecision::Overwrite,
                    crate::options::OnConflict::Rename => ConflictDecision::Rename,
                    crate::options::OnConflict::KeepNewer => {
                        let src_newer = task.mtime > dst_mtime(&dst_path);
                        if src_newer {
                            ConflictDecision::Overwrite
                        } else {
                            ConflictDecision::Skip
                        }
                    }
                }
            };
            let decision = match raw {
                ConflictDecision::OverwriteAll => {
                    sticky.overwrite_all = true;
                    ConflictDecision::Overwrite
                }
                ConflictDecision::SkipAll => {
                    sticky.skip_all = true;
                    ConflictDecision::Skip
                }
                d => d,
            };
            match decision {
                ConflictDecision::Skip => {
                    (self.on_event)(&Event::FileSkip {
                        path: task.rel.clone(),
                        reason: "conflict_skip",
                    });
                    return Ok(Prep::Skip);
                }
                ConflictDecision::Cancel => {
                    (self.on_event)(&Event::Canceled {
                        files_copied: sum.files_copied,
                        bytes_copied: sum.bytes_copied,
                    });
                    return Err(Error::Canceled);
                }
                ConflictDecision::Rename => {
                    let renamed = unique_sibling(&dst_path);
                    if let Ok(new_rel) = renamed.strip_prefix(dst) {
                        task.rel = new_rel.to_string_lossy().replace('\\', "/");
                    }
                    task_dst_override(task, &renamed);
                }
                ConflictDecision::Overwrite => {}
                // Normalized above; cannot reach here.
                ConflictDecision::OverwriteAll | ConflictDecision::SkipAll => {}
            }
        }
        Ok(Prep::Exec { start_offset })
    }

    /// Post-copy accounting shared by both pipelines: counters, events,
    /// move-source removal. Returns the accounting verdict plus the inline
    /// manifest digest when the transfer produced one. The caller handles
    /// progress bookkeeping and the copied list, and decides how a
    /// cancellation terminates the run.
    fn account_result(
        &mut self,
        res: Result<crate::pipeline::CopyStats>,
        meta: &PreparedTask,
        sum: &mut Summary,
        move_files: bool,
    ) -> (Accounted, Option<String>) {
        match res {
            Ok(stats) => {
                let digest = stats.hash_digest.clone();
                if stats.cloned {
                    sum.files_cloned += 1;
                }
                if stats.used_cfr {
                    sum.used_cfr = true;
                }
                sum.sparse_bytes += stats.sparse_bytes;
                sum.files_copied += 1;
                sum.bytes_copied += stats.bytes;
                (self.on_event)(&Event::FileDone {
                    path: meta.rel.clone(),
                    bytes: meta.size,
                    action: if stats.cloned {
                        "cloned"
                    } else if meta.start_offset > 0 {
                        "resumed"
                    } else {
                        "copied"
                    },
                    verified: self.opts.verify == VerifyMode::Inline,
                });
                if move_files {
                    if let Err(e) = std::fs::remove_file(&meta.src) {
                        (self.on_event)(&Event::Error {
                            path: meta.rel.clone(),
                            message: format!("copied, but source remove failed: {e}"),
                        });
                    }
                }
                (Accounted::Copied, digest)
            }
            Err(e) if e.is_canceled() => {
                // Cancel must NOT fall through to any truncating/cleanup
                // path — the partial file and its journal records stay for
                // --resume.
                (self.on_event)(&Event::Error {
                    path: meta.rel.clone(),
                    message: "canceled \u{2014} partial file kept for resume".into(),
                });
                (Accounted::Canceled, None)
            }
            Err(e) => {
                sum.files_failed += 1;
                (self.on_event)(&Event::Error {
                    path: meta.rel.clone(),
                    message: e.to_string(),
                });
                (Accounted::Failed, None)
            }
        }
    }

    /// Read-test sources (throughput + optional hash).
    pub fn test(&mut self, sources: &[PathBuf], algo: &str) -> Result<Summary> {
        let t0 = Instant::now();
        let plan = plan::build(sources, Path::new("/dev/null"), &Options {
            flatten: false,
            ..Default::default()
        })?;
        let mut sum = Summary {
            files_total: plan.tasks.len() as u32,
            bytes_total: plan.total_bytes,
            ..Default::default()
        };
        (self.on_event)(&Event::Started {
            total_files: sum.files_total,
            total_bytes: plan.total_bytes,
            phase: "test",
        });
        let mut prog = Progress::new();
        prog.total_bytes = plan.total_bytes;
        for (i, t) in plan.tasks.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Error::Canceled);
            }
            (self.on_event)(&Event::FileStart {
                path: t.rel.clone(),
                size: t.size,
                index: i as u32,
                total_files: sum.files_total,
            });
            let mut last = 0u64;
            let mut last_emit = Instant::now();
            let res = crate::options::hash_file_with_progress(
                &t.src,
                algo,
                1024 * 1024,
                self.cancel,
                &mut |n| {
                    last += n;
                    prog.add_bytes(n);
                    if last_emit.elapsed() >= Duration::from_millis(100) {
                        last_emit = Instant::now();
                        (self.on_event)(&Event::FileProgress {
                            path: t.rel.clone(),
                            copied: last,
                            size: t.size,
                            file_index: i as u32,
                            total_copied: prog.copied_bytes,
                            total_bytes: prog.total_bytes,
                            speed_bps: prog.speed_bps(),
                            eta_secs: prog.eta_secs(),
                        });
                    }
                },
            );
            match res {
                Ok(digest) => {
                    sum.files_copied += 1;
                    sum.bytes_copied += t.size;
                    prog.file_done();
                    (self.on_event)(&Event::FileDone {
                        path: t.rel.clone(),
                        bytes: t.size,
                        action: "read_ok",
                        verified: true,
                    });
                    (self.on_event)(&Event::Message(format!("{} {algo}={digest}", t.rel)));
                }
                Err(e) => {
                    sum.files_failed += 1;
                    (self.on_event)(&Event::Error {
                        path: t.rel.clone(),
                        message: e.to_string(),
                    });
                }
            }
        }
        sum.duration = t0.elapsed();
        (self.on_event)(&Event::Finished {
            files_copied: sum.files_copied,
            files_skipped: sum.files_skipped,
            files_failed: sum.files_failed,
            bytes_copied: sum.bytes_copied,
            elapsed_secs: sum.duration.as_secs_f64(),
        });
        Ok(sum)
    }

    /// Verify a destination tree against a manifest file.
    pub fn verify(&mut self, root: &Path, manifest_path: &Path) -> Result<VerifyReport> {
        let man = manifest::load(manifest_path)?;
        (self.on_event)(&Event::Started {
            total_files: man.files.len() as u32,
            total_bytes: man.files.iter().map(|f| f.size).sum(),
            phase: "verify",
        });
        let statuses = manifest::verify(root, &man, self.cancel)?;
        let mut rep = VerifyReport { total: statuses.len() as u32, ..Default::default() };
        for (path, st) in &statuses {
            match st {
                manifest::VerifyStatus::Ok => rep.ok += 1,
                manifest::VerifyStatus::Missing => {
                    rep.missing += 1;
                    (self.on_event)(&Event::Error {
                        path: path.clone(),
                        message: "missing".into(),
                    });
                }
                manifest::VerifyStatus::SizeMismatch { expected, actual } => {
                    rep.size_mismatch += 1;
                    (self.on_event)(&Event::Error {
                        path: path.clone(),
                        message: format!("size mismatch: expected {expected}, found {actual}"),
                    });
                }
                manifest::VerifyStatus::ChecksumMismatch { expected, actual } => {
                    rep.checksum_mismatch += 1;
                    (self.on_event)(&Event::Error {
                        path: path.clone(),
                        message: format!("checksum mismatch: expected {expected}, found {actual}"),
                    });
                }
            }
        }
        Ok(rep)
    }
}

/// Journal entry helper used by tests and the GUI shell.
pub fn journal_entry(rel: &str, size: u64, chunk: u64, offset: u64, done: bool) -> JournalEntry {
    JournalEntry {
        rel: rel.to_string(),
        size,
        chunk,
        offset,
        status: if done { "done" } else { "partial" }.to_string(),
    }
}

fn mtime_secs(md: &std::fs::Metadata) -> i64 {
    use std::time::UNIX_EPOCH;
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn dst_mtime(p: &Path) -> i64 {
    std::fs::metadata(p).map(|m| mtime_secs(&m)).unwrap_or(0)
}

/// Find a free `name (2).ext`-style sibling.
fn unique_sibling(p: &Path) -> PathBuf {
    let parent = p.parent().unwrap_or(Path::new("."));
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for i in 2..10000 {
        let cand = parent.join(format!("{stem} ({i}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    parent.join(format!("{stem}.dup{ext}"))
}

/// Override a task's destination (rename conflicts) in place.
fn task_dst_override(task: &mut FileTask, new_dst: &Path) {
    task.dst = new_dst.to_path_buf();
}
