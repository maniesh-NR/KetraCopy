# Plan 02 — 2026-11-13: Roadmap amendment — schedule S10 (TeraCopy reference study)

**Trigger**: owner directive (2026-11-13, verbatim): "Add to roadmap:
decompile and reverse engineer the install files for teracopy 3.17
(https://codesector.com/files/teracopy.exe) and teracopy 4.0.5
(https://codesector.com/files/teracopy4.0.5.exe), to study if there are any
methods for local file transfer and verification that can be incorporated
in to this project to improve performance, stability, and resource use."

**State on entry**: v1.2.1 released and live (S4–S9 done; Gates 1 + 2
passed). The sandbox snapshot automation had deleted the project `.git`
again (2nd occurrence, after backup #6) — recovered per MEMORY.md before any
amendment work, including the never-pushed ROADMAP S9→done edit.

## Steps

1. [x] Session bootstrap per MEMORY.md: read state/plans/reviews; verify
   repo — found `.git` deleted → recovered (re-init, config, remote, fetch,
   reset onto origin/main, mode-flips reverted). Protective backup `b3ef0ed`
   ("recovery: …", push ok).
2. [x] Author the amendment: this plan, `decisions/ADR-0003`, and the
       `state/ROADMAP.md` S10 row + charter subsection (two-track method with
       clean-room guardrails; CLEANROOM.md disclosure amendment required inside
       S10; vendor binaries stay outside the repo). Also part of this plan
       (harness infra, not product code — scope guard's "no code changes"
       means no study/product execution): the MEMORY.md 2nd-`.git`-deletion
       note, and — ordered by Gate 3 condition RT-4 — the mechanical
       vendor-material backstop (.gitignore denylist rules + `backup.sh`
       pre-flight guard on staged files, syntax-checked and tested both ways:
       pass-through on the 6 amendment files, FATAL abort on fake PE magic).
3. [x] **Gate 3** — adversarial panel (Red Team / Auditor / Strategist;
       independent, stateless, no cross-reading) on the amended roadmap and
       the S10 charter. Outcome: **PASSED — zero BLOCK**; all three
       PROCEED-WITH-CONDITIONS (RT: high, AU: high, ST: medium).
4. [x] Record all verdicts in `reviews/2026-11-13-gate-3-s10-charter.md`;
       apply amendment-scope conditions immediately (done: venue pinned to
       owner hardware, Track B feasibility pre-flight, guardrails 6–7,
       deliverable/exit hardening incl. time-box; ADR-0003 outcome section);
       stage-scope conditions recorded in the gate record as S10 entry
       criteria. S10 status cell updated to "planned — Gate 3 PASSED".
5. [ ] Final step (mandatory): `bash harness/scripts/backup.sh "<desc>"`.

## Scope guard (binding for this plan)

S10 is **added to the roadmap only** — this plan performs NO study
execution: no vendor downloads, no installer unpacking, no code changes.
Execution starts in a later plan and only if Gate 3 reached consensus.
