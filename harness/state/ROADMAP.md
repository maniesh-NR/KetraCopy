# KetraCopy — Roadmap & Gate Status

> Single source of truth for stage progress. Every stage transition
> requires a PASSED adversarial gate (`WORKFLOW.md` §2); gate records live
> in `../reviews/`.

| # | Stage | Status | Gate |
|---|---|---|---|
| S0 | 1.0.0 — core engine, CLI, MD3 GUI, packaging assets | done 2026-11-02 | pre-workflow |
| S1 | 1.1.0 — LAN transfer, clipboard engine, OS shell integration, Android track, GUI rebuild | done 2026-11-05 | pre-workflow |
| S2 | 1.1.1 — interop test campaign (106 checks) + fix sweep | done 2026-11-05 | pre-workflow |
| S3 | 1.2.0 — performance / stability / memory optimization pass | done 2026-11-05 | pre-workflow |
| S4 | Harness workflow + GitHub backup infrastructure (owner mandate 2026-11-13) | done 2026-11-13 | n/a (this stage IS the workflow) |
| S5 | Full re-verification of 1.2.0 — evidence pack (unit / integration / interop / bench / cross-checks) | done 2026-11-13 — findings handed to Gate 1: macOS-GUI objc2 break on modern rustc + packaging nits (`../evidence/2026-11-13-S5/README.md`) | — |
| S6 | **Gate 1** — adversarial consensus: work-so-far + roadmap | round 1 FAILED (Red Team BLOCK); **round 2 PASSED 2026-11-13** (`../reviews/2026-11-13-gate-1-round-2-and-gate-2.md`) | done |
| S7 | Address Gate-1 findings; rebuild release artifacts + hash baseline | done 2026-11-13 — all findings fixed with evidence; **new latent bug found + fixed** (`--conflict overwrite`/GUI overwrite-all EEXIST); version **1.2.1**; Windows+macOS binaries moved to native CI builds (single-producer rule) | done |
| S8 | **Gate 2** — release-readiness consensus | **PASSED 2026-11-13** (combined panel; all conditions applied pre-tag — see the round-2 record) | done |
| S9 | **Release v1.2.1** — tag, GitHub release page with packages for all targets (sandbox Linux/src assets + `release.yml` native Windows/macOS builds), release notes + known limitations + all-asset SHA-256 table, topics | **done 2026-11-13** — release LIVE: 9 assets verified (5 sandbox + 4 CI, all hashed), both CI runs green on the release commit; https://github.com/maniesh-NR/KetraCopy/releases/tag/v1.2.1 | — |
| S10 | **TeraCopy reference study** — owner-directed (2026-11-13) study of the reference product's installers (3.17 + 4.0.5) for local-transfer / verification techniques that can inform a future optimization pass (performance, transfer stability, resource use) | **in progress — plan 1 of ≤2 done 2026-11-13** (Gate 3 PASSED → CLEANROOM disclosure was S10's first commit; Track B: packaging mapped for both versions, full PE inventory for 3.17 via the vendor MSI channel, 4.0.5 PE facts pending owner-rig export per the charter's degradation path; research docs + reproducible tooling committed to `docs/research/` + `scripts/research/`; vendor hashes denylisted in the backup guard; Track-A owner kit delivered in the study doc §4) | plan 2 = findings gate: per-candidate verdicts C1–C8 + tree contamination check; that gate schedules or buries S11 |

## Scheduled: S10 — TeraCopy reference study (charter)

