# Gate 3 — S10 charter (TeraCopy reference study)

- **Date / time (UTC)**: 2026-11-13 ~14:20
- **Stage under review**: the S10 roadmap amendment — owner directive of
  2026-11-13 ("Add to roadmap: decompile and reverse engineer the install
  files for teracopy 3.17 … and teracopy 4.0.5 …"), drafted as the
  "TeraCopy reference study" charter (`state/ROADMAP.md` §Scheduled: S10) +
  `decisions/ADR-0003` + `plans/2026-11-13-plan-02`. **No S10 execution
  had occurred** (plan-02 scope guard: amendment only; Auditor verified:
  no `research/` dir, no tracked vendor files, no "codesector" refs outside
  the quoted directive).
- **Trigger**: WORKFLOW §2 — no stage may execute before a passed
  adversarial gate; the owner's original workflow mandate also requires
  adversarial examination of roadmap changes.
- **Inputs**: repo HEAD `26ce29d` (recovery backup `b3ef0ed` + LOG append;
  local == origin/main, verified by Auditor), the three amendment files,
  `docs/CLEANROOM.md`, `WORKFLOW.md`, `MEMORY.md`, `backup.sh`,
  `.gitignore`, live sandbox tool probe. Reviewers ran stateless, in
  parallel, without visibility into each other (independence rule).

## Verdicts (recorded independently, before any cross-reading)

### Reviewer 1 — Red Team (compliance & contamination)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- Findings:
  - [MAJOR] Mechanical auto-publish leak path: guardrail 1 was
    discipline-only — `backup.sh` `git add -A` + push at the end of every
    plan auto-publishes anything dropped in the workspace to the PUBLIC
    repo; no research/trace rules existed in `.gitignore`. This repo's own
    history (PAT leak; `.git` deleted twice) proves discipline-only fails.
  - [MAJOR] ADR-0003 §4 (private analysis "may go deeper") opened an
    unlogged laundering path — decompiler-derived knowledge could be
    disguised as "functional observations", making the amended CLEANROOM
    a half-truth and guardrail 3 unenforceable.
  - [MAJOR] "EARLY in S10" was not checkable; CLEANROOM.md goes factually
    false the moment Track B unpacks anything — the honesty window had to
    be pinned by commit order.
  - [MAJOR] Track A venue left open to CI: public workflow logs/artifacts
    are publication channels; hosted runners are not owner-controlled.
  - [MINOR] Guardrail wording gaps (raw trace exports, UI screenshots,
    version-resource vendor strings vs "no vendor text"); [MINOR] RE-for-
    performance sits outside the cleanest legal carve-outs, jurisdiction-
    variable, vendor named with URLs in a public repo (accepted residual
    risk per ADR); [MINOR] exit criteria lacked a contamination check.

### Reviewer 2 — Auditor (independent re-verification)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- Verified: repo root = workspace; origin = maniesh-NR/KetraCopy; local
  main == origin/main == `26ce29d` (LOG-append commit; parent `b3ef0ed`);
  owner directive quoted verbatim (incl. "in to") in ADR-0003, plan-02,
  ROADMAP charter; S10 row + checkable entry/exit criteria; no duplicate
  study mention in the candidates list; plan-02 house rules (backup as
  final step, checkbox reality); scope guard honored on disk; CLEANROOM
  quote accurate; ADR-0003 ↔ charter consistent; Windows CI claim TRUE
  (release.yml `build-windows`, windows-latest); Linux-only sandbox.
- Findings:
  - [MAJOR] Track B "**sandbox-executable**" was an overstatement: first
    required verb is "unpack", but innoextract/7z/7za/wine are ABSENT and
    there is no sudo — unpacking infeasible as-is; tooling gap acknowledged
    nowhere. (objdump present WITH PE support — pei-x86-64/pei-i386 — and
    strings present.)
  - [MINOR] Uncommitted MEMORY.md edit not enumerated in plan-02 steps.
  - [MINOR] "The repo already has Windows CI" accurate but tag-triggered
    release-only; an instrumentation venue would be new work.

### Reviewer 3 — Strategist (roadmap & value)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: medium (governance/
  feasibility findings firm; adoptable value genuinely unknowable until the
  4.x delta is observed — conditions bound the downside)
