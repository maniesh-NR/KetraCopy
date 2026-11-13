# Gate 1 — work-so-far + roadmap (round 1)

- **Date / time (UTC)**: 2026-11-13 ~11:00
- **Stage under review**: all work so far (S0–S5: KetraCopy 1.0.0 → 1.2.0 + harness
  bootstrap + S5 evidence pack) and the future roadmap (S7–S9 → release, post-1.2.0).
- **Trigger**: owner-mandated adversarial consensus gate (`WORKFLOW.md` §2) —
  first gate of the workflow era; no prior stage was gated (pre-workflow).
- **Inputs given to reviewers**: repo at HEAD `545d393` (clean baseline + S5
  backups), `harness/evidence/2026-11-13-S5/` (README + 13 logs), CHANGELOG.md,
  README.md, docs/, `harness/state/*`, `.github/workflows/*`, packaging.
  Reviewers ran stateless and independently; Red Team and Strategist were
  read-only, the Auditor re-ran the verification suites.

## Verdicts (recorded independently)

### Reviewer 1 — Red Team (adversarial code review)
- Verdict: **BLOCK** — Confidence: high
- Findings:
  - **[CRITICAL] LAN receiver filename collision → silent loss of the first
    received file.** `lan/server.rs:324-325` assigns paths via `unique_path`
    (disk-existence check only, `server.rs:674-690`); nothing tracks paths
    already assigned to other files in the same session, and v2 sessions
    preallocate nothing (`server.rs:330-339`). Two colliding names in one
    prepare (duplicate `invoice.pdf`, or sanitize collision `a:b.txt` +
    `a_b.txt`) → second upload truncates the first's verified file, both
    return `{"ok":true}`. Also a seconds-wide race between two concurrent
    sessions choosing the same not-yet-existing name.
  - **[MAJOR] Source file shrinking mid-copy → zero-padded destination
    journaled `done` and reported as success** (`pipeline.rs:379-380` sets
    `hit_eof` with no error; `pipeline.rs:490` `set_len(size)` zero-pads;
    `pipeline.rs:524-531` writes `done`; resume then skips it,
    `engine.rs:642-649`).
  - **[MAJOR] `--no-times` degenerates the Phase-1 fast scan to size-only**:
    `engine.rs:623-624` makes `same_mtime` unconditionally true when
    mtime preservation is off, so a same-size modified destination file is
    silently skipped as up-to-date (wiring `args.rs:185` → `run_cmd.rs:71`).
  - **[MAJOR] HTTP client pre-allocates the full response body from an
    untrusted `Content-Length`** (`lan/http.rs:466-471`) → one hostile
    header aborts/OOMs the sender; mDNS makes hostile peers trivially
    presentable. (Server side is capped; client side is not.)
  - **[MAJOR] Crash-consistency gap**: batched `partial` journal records can
    be durable while the data below the bound is not (no fsync before the
    journal append; `write_exact_at` is a bare pwrite loop,
    `pipeline.rs:629-648`), and `probe_resume` verifies only the final tail
    chunk (`resume.rs:117-135`) — after power loss a durable bound above
    missing data resumes from the bound and marks `done` → silent
    corruption. Existing test covers only the all-zeros case.
  - [MINOR] `manifest_check` orphans preallocated files (missing-digest
    branch never removes, `server.rs:614-617`).
  - [MINOR] plan→pipeline TOCTOU: file swapped to a symlink between stat
    and open is followed (`pipeline.rs:215-218`, no `is_file()` re-check).
  - [MINOR] generated `ketra-paste-smart` accepts `..` in window titles
    (`shell.rs:517`) — paste can be redirected outside `$HOME`.
  - [NOTE] `SECURE_NAME` doc/impl mismatch (Windows reserved *names* not
    handled, only characters — `lan/mod.rs:43-52` vs `53-68`).
  - [NOTE] LAN token is deterministic/unauthenticated; `cancel` needs no
    token; dead `if !prep.info.download && !ketra {}` (`server.rs:296-298`) —
    consistent with the documented trusted-LAN model.
  - [NOTE] Verified-safe: mDNS wire parsing bounds-safe; Windows FFI
    `#[repr(C)]` + `size_of`-checked; parallel engine journal sharing
    mutex-guarded; inline-verify re-read logic correct incl. partial-CFR
    top-up.
  - [NOTE] Test gaps: LAN duplicate/colliding names + concurrent sessions;
    source shrink mid-copy; `--no-times` same-size modified; hostile
    `Content-Length`; power-loss (good tail / bad below); Windows reserved
    names.

