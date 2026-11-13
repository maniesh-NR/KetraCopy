# KetraCopy Harness — Workflow Rules

These rules govern all agent-driven work on KetraCopy. They were mandated
by the project owner on 2026-11-13 (see `decisions/ADR-0001`) and take
precedence over convenience.

## 1. Backup policy (event-driven, never time-driven)

- **Every execution plan ends with a backup as its FINAL step**, and every
  prompt output ends with a backup: run
  `harness/scripts/backup.sh "<description of what this backup captures>"`.
- Backups are **event-driven** — never on a fixed time interval.
- A backup commits the full working tree (source, docs, harness state,
  evidence, review records) to `main` and pushes to the origin remote on
  GitHub.
- **Secrets are never committed.** The GitHub PAT lives outside the repo
  (`/home/z/my-project/.secrets/` in the sandbox); if it is lost the owner
  re-provides it (see `state/MEMORY.md`).
- If a push fails (network/credentials), the commit still stands locally;
  `backup.sh` records `push: failed` in the log and the next successful
  backup uploads the accumulated commits.

## 2. Adversarial stage gates

- **Every stage of work must pass a comprehensive assessment by independent
  adversarial agents BEFORE the next stage begins.**
- Minimum panel of three independent roles:
  1. **Red Team** — code/correctness adversary (hunts real defects),
  2. **Auditor** — independent re-verification of claims and evidence,
  3. **Strategist** — roadmap/release-readiness adversary.
- Reviewers are stateless and do not see each other's verdicts
  (independence). Each returns: findings (Critical / Major / Minor, each
  with evidence), a verdict — `PROCEED` | `PROCEED-WITH-CONDITIONS` |
  `BLOCK` — and a confidence level.
- **Consensus rule: a gate passes only with NO BLOCK verdict.** Conditions
  attached to `PROCEED-WITH-CONDITIONS` must be resolved before or during
  the next stage, and their resolution recorded in the gate record.
- If any reviewer BLOCKs: fix the issues, then run a **fresh independent
  review round** on the fixes. Repeat until consensus.
- Environmental limits that are already documented and planned (e.g. no
  macOS/Android runtime in the sandbox) are Minor notes, not blockers —
  gate findings must be actionable.
- Gate records live in `reviews/` (one file per gate, written only after
  all verdicts are in).

## 3. Memory & state

- `state/MEMORY.md` is the persistent memory — read it FIRST at the start
  of every session; update it when state changes.
- `state/ROADMAP.md` tracks stages and gate status and is the single
  source of truth for progress.
- One plan file per user prompt in `plans/` (the plan itself must list the
  backup as its final step); decisions in `decisions/` (ADRs); backup log
  in `backups/LOG.md`; supporting logs in `evidence/`.

## 4. Stage sequence

See `state/ROADMAP.md`. No stage starts without a passed gate for the
previous stage.
