# Clean-room methodology

## Statement

KetraCopy is an **independent, original implementation in Rust** of functionality made popular by TeraCopy (Code Sector). The project practices functional clean-room replication:

- The design was derived exclusively from **public, observable behavior**: the vendor's public feature listings and documentation, and behavior any user can observe by running a file copy — dialog options, progress display, exit behavior. No confidential information was used.
- **No decompilation or disassembly** of TeraCopy was performed, and no binary analysis of any kind was used to derive feature behavior or algorithms — true for **all work through v1.2.1**. (Post-1.2.1 optimization research is bounded by the S10 methodology disclosure below.)
- **No code, strings, icons, images, sounds, installer components, or other assets** from TeraCopy were copied, translated, or adapted. Every line in this repository — including the hashing implementations, JSON parser, glob matcher, and the hand-rolled argument parser — was written from scratch against public specifications and public OS documentation (cited in `docs/ALGORITHMS.md`).
- The name **"KetraCopy"** was chosen independently to avoid confusion. "TeraCopy" is a trademark of Code Sector; this project is **not affiliated with, endorsed by, or connected to** Code Sector in any way. This repository contains no Code Sector intellectual property.

## Post-1.2.1 reference study (S10) — methodology disclosure

By owner directive (2026-11-13), development after v1.2.1 may be informed
by a **reference study of TeraCopy 3.17 and 4.0.5** (development-harness
stage S10; decision record `harness/decisions/ADR-0003-s10-teracopy-reference-study.md`,
governed by the adversarial gate record
`harness/reviews/2026-11-13-gate-3-s10-charter.md`). Its methodology and
hard limits:

- **Track A — behavioral observation.** The vendor's software is run on
  the project owner's own Windows hardware under standard OS
  instrumentation (Process Monitor, ETW/xperf, API traces) — an extension
  of the "behavior any user can observe" rule above. KetraCopy is
  co-instrumented on the same rig so findings are comparative, never
  absolute.
- **Track B — static interface inventory.** The vendor's public installers
  are unpacked, and only **functional interface facts** are recorded:
  payload file inventory (names/sizes/versions), PE format metadata, and
  the names of operating-system APIs the binaries import.
- **What never changes**: no decompiled code, disassembly listings, vendor
  strings, resources, icons, screenshots, or binaries are copied into this
  repository or its products; vendor material stays outside the repo;
  every recorded finding carries a provenance tag — [A] (trace-derived,
  citing the observing instrumentation) or [B] (inventory-derived) — and
  facts that cannot be attributed to a tagged observation are excluded;
  all KetraCopy implementation continues to be written from scratch
  against the recorded functional notes. If analysis deeper than the
  disclosed methodology is ever performed by the owner privately, its
  output does not inform this implementation.
- Every study document opens with a scope header stating what it is and
  is not (`docs/research/`).

## Feature parity (summary)

The full, detailed matrix lives in `docs/FEATURE-PARITY.md`. Condensed mapping of publicly known TeraCopy functionality to KetraCopy implementations:

| TeraCopy feature (public) | KetraCopy implementation |
|---|---|
| Copy / Move / Test operations | `crates/ketra-core/src/options.rs` (`Op`), `pipeline.rs::run_file_task`, CLI `copy`/`move`/`test` |
| Verification (size/time, byte-compare, hash) | `pipeline.rs` phases 1-2, `options.rs::VerifyLevel` |
| Pause / cancel | `controller.rs` (`Controller`), engine chunk gates; CLI wires Ctrl+C via `crates/ketra-cli/src/sig.rs` |
| Resume interrupted transfers | `journal.rs` + `resume.rs` + `engine.rs` (journal + prefix probe) |
| Skip file / retry failed | `controller.rs` skip flags, `pipeline.rs` retry loop with backoff, `--retries` |
| Error list / report | `engine.rs::Summary`/`ErrorRecord`, `--report` JSON in `run_cmd.rs` |
| Speed control | `throttle.rs` token bucket + `Controller::set_speed_limit`, `--limit` |
| Conflict policies (overwrite/newer/skip/rename/ask) | `options.rs::ConflictPolicy`, `pipeline.rs::resolve_conflict`, interactive prompt in `run_cmd.rs` |
| Preserve timestamps / attributes | `pipeline.rs::finalize_metadata`, `platform/windows.rs::set_creation_time` |
| Include/exclude filters, file lists | `plan.rs` + `glob.rs`; CLI `--include`/`--exclude`, `--flatten` |
| Interactive GUI | In progress: `ketra-md3` design-system sources complete (`crates/ketra-md3/src`); `ketra-gui` shell scaffolded (`crates/ketra-gui`, eframe/glow); application UI not complete in this snapshot |
| Windows sharing-violation waiting | `pipeline.rs::open_read_with_wait`, `delete_source`; `--wait-lock` |
| Sparse file handling | `platform/linux.rs`/`platform/windows.rs` hole maps, `pipeline.rs` |
| Long path (260+ char) support | `platform/windows.rs::to_extended_path`, applied in `plan.rs::absolutize` |
| Checksum file export | Different approach: `.ketra/manifest.jsonl` (SHA-256) instead of `.md5`/`.sha256` sidecar files — `manifest.rs` |
| Explorer/Finder/file-manager integration, installers | Provided as scripts/definitions (`integrations/`, `packaging/`); syntax-checked, not executed on native OSes in this snapshot |

## Differences from TeraCopy

- **Manifest archival is an addition.** TeraCopy verifies transfers; KetraCopy additionally writes a durable SHA-256 manifest (`.ketra/manifest.jsonl`) and provides `ketra verify` for detecting bit rot years after the transfer. There is no TeraCopy-equivalent behavior being replicated here — it is original functionality.
- **ACL and alternate data stream (ADS) copying are not implemented.** NTFS ACLs and ADS are out of scope and listed as limitations; KetraCopy preserves mode bits/permissions and timestamps only (`pipeline.rs::finalize_metadata`).
- **Parallel ranged network mode is an addition** (TeraCopy has no equivalent public feature): `netmode.rs`, experimental.
- **Different checksum export format** (manifest JSONL vs. per-file `.md5`/`.sha256` sidecars).
- **CLI-first.** The runtime-verified surface in this snapshot is the engine and CLI; the GUI (`ketra-md3` + `ketra-gui`, eframe/egui with an MD3 theme) compiles and release-builds but its display was not exercised headless. GUI-only TeraCopy conveniences such as drag-drop and file-list editing are minimal or absent.

## License and disclaimer

KetraCopy is released under the MIT license (see `LICENSE` at the repository root; this documentation is part of the repository and carries the same license). The software is provided **AS IS**, without warranty of any kind, express or implied. Users are responsible for complying with all applicable laws and third-party rights when using this software. Nothing in this document constitutes legal advice.

## How parity was derived

The feature list in `docs/FEATURE-PARITY.md` was assembled from public sources only: the vendor's publicly published feature descriptions and help documentation, and behavior observable by any user of a file manager (what happens on copy, move, test, pause, skip, conflict prompts, and so on). No binary, installer, or registry content of TeraCopy was examined, decompiled, or disassembled at any point **through v1.2.1**. Where KetraCopy's approach differs from the original's (or adds to it), the parity table says so explicitly rather than implying equivalence. (Post-1.2.1 research is disclosed in the S10 section above and remains bounded by its provenance and no-copy rules.)