### Reviewer 2 — Auditor (independent re-verification)
- Verdict: **PROCEED** — Confidence: high
- Re-verified personally: 129/0 unit tests (20 CLI + 85 core + 24 MD3),
  39/39 integration, 103/106 interop (only the 3 S7-scoped cross-binary
  fails), GUI smoke green, bench in range, `sha256sum -c` all 5 OK and
  matching `artifact-hashes.md`, rustc 1.100.0, git clean/in-sync/single
  main/no tags. 12 CHANGELOG-1.2.0 claims verified against source with line
  quotes (`--parallel` 1..=16, `--retries` default 2 + backoff, streaming
  hashes, 32 MiB journal stride + first-chunk anchor, TCP_NODELAY both ends,
  posix_fadvise readahead, fdatasync, multi-stream buffer cap, parallel
  workers + shared journal, inline manifest digest reuse, per-crate test
  counts).
- Findings: [MINOR] README macOS claim overbroad vs the objc2 finding
  (`README.md:34,171`); [MINOR] CHANGELOG says "127 unit tests", actual 129;
  [MINOR] placeholder URLs also in `packaging/windows/ketracopy.iss:27` and
  `packaging/linux/io.github.ketracopy.KetraCopy.metainfo.xml:22` (beyond
  the deb Homepage); [NOTE] `dist/README.txt` absent on fresh clones (soft
  skip); [NOTE] README links no project repository; [NOTE] warnings as
  disclosed.
- Conditions: scope README macOS wording; fix CHANGELOG count; sweep all
  placeholder URLs; make dist/README.txt reproducible; carry S7 items
  (zigbuild cross binaries, objc2 resolution).

### Reviewer 3 — Strategist (roadmap + release readiness)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- Findings:
  - **[MAJOR] release.yml macOS jobs will fail on the stable toolchain**
    (objc2 E0277; chain eframe 0.28.1 → glutin 0.31.3 → icrate 0.0.4 →
    objc2 0.4.1 is terminally pinned — the fix landed in the 0.5 line).
    Recommendation: macOS **CLI-only** for 1.2.0 (matches what the sandbox
    dist ships; macOS GUI has never been runtime-tested anyway); verify
    with a workflow_dispatch CI run before the tag; `macos-13` runner
    availability is a separate live risk (prefer macos-latest + cross
    target). Close option (b) with `cargo update -p objc2 --dry-run`
    evidence.
  - **[MAJOR] "LocalSend compatible" claims not release-honest**: README
    says "accepts uploads … from official LocalSend clients" and "(as
    LocalSend does)" for plain HTTP — official clients are HTTPS-only;
    `docs/ARCHITECTURE.md:184,219,227` claims an HTTPS receiver (that is
    the attic prototype); `docs/LAN-PROTOCOL.md:4` points at the stale
    `crates/ketra-net` path and its TLS sections contradict the honest
    implementation-status section. Fix = wording/doc restructure, not code.
  - **[MAJOR] Android build broken at HEAD**: `android/app/build.gradle.kts:117`
    builds `-p ketra-ffi` with inputs `crates/ketra-ffi/src`,
    `crates/ketra-net/src` — deleted; Kotlin imports the UniFFI facade
    while `jni.rs` documents a different contract; `docs/ANDROID.md`
    describes the deleted architecture. build-apk.yml runs red on every
    push. Descope Android from 1.2.0 release claims; restrict the workflow
    to `workflow_dispatch` until repaired (repair is post-1.2.0 roadmap).
  - **[MAJOR] Release asset provenance ambiguous**: sandbox dist and
    release.yml produce same-named artifacts; `softprops/action-gh-release@v2`
    overwrites by default → CI builds would silently replace hash-baselined
    sandbox artifacts. Give each asset exactly one producer, then regenerate
    the hash baseline to match the release page exactly.
  - **[MAJOR] PAT rotation advised but unconfirmed** — must be confirmed by
    the owner and recorded in MEMORY.md before the v1.2.0 tag.
  - [MINOR] deb placeholders ×3 (Homepage, Maintainer, Source);
    [MINOR] Cargo.toml repository URL lowercase vs canonical; [MINOR]
    `docs/BUILD.md` stale (rfd, test counts, binary name); [MINOR]
    `dist/README.txt` not reproducible; [MINOR] CHANGELOG 127→129;
    [MINOR] macos-13 runner risk; [NOTE] ATTRIBUTIONS should credit the
    harness/adversarial-gate workflow; [NOTE] backup.sh `git add -A` could
    auto-commit a stray credential (consider a guard); [NOTE] post-1.2.0
    roadmap placement otherwise correct — do not pull rustls TLS or the
    Android rebuild into 1.2.0 (scope creep through the gate).