- Findings:
  - [MAJOR] Track A venue open to CI (same as RT); owner's machine gives
    clean separation between off-repo evidence and on-repo conclusions.
  - [MAJOR] No comparative baseline: all 1.2.0 benchmark evidence is
    Linux-sandbox-only; observing TeraCopy's Windows I/O without
    co-instrumenting KetraCopy v1.2.1 on the same rig produces unadjudicable
    findings. Biggest value gap; fixing it makes the study classic
    comparative performance research.
  - [MAJOR] Track B sandbox-executability optimistic (tooling probe; both
    installers are Inno Setup containers) — needs a recorded pre-flight.
  - [MAJOR] Engine already captured the standard playbook (O_DIRECT/aligned
    no-memset buffers, fadvise, inline single-read verify, 32 MiB journal
    batching, --parallel, sparse/extent, AIMD+token bucket) — yield is
    narrow; the 3.17→4.0.5 rewrite delta is the only information not
    obtainable from any public source.
  - [MINOR] Exit criterion said "accepted" without the findings gate
    explicitly scheduling or burying S11; [MINOR] CLEANROOM amendment must
    be hardened to "first deliverable, same plan as any research doc";
    [MINOR] stage-fit acceptable but time-boxed so it does not silently
    consume the post-1.2.1 window (HTTPS LAN receiver remains the highest
    user-value candidate).
- Highest-yield study targets (Strategist): Windows I/O strategy
  (overlapped/IOCP vs sync, CopyFileEx/CopyFile2 vs manual loops,
  FILE_FLAG_NO_BUFFERING, request sizes/queue depth — adoptable via
  `platform/windows.rs` FFI, std-only core intact); per-media chunk-size
  adaptation (ours fixed 1 MiB); many-small-files scheduling (walk-ahead,
  per-file fsync policy, parallelism auto-tuning); the 3.17→4.0.5 delta
  itself. Verification timing likely confirms OUR inline single-read verify
  is superior — valuable as documented justification, not as an
  optimization. Theater risks: wholesale import-list transcription, payload
  inventory tables — keep time-boxed to the functional-fact list.

## Consensus determination

- BLOCK count: **0** (rule: gate passes only with zero BLOCK).
- Outcome: **PASSED (round 1)** — three × PROCEED-WITH-CONDITIONS with
  concrete, actionable, checkable conditions. S10 is authorized to execute
  **under the consolidated conditions below**.

## Conditions & resolution tracking

Amendment-scope conditions were applied at gate time (2026-11-13); stage-
scope conditions are S10 entry criteria, binding before/at the marked
point. Charter cross-reference: `state/ROADMAP.md` §S10 guardrails 1–7 and
Entry/Exit.