**Owner directive (2026-11-13, verbatim)**: "Add to roadmap: decompile and
reverse engineer the install files for teracopy 3.17
(https://codesector.com/files/teracopy.exe) and teracopy 4.0.5
(https://codesector.com/files/teracopy4.0.5.exe), to study if there are
any methods for local file transfer and verification that can be
incorporated in to this project to improve performance, stability, and
resource use." Full decision record: `../decisions/ADR-0003`.

**Goal**: extract functional knowledge of the reference product's local
transfer and verification machinery — I/O strategy and request sizes,
buffering, async pipeline, verification timing (inline streaming hash vs
second pass), resume/journal layout, throttle adaptation, memory profile —
and map it to concrete optimization candidates for a **future** stage
(S11, deliberately NOT scheduled; exists only if S10 yields adoptable
findings).

**Method — two tracks**:

- **Track A — dynamic black-box behavioral analysis (primary)**. Install
  each version on the owner's Windows machine (owner-controlled hardware
  ONLY — Gate 3 removed the CI-runner option: no vendor downloads,
  installs or instrumentation in any `.github/workflows` change, no vendor
  material in CI logs/artifacts) and observe via
  OS instrumentation only (Process Monitor / ETW / xperf / API traces):
  I/O request sizes and queue depth, sync vs async I/O, `CopyFileEx`-style
  OS calls vs manual read/write loops, buffer sizes, verification timing,
  resume behavior after interruption, adaptive throttle behavior, working
  set under 1×10 GB and 10 k×1 MB loads, sparse-file handling; compare
  3.17 vs 4.0.5 (the 4.x rewrite is the interesting delta). **Every
  scenario co-instruments KetraCopy v1.2.1 on the same rig** — findings
  carry side-by-side columns and candidates must cite measured deltas,
  not absolute observations (Gate 3: without our own baseline the study
  is unadjudicable).
- **Track B — static inventory (secondary; feasibility pre-flight
  required)**. Unpack the two installers and record **functional
  interface facts only**: payload file inventory (names/sizes/versions),
  PE machine/subsystem/section summary, DLL import lists (API names),
  version resources — no verbatim vendor copyright/trademark strings, no
  UI screenshots or captured imagery, no disassembly, no decompiler
  output, no vendor resources in the repo. Feasibility (Auditor-verified
  2026-11-13): the sandbox has objdump-with-PE + strings but NO
  innoextract/7z/7za/wine and no sudo, so S10's first step is a recorded
  pre-flight — rootless unpacking-tool acquisition (`apt-get download` +
  `dpkg -x`, the documented xkblibs pattern) or explicit degradation to
  objdump PE-header/import facts on binaries exported from the owner's
  Windows rig.

**Clean-room guardrails (binding)**:

1. Vendor installers/binaries live OUTSIDE the repo
   (`/home/z/my-project/research/` in the sandbox), never committed.
2. The repo records functional observations only — behavior notes, API
   names, sizes, timings. No decompiled code, disassembly listings,
   extracted vendor assets, or vendor binaries.
3. Any resulting KetraCopy implementation is written from scratch against
   the functional notes (no translation/adaptation of vendor code).
4. `docs/CLEANROOM.md` is S10's **first deliverable and first commit** —
   before any download, unpacking or trace capture. It time-scopes the
   current absolute claims to ≤ v1.2.1, discloses the two-track
   methodology (incl. Track B installer unpacking / PE / import analysis),
   and states the boundary for any deeper private analysis (per guardrail
   6). Verified by commit order at the findings gate.
5. Trademark hygiene preserved (no affiliation; no vendor text).
6. **Provenance labels**: every findings entry is tagged [A] (trace-
   derived, citing the observing instrumentation) or [B]
   (inventory-derived); facts not attributable to A or B are excluded
   from the repo AND from informing any KetraCopy implementation. If the
   owner performs deeper private analysis, the CLEANROOM amendment
   discloses it and its output is barred from implementation.
7. **Mechanical backstop** (implemented 2026-11-13 at Gate 3): raw trace
   formats and research dirs are gitignored (layer 1); the backup
   pre-flight aborts on staged files with executable/OLE magic bytes or
   denylisted extensions (layer 2, tested both ways); the two installer
   SHA-256 hashes join the denylist at first acquisition. Pure-discipline
   control is not enough — this repo's own history proves it.

**Deliverables**: `docs/research/teracopy-reference-study.md` (methodology,
acquisition hashes, Track-A instrumentation checklist),
`docs/research/teracopy-findings.md` (side-by-side observations: TeraCopy
vs co-instrumented KetraCopy v1.2.1; findings → candidate → measured
delta → expected effect → risk), the CLEANROOM.md amendment, and a fresh
gate record on the findings. Every `docs/research/*` file opens with a
scope-and-disclaimer header (what this is / is not: no circumvention, no
vendor assets).

**Entry**: Gate 3 PASSED 2026-11-13 (zero BLOCK; record:
`../reviews/2026-11-13-gate-3-s10-charter.md`) — the conditions table
there is binding; items marked "at S10 start" must be resolved before
the first research commit. **Exit**: the findings gate issues explicit
per-candidate verdicts — adopt (→ an S11 row is added to the stage table
with expected effect + benchmark + effort), defer (→ candidates list), or
reject (→ with reason) — and verifies by tree inspection that no vendor
binaries, assets, raw traces or decompiler output exist in any S10
commit. **Time-box**: S10 completes within ≤ 2 execution plans.

## Post-1.2.1 roadmap (candidates, not yet scheduled)

- HTTPS LAN receiver (self-signed certs, strict LocalSend v2 compat) —
  rustls design preserved in `attic/ketra-net`; engine stays std-only via a
  separate `ketra-tls` crate (needs its own adversarial gate).
- Android APK distribution — CI (`.github/workflows/build-apk.yml`) already
  builds it on every push; wire the APK into release assets.
- Flatpak publish (`packaging/flatpak/`).
- Live interop testing against the real LocalSend application.
- GUI/CLI i18n.

## Stage/gate history

- 2026-11-13 — S4 done: harness bootstrapped, clean history (ADR-0002),
  first backup pushed to GitHub. Security: the pre-clean remote history
  (which contained the owner's PAT file) was force-pushed away; remote
  `v1.1.0` tag + release deleted; **PAT rotation advised** (MEMORY.md
  "Security incident").
- 2026-11-13 — S5 done (evidence pack); S6 Gate 1 round 1 FAILED (Red Team
  BLOCK: LAN filename-collision data loss); S7 fixed all findings + a new
  latent bug (`--conflict overwrite` EEXIST); S6/S8 Gate round 2 + Gate 2
  PASSED (zero BLOCK, conditions applied); S9 released v1.2.1 (9 assets,
  all hashed; both CI runs green).
- **2026-11-13 — KetraCopy v1.2.1 is LIVE.** All owner directives through
  this date are complete; the workflow (backups + adversarial gates) is
  standing infrastructure.
- 2026-11-13 — snapshot automation deleted the project `.git` a **2nd**
  time (after backup #6); recovered per MEMORY.md (`b3ef0ed`), including
  the never-pushed ROADMAP S9→done edit. Owner directive received: add the
  TeraCopy reference study to the roadmap → drafted as **S10** (ADR-0003,
  plan 02) and submitted to **Gate 3** for adversarial consensus before
  any execution. **Gate 3 PASSED** (zero BLOCK) → charter hardened.
- 2026-11-13 — **S10 plan 1 executed** (plan 03): CLEANROOM.md disclosure
  amendment landed as S10's first commit (`c138d82`); both target versions
  acquired from the vendor (3.17 via its CDN artifact, 4.0.5 confirmed =
  current release) and verified by embedded manifests; the Advanced
  Installer container mapped head-only XOR obfuscation + protected content
  streams (NOT circumvented — 3.17 binaries obtained via the vendor's own
  MSI channel, 4.0.5 deferred to the owner-rig export); Track B inventories
  + 3.17 PE facts + candidate map C1–C8 in `docs/research/`; six research
  tools in `scripts/research/`; vendor hashes denylisted in backup.sh.
  Awaiting: owner-rig Track-A data (kit in the study doc §4) → S10 plan 2
  (findings gate) schedules or buries S11.
