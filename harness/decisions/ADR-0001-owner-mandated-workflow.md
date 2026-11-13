# ADR-0001: Owner-mandated harness workflow (backups + adversarial gates)

Date: 2026-11-13 · Status: ACCEPTED (owner mandate)

## Context

The project owner directed on 2026-11-13:

> "Add workflow: frequent sandbox backups to github, create harness-like
> memory and infrastructure on a section of the github repo. Backups
> should always be the last step in every execution plan, every out put
> to a prompt should end with a backup, rather than backups at a fixed
> time interval. assign adversarial roles to agents to examine work done
> so far and the future roadmap, only proceed to the next stage when a
> consensus has been reached. Every stage in future must have a
> comprehensive assessment by adversarial agents, and require independent
> consensus in order to proceed further."

## Decision

1. `harness/` is the mandated section: memory, roadmap, plans, reviews,
   decisions, evidence, backup log and `scripts/backup.sh`.
2. Backups are **event-driven**: `harness/scripts/backup.sh` runs as the
   FINAL step of every execution plan (commit + push to GitHub main). No
   cron, no fixed interval.
3. Every stage passes an **adversarial gate** before the next starts: a
   panel of at least three independent agents (at minimum Red Team /
   Auditor / Strategist), each returning findings plus a verdict of
   `PROCEED`, `PROCEED-WITH-CONDITIONS` or `BLOCK`.
4. **Consensus = zero BLOCK verdicts.** Conditions attached to
   conditional verdicts must be resolved before/during the next stage and
   recorded in the gate record. A BLOCK sends the stage back for fixes
   and a fresh independent review round.
5. Independence: reviewers run stateless, do not see each other's
   verdicts, and the gate record is written only after all verdicts are
   collected.

## Consequences

- Stage transitions cannot silently skip review.
- Every prompt leaves a backup trail in `harness/backups/LOG.md` and on
  GitHub.
- Environmental limits that are documented and planned (e.g. no macOS /
  Android runtime in the sandbox) are Minor notes rather than blockers —
  gate findings must remain actionable.