| # | From | Condition | Applies | Status |
|---|---|---|---|---|
| RT-1 | Red Team | CLEANROOM.md amendment is S10's **first commit** — before any download/unpacking/trace capture; time-scopes absolute claims to ≤ v1.2.1, discloses Track B unpacking/PE/import analysis, states the private-analysis boundary | at S10 start | charter hardened (guardrail 4); verification = commit order at findings gate |
| RT-2 | Red Team + Strategist | Venue pinned: Track A on owner-controlled Windows hardware ONLY; no vendor downloads/installs/instrumentation in any `.github/workflows` change; no vendor material in CI logs/artifacts | now | **applied** — ROADMAP charter + ADR-0003 rewritten |
| RT-3 | Red Team | Provenance labels: every findings entry tagged [A] (trace-derived, instrumentation cited) or [B] (inventory-derived); non-attributable facts excluded from repo AND from informing implementation; private deeper analysis disclosed + barred from implementation | now + S10 | **applied to charter** (guardrail 6); CLEANROOM wording lands with RT-1 |
| RT-4 | Red Team | Mechanical backstop: gitignore trace/research rules + backup.sh pre-flight abort on PE/MSI signatures; installer SHA-256 hashes join denylist at acquisition | now + at acquisition | **implemented + tested** (layer 1 `.gitignore`, layer 2 `backup.sh` guard — pass-through verified on amendment files, FATAL verified on fake PE magic); hash denylist pending acquisition |
| RT-5 | Red Team | Track B = structural facts only; no verbatim vendor copyright/trademark strings from version resources; no UI screenshots or captured imagery | now | **applied** — Track B bullet rewritten |
| RT-6 | Red Team | Exit adds contamination check: findings gate verifies via `git ls-files`/tree inspection that no vendor binaries, assets, raw traces or decompiler output exist in any S10 commit | S10 exit | **applied** — Exit criteria |
| AU-1 | Auditor | Fix ROADMAP Track B "sandbox-executable" overstatement (or prove feasibility) | now | **applied** — relabeled "feasibility pre-flight required" + Auditor-verified tool inventory baked into the charter |
| AU-2 | Auditor | Gate record durably documents the verified tool inventory | now | **done** (see below) |
| AU-3 | Auditor | Final backup description must mention the MEMORY.md edit alongside the amendment files | now | done at final backup (see backups/LOG.md row for this plan) |
| ST-1 | Strategist | = RT-2 (owner rig only) | now | **applied** (see RT-2) |
| ST-2 | Strategist | Every Track-A scenario co-instruments KetraCopy v1.2.1 on the same rig; findings carry side-by-side columns; candidates cite measured deltas | now + S10 | **applied** — Track A + Deliverables |
| ST-3 | Strategist | = RT-1 + scope-and-disclaimer header on every `docs/research/*` file | S10 start | charter Deliverables updated (header requirement) |
| ST-4 | Strategist | Recorded Track-B pre-flight as S10's first step (tool acquisition or degradation to objdump PE facts on binaries exported from the Windows rig) | at S10 start | **applied** — Track B bullet |
| ST-5 | Strategist | Findings gate issues per-candidate verdicts: adopt (→ S11 row + expected effect + benchmark + effort) / defer / reject-with-reason; recorded in gate record + ROADMAP | S10 exit | **applied** — Exit criteria |
| ST-6 | Strategist | Time-box: S10 ≤ 2 execution plans; deeper-than-public-record analysis stays off-repo and out of deliverables | S10 | **applied** — Time-box clause |

**Auditor tool inventory (durable record, 2026-11-13)**: sandbox is
Linux-only; NO innoextract, 7z, 7za, wine; NO sudo; objdump present WITH
PE support (pei-x86-64/pei-i386 targets); strings present; rustup needs
PATH export after recycles; no Android SDK. Environmental limits are
documented-and-planned (WORKFLOW §2: Minor notes, not blockers).

## Follow-ups handed to S10 (entry checklist before first research commit)

1. Am CLEANROOM.md per RT-1/ST-3 — S10's FIRST commit, before any
   acquisition (content requirements: time-scope ≤ v1.2.1; two-track
   disclosure; private-analysis boundary + bar; scope headers for
   `docs/research/*`).
2. Record the two installer SHA-256 hashes in the `backup.sh` denylist at
   first acquisition (RT-4 completion).
3. Run + record the Track B pre-flight (rootless innoextract/7z acquisition
   via `apt-get download` + `dpkg -x`, or explicit degradation path).
4. Track A scenarios run on the owner's Windows rig with side-by-side
   KetraCopy v1.2.1 instrumentation; findings tagged [A]/[B].
5. Exit: per-candidate verdicts at a fresh gate + tree contamination check;
   ≤ 2 execution plans total.