## Consensus determination

- BLOCK count: **1** (Red Team). Rule: gate passes only with zero BLOCK.
- **Outcome: FAILED — round 1.** Fixes required, then a fresh independent
  review round on the fixes (round 2) before S7 may proceed to packaging /
  S8 / S9.

## Fix queue for round 2 (owner-approved by the gate itself)

| # | Finding | Severity | Fix owner stage |
|---|---|---|---|
| F1 | LAN receiver path collision (in-session registry + reserve-at-prepare incl. v2 and size==0) + 3 regression tests | CRITICAL | S7-a |
| F2 | Source shrink mid-copy must fail loudly (no zero-pad, no `done`) + test | MAJOR | S7-a |
| F3 | `--no-times` fast-scan skip must not reduce to size-only + test | MAJOR | S7-a |
| F4 | Client-side `Content-Length` sanity cap + hostile-header test | MAJOR | S7-a |
| F5 | Journal durability: fsync data before batched `partial` records (or verify-all-below at resume) + invariant test + docs/ALGORITHMS.md | MAJOR | S7-a |
| F6 | manifest orphan cleanup; paste-smart `..` containment; SECURE_NAME reserved names + doc fix; dead code | MINOR | S7-a |
| F7 | Docs honesty: README (macOS wording, LocalSend claims, feature matrix), LAN-PROTOCOL restructure + stale path, ARCHITECTURE HTTPS purge, BUILD.md refresh, CHANGELOG 127→129, Cargo.toml canonical URL, ATTRIBUTIONS harness line | MAJOR/MINOR | S7-b |
| F8 | release.yml: macOS CLI-only both darwin targets (macos-latest + cross x86_64), workflow_dispatch trigger, non-overlapping asset names (CI = macOS native only; sandbox = Linux + Windows + src) | MAJOR | S7-b |
| F9 | build-apk.yml: restrict to workflow_dispatch until Android is repaired (post-1.2.0); descope Android from release claims | MAJOR | S7-b |
| F10 | Packaging placeholders ×3 + metainfo + iss URLs; dist/README.txt tracked template; `cargo update -p objc2 --dry-run` evidence | MINOR | S7-b |
| F11 | PAT rotation: owner confirmation to be recorded in MEMORY.md **before** the v1.2.0 tag (S9 security gate) | MAJOR | S9 |

## Follow-ups handed to the next stage

- Round 2 of this gate reviews the fixes above (fresh independent agents).
- The Strategist's release-notes conditions (known-limitations block) go to S9.
- backup.sh secret-guard hardening is a NOTE for a future harness pass.
