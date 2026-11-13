# KetraCopy Harness — persistent memory, infrastructure & governance

This section of the repository is the project's *harness*: the memory and
infrastructure layer for agent-driven development. It was mandated by the
project owner on 2026-11-13 and is governed by the rules in
[`WORKFLOW.md`](WORKFLOW.md) (see `decisions/ADR-0001`).

## Layout

| Path | Purpose |
|---|---|
| `WORKFLOW.md` | The governing rules: event-driven backups + adversarial consensus gates |
| `state/MEMORY.md` | Persistent memory — read FIRST at every session start |
| `state/ROADMAP.md` | Stage list + gate status; single source of truth for progress |
| `state/artifact-hashes.md` | sha256 baseline of the current `dist/` release artifacts |
| `plans/` | One execution plan per user prompt (each ends with a backup step) |
| `reviews/` | Adversarial gate records: panel, findings, verdicts, consensus |
| `decisions/` | Decision records (ADRs) |
| `evidence/` | Test/bench logs backing the claims of each stage |
| `backups/LOG.md` | Append-only backup log (one row per backup) |
| `scripts/backup.sh` | The backup routine — final step of every execution plan |

## The two invariants

1. **Every execution plan ends with `harness/scripts/backup.sh`** —
   commit + push to GitHub. Event-driven, never on a timer.
2. **No stage starts without a passed adversarial gate** — three or more
   independent adversarial agents, no BLOCK verdicts (see `WORKFLOW.md`).
