# Gate 1 (round 2) + Gate 2 — fixes verification + release readiness

- **Date / time (UTC)**: 2026-11-13 ~12:10
- **Stage under review**: the S7 fix round (all Gate-1 findings + the
  follow-up bug) and the S9 release plan (tag `v1.2.1`, CI-native
  Windows/macOS builds, single-producer release assets, release notes).
- **Trigger**: Gate 1 round 1 FAILED (Red Team BLOCK) → fixes applied →
  fresh panel required per WORKFLOW.md §2; combined with Gate 2 (one panel,
  both mandates — recorded here for both gates).
- **Inputs**: repo HEAD (S7 commits), round-1 record, CHANGELOG 1.2.1,
  `harness/evidence/2026-11-13-S7/`, release.yml, dist/ + hashes.

## Verdicts (recorded independently)

### Reviewer 1 — Fix Verifier (Auditor role)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- Re-ran everything personally: 133 unit (at that HEAD) / 41 integration /
  105 interop, all regression tests real and would fail on old code;
  F1-F6 + follow-up all verified fixed at code level.
- Findings: [MINOR] F1 "expiry cleanup" claim was not implemented
  (purge_expired retained only); [MINOR] BUILD.md count off by one;
  [MINOR] no test for the probe below-bound branch + ALGORITHMS.md resume
  section stale; [NOTE] round-1 symlink TOCTOU MINOR silently out of
  scope; [NOTE] prepare-abort orphan edge (subsumed by the F1 gap).

### Reviewer 2 — Red Team (fix regression review)
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- No new CRITICAL; the reservation design itself verified sound
  (concurrent sessions, upload interplay, EEXIST fall-through, empty-file
  edges, sync ordering, 8 MiB cap sizing, reserved-name semantics).
- Findings: [MAJOR] the promised abandoned-reservation cleanup did not
  exist (expiry/cancel/prepare-abort all leak zero-progress reservations;
  prealloc'd ketra sends leave declared-size sparse files) — either
  implement or strip the claims; [MINOR] cancel-path journal record
  ignores a failed sync; [MINOR] manifest_check deletes on digest
  omission, not on "never uploaded"; [MINOR] ALGORITHMS.md F5 sub-item
  not delivered.

### Reviewer 3 — Release Readiness Strategist
- Verdict: **PROCEED-WITH-CONDITIONS** — Confidence: high
- Findings: [MAJOR] release-creation order — the tag push auto-creates
  the release (softprops) → PATCH, not POST, and never pre-create via API
  (API tag creation fires no tag-push event); [MAJOR] artifact-hashes.md
  still 1.2.0-era; [MAJOR] package-dist.sh still able to mint colliding
  windows/macOS asset names (single-producer not enforced in code);
  [MAJOR] five docs-consistency spots (README counts, FEATURE-PARITY
  stale rows, BUILD.md macos.rs caveat, .iss header comments);
  [MAJOR] PAT rotation unconfirmed — not a blocker with compensating
  controls (all-asset SHA-256 table in the release notes + MEMORY
  deferral record); [MINOR] `--locked` missing on CI builds; python3/7z
  on runners to be proven by the mandatory green dispatch run.
- Verified sound: YAML, asset names, globs, tag-gating of attach steps,
  single-producer coverage (9 assets = 5 sandbox + 4 CI), skipping
  v1.2.0 tag (never publicly released), macOS-GUI deferral consistency.

## Consensus determination

- BLOCK count: **0**. Outcome: **PASSED (round 2 + Gate 2)**.
- Consensus rule satisfied: zero BLOCK, all three verdicts
  PROCEED-WITH-CONDITIONS with concrete, actionable conditions.

## Conditions & resolution tracking

| # | Condition (from verdict) | Resolution (all applied 2026-11-13, pre-tag commit) | Status |
|---|---|---|---|
| 1 | Implement the abandoned-reservation cleanup (Red Team MAJOR; Verifier F1) | Implemented for real: `uploading` in-flight guard on FileSlot; `cleanup_abandoned()` on session expiry (purge_expired) and cancel; `unwind_reservations()` on prepare-abort; manifest_check deletion now keyed on `written == 0 && !done_reported`, keeping received-but-unconfirmed bytes | done |
| 2 | Cancel-path journal must not record over a failed sync (Red Team MINOR) | Record only when `sync_dst(...)` succeeds (keeps the older verified bound) | done |
| 3 | Probe below-bound test + ALGORITHMS.md resume refresh (both reviewers) | `test_trusted_bound_with_bad_chunk_below_steps_back` added; ALGORITHMS.md rewritten (probe_resume + durability invariant) | done |
| 4 | BUILD.md count; README matrix counts; FEATURE-PARITY staleness; BUILD.md macos.rs caveat; .iss header (Strategist) | All fixed: BUILD.md 134; README 90-core/134 + scripts line; FEATURE-PARITY snapshot banner; BUILD.md macos dispatch truth; .iss 1.2.1 headers | done |
| 5 | artifact-hashes.md stale (Strategist MAJOR) | Rewritten: 1.2.1 sandbox hashes + CI section to be appended at release time | done |
| 6 | Single-producer enforced in code (Strategist MAJOR) | package-dist.sh windows/macOS packaging steps removed with an explanatory note | done |
| 7 | `--locked` on CI builds (Strategist MINOR) | Added to both release.yml build steps | done |
| 8 | PAT rotation deferral with compensating controls (Strategist) | MEMORY.md records the decision + owner action items; release notes to publish all-asset SHA-256 | done |
| 9 | Green workflow_dispatch run on the NEW HEAD before tagging (Strategist) | Re-dispatched on the post-fix HEAD; tag only after both jobs green | in progress |
| 10 | Symlink TOCTOU recorded, not lost (Verifier NOTE) | README Known limitations + post-1.2.x roadmap item | done |
| 11 | Release execution order: tag → push → wait green → PATCH notes + hash table → upload sandbox assets → verify 9 assets; do not re-run package-dist during S9 (Strategist) | Adopted as the S9 runbook | adopted |

## Follow-ups handed to the next stage (S9 release)

- Execute the Strategist runbook exactly (conditions 9 + 11).
- Post-release: append the 4 CI asset hashes to artifact-hashes.md.
- Owner action items (MEMORY.md): rotate the PAT; optionally request
  GitHub Support purge of dangling pre-clean commits.
