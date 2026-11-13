# Feature parity: TeraCopy 3.17/4.0.5 vs KetraCopy

Statuses: **implemented** (works as described, with code pointer), **partial** (subset of the observable behavior), **different approach** (goal met, mechanism differs), **not implemented** (absent from this snapshot; pointers to reserved placeholders where applicable). Every pointer refers to code that exists in this repository. Parity was derived from public behavior only — see `docs/CLEANROOM.md`.

| Feature | TeraCopy 3.17/4.0.5 (public behavior) | KetraCopy status | Where in this repo |
|---|---|---|---|
| Copy | Copy files/directories with list + progress | Implemented | `crates/ketra-core/src/plan.rs` (`build_plan`), `pipeline.rs`, `engine.rs`; CLI `ketra copy` (`crates/ketra-cli/src/run_cmd.rs`) |
| Move | Move with same-volume rename optimization | Implemented | `Op::Move` in `options.rs`; rename fast path + copy-then-delete in `pipeline.rs::copy_inner`/`delete_source`; `delete_failed` outcome reported if source unlink fails |
| Test / verify | Test copied files against sources | Implemented | `ketra test` (`Op::Test`, `pipeline.rs::verify_existing`); `--verify off/quick/inline/full` (`options.rs::VerifyLevel`); inline byte-compare in `pipeline.rs::stream_copy` |
| Pause | Pause a running transfer | Implemented (engine API) — no CLI key binding in this snapshot | `crates/ketra-core/src/controller.rs` (`pause`, `wait_if_paused`); chunk gates in `pipeline.rs::gate`; GUI view shows PAUSED (`crates/ketra-cli/src/view.rs`) |
| Resume | Resume interrupted transfers | Implemented | `journal.rs` (`.ketra/journal.jsonl`), `resume.rs` (prefix probe), wiring in `engine.rs::run`; `--no-resume`/`--no-journal` to disable. Verified by integration test 7 |
| Cancel | Cancel, keep partial files | Implemented | `controller.rs::cancel`; Ctrl+C via `crates/ketra-cli/src/sig.rs`; partials + journal preserved (`pipeline.rs::log_partial`); exit code 2 |
| Skip file | Skip the current problematic file | Implemented (engine API) — no CLI key binding in this snapshot | `controller.rs::skip_current`/`skip_task`; per-task skip flags checked in `pipeline.rs`/`netmode.rs` chunk loops |
| Retry failed | Retry failed files | Implemented | `pipeline.rs::run_file_task` retry loop (backoff 250 ms doubling), `--retries N` (default 3), retriable-kind classification in `error.rs` |
| Error list report | List of failed files after the run | Implemented | `engine.rs::Summary`/`ErrorRecord`; `TaskError` events in `progress.rs`; JSON `--report FILE` in `run_cmd.rs::write_report` |
| Speed control | Throttle transfer speed | Implemented | `throttle.rs` shared token bucket; `--limit 10M` (`args.rs`); live rate changes honored via `Controller::set_speed_limit` + `pipeline.rs::gate` |
| Overwrite policies: ask / overwrite / newer / skip / rename | Conflict dialog choices | Implemented | `options.rs::ConflictPolicy`; `pipeline.rs::resolve_conflict` (+ `unique_sibling` for rename); CLI `--conflict` with interactive prompt (`run_cmd.rs::prompt_conflict`, `-y` to force non-interactive) |
| Preserve timestamps | Keep modification times | Implemented | `pipeline.rs::finalize_metadata` (`FileTimes`, accessed+modified); creation-time copy on Windows (`platform/windows.rs::set_creation_time`); `--no-times` |
| Preserve attributes | Keep file attributes/permissions | Implemented (mode bits / read-only attribute; see ACL/ADS row) | `pipeline.rs::finalize_metadata` (unix mode, Windows permissions); `--no-attrs` |
| Directory filtering (include/exclude) | Filter by wildcards | Implemented | `glob.rs` (`*`, `**`, `?`, case-insensitive, basename matching); `--include`/`--exclude` (robocopy-style: include selects files, exclude prunes subtrees) in `plan.rs` |
| File list editing (add/remove rows in GUI) | GUI source list | Not implemented — the GUI is a scaffold in this snapshot; the CLI equivalent is the command-line source list + filters (`args.rs`) | `crates/ketra-cli/src/args.rs` (sources + `--include`/`--exclude`); GUI scaffold `crates/ketra-gui/` |
| Drag-and-drop shell integration | Drop files onto window | Not implemented. Context-menu integration is the planned substitute; its scaffolding exists but no scripts are committed yet | `integrations/{windows,macos,linux}/` (empty) |
| Explorer context menu (Windows) | Shell integration | Not implemented in this snapshot | `integrations/windows/` exists but is empty (no reg/PS scripts shipped) |
| Finder Quick Action (macOS) | Services integration | Not implemented in this snapshot | `integrations/macos/` exists but is empty |
| Nemo / Caja / Thunar / Nautilus / Dolphin menu entries | Linux file-manager actions | Not implemented in this snapshot | `integrations/linux/{nemo,nautilus,caja,thunar,dolphin}/` directories exist but are empty |
| Checksum file export | Write `.md5`/`.sha256` checksum files | Different approach | `.ketra/manifest.jsonl` (JSONL, SHA-256 + XXH64 per file) written by `manifest.rs::write`; consumed by `ketra verify`/`ketra manifest` (`crates/ketra-cli/src/manifest_cmd.rs`) |
| Close dialogs on completion | GUI auto-close option | Not implemented — the GUI shows a summary card instead of auto-closing | `crates/ketra-gui/src/app.rs` |
| Graphical interface (window, progress, buttons) | TeraCopy's primary interface | Implemented (compile-verified) — Material Design 3 design system on egui + `ketra-gui` shell (eframe/egui 0.28, glow renderer for Win7/macOS/Linux, rfd dialogs, live progress/pause/skip/cancel, summary card); display not exercised headless | `crates/ketra-md3/src/`, `crates/ketra-gui/src/` |
| Windows sharing-violation waiting | "Try again" / wait for locked files | Implemented | `pipeline.rs::open_read_with_wait` and `delete_source` (300 ms poll until `--wait-lock` deadline, default 60 s); `ERROR_SHARING_VIOLATION` classification in `error.rs` |
| Sparse file handling | — | Implemented (addition) | Hole maps (`platform/linux.rs::hole_ranges` SEEK_DATA/SEEK_HOLE, `platform/windows.rs` FSCTL ranges), digest zero-folding in `pipeline.rs::feed_zeros`; `--sparse on/off/auto` |
| Long path support (Win32, >260 chars) | Long-path handling | Implemented | `platform/windows.rs::to_extended_path` (`\\?\`, `\\?\UNC\`), applied in `plan.rs::absolutize` |
| Network multi-stream transfer | Not offered by TeraCopy | Implemented (experimental addition) | `crates/ketra-core/src/netmode.rs` (parallel contiguous ranges, shared token bucket, >= 64 MiB files, requires `--verify off/quick`), engaged from `pipeline.rs::copy_inner` |
| ACL copying (NTFS permissions) | Preserves ACLs (documented feature) | Not implemented — limitation | Only mode-bit/permission preservation exists (`pipeline.rs::finalize_metadata`) |
| Alternate data streams (NTFS ADS) | Copies ADS (documented feature) | Not implemented — limitation | Stream copy covers file data only (`pipeline.rs::stream_copy`) |

## Notes

- Rows marked "no CLI key binding": the engine control surface (pause, skip, live speed change) is complete and tested at the API level (`crates/ketra-core/src/controller.rs`), but this snapshot's CLI only binds Ctrl+C (cancel). A GUI would attach these to buttons.
- Rows pointing at `integrations/` or `packaging/`: those directory trees exist as scaffolding and are empty in this snapshot; no installer, Flatpak manifest, or shell-menu script has been committed yet (see `docs/BUILD.md`).
- The 3-phase verification pipeline (quick scan / inline compare / SHA-256 manifest) is KetraCopy's own design for TeraCopy's "verify after copy" behavior; it is documented in `docs/ARCHITECTURE.md` and measured by `scripts/test_integration.sh`.
