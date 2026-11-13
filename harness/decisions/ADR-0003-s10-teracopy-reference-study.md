# ADR-0003: Schedule S10 — TeraCopy reference study (3.17 / 4.0.5)

Date: 2026-11-13 · Status: ACCEPTED (owner directive; execution gated by Gate 3)

## Context

The project owner directed on 2026-11-13:

> "Add to roadmap: decompile and reverse engineer the install files for
> teracopy 3.17 (https://codesector.com/files/teracopy.exe) and teracopy
> 4.0.5 (https://codesector.com/files/teracopy4.0.5.exe), to study if
> there are any methods for local file transfer and verification that can
> be incorporated in to this project to improve performance, stability, and
> resource use."

Tension to resolve: the project's public clean-room statement
(`docs/CLEANROOM.md`) says the v1.x design was derived exclusively from
public, observable behavior and that "no decompilation or disassembly of
TeraCopy was performed, and no binary analysis of any kind was used" —
true for all work through v1.2.1. The directive now proposes binary
analysis of the reference product to inform future optimization. TeraCopy
is proprietary software (Code Sector); its license terms and
jurisdiction-specific reverse-engineering law constrain what the project
records in its **public** repository. The repo is public and v1.2.1 is
already released, so methodology decisions are externally visible.

## Decision

1. Schedule the study as **stage S10** (charter in `state/ROADMAP.md`),
   executed with a **two-track method**:
   - **Track A (primary)** — dynamic black-box behavioral analysis on
     the owner's Windows machine (owner-controlled hardware only — the
     CI-runner option was considered and REMOVED by Gate 3: a public
     repo's workflow logs and artifacts are publication channels, and
     hosted runners are not owner-controlled systems): observation via OS
     instrumentation only (Process Monitor, ETW/xperf, API traces). This
     extends the "behavior any user can observe" methodology the
     clean-room statement already blesses. Every scenario co-instruments
     KetraCopy v1.2.1 on the same rig for a side-by-side baseline.
   - **Track B (secondary)** — static inventory of the installers in the
     sandbox, **limited to functional interface facts**: payload file
     inventory (names/sizes/versions), PE machine/subsystem/section
     summary, DLL import lists (API names), version resources. Decompiler
     or disassembler output is never committed to the repository.
2. **Binding clean-room guardrails** (full list in the charter):
   vendor binaries live outside the repo (`/home/z/my-project/research/`)
   and are never committed; the repo records functional observations only
   (behavior notes, API names, sizes, timings — no decompiled code, no
   disassembly listings, no extracted vendor assets); any resulting
   KetraCopy implementation is written from scratch against the functional
   notes; `docs/CLEANROOM.md` is amended early in S10 to honestly disclose
   the post-1.2.1 methodology change; trademark hygiene is preserved.
3. **Gate 3 must pass on this charter before S10 executes** (WORKFLOW §2).
   A **fresh gate on the S10 findings** is required before any follow-on
   optimization stage (S11 — deliberately NOT scheduled; it exists only if
   S10 produces adoptable candidates).
4. On the owner's word "decompile": analysis on owner-controlled systems
   may go deeper than the project's committed record. What the public
   repository holds stays behavioral + interface facts, per the guardrails.
   The adversarial panel adjudicates whether these limits are the right
   ones for a public repository; a BLOCK verdict sends the charter back for
   revision.

## Gate 3 outcome (2026-11-13)

The charter gate **PASSED**: Red Team, Auditor and Strategist each
returned PROCEED-WITH-CONDITIONS (zero BLOCK, consensus satisfied).
Conditions applied at gate time: venue pinned to owner-controlled
hardware (CI option removed), Track B relabeled with a feasibility
pre-flight (sandbox lacks unpacking tooling — verified), guardrails
extended with provenance labels [A]/[B] and the mechanical vendor-material
backstop (.gitignore layer + backup pre-flight, implemented and tested),
co-instrumentation baseline required, per-candidate verdicts + tree
contamination check at the findings gate, time-box ≤ 2 execution plans.
The binding conditions table lives in
`harness/reviews/2026-11-13-gate-3-s10-charter.md`; the hardened charter
in `harness/state/ROADMAP.md` is the authoritative stage text.

## Consequences

- The project gains a defensible path to mine reference-product techniques
  (I/O strategy, buffering, verification timing, resume/throttle behavior,
  memory profile) for a future optimization pass without breaking its
  clean-room posture.
- `docs/CLEANROOM.md` must not become misleading: the disclosure amendment
  ships **inside S10, before any findings land** — a stage exit criterion.
- Legal exposure is minimized, not eliminated: reverse-engineering law is
  jurisdiction-dependent; the owner directs the study and accepts the
  residual risk. Nothing in this ADR is legal advice.
- The other post-1.2.1 candidates (HTTPS LAN receiver, APK release wiring,
  Flatpak publish, live LocalSend interop, i18n) remain unscheduled and
  unaffected by this ADR.
