# Plan 03 — 2026-11-13: S10 execution, plan 1 of ≤ 2 — CLEANROOM disclosure + Track B + Track-A kit

**Trigger**: owner "continue" → start S10 (TeraCopy reference study), per
the Gate 3 PASSED charter and its entry checklist
(`../reviews/2026-11-13-gate-3-s10-charter.md` §Follow-ups).

**State on entry**: repo healthy (root = workspace, `main == origin/main`
at `9150605`, tree clean). Time-box: S10 must complete within **2
execution plans** — this is plan 1; plan 2 runs the findings gate (exit).

## Steps (mapped to gate-record follow-ups)

1. [x] Bootstrap: repo root / sync / secrets verified — no `.git`
       deletion this time.
2. [x] Write this plan (final step = backup, per WORKFLOW §3).
3. [ ] **RT-1 (first commit of S10)**: amend `docs/CLEANROOM.md` —
       time-scope the absolute claims to ≤ v1.2.1; disclose the S10
       two-track methodology; state the private-analysis boundary +
       bar; consistency-sweep other docs for now-stale absolute claims.
       **Backup #1 immediately** so the amendment is S10's first commit,
       before any tool acquisition or vendor download (commit-order
       verification point at the findings gate).
4. [ ] **Track B pre-flight (ST-4)**: rootless unpacking-tool acquisition
       (`apt-get download` + `dpkg -x` into `/home/z/my-project/tools/`);
       record the result (or the degradation path: objdump PE facts on
       binaries exported from the owner's Windows rig).
5. [ ] **Acquisition (RT-4)**: download both installers to
       `/home/z/my-project/research/teracopy/` (OUTSIDE the repo);
       record sizes + SHA-256; add the two hashes to the `backup.sh`
       vendor-material denylist. **Backup #2** (acquisition record).
6. [ ] **Track B inventory**: unpack (pre-flight tool) + record functional
       interface facts only (payload file names/sizes/versions, PE
       machine/subsystem/sections, DLL import API names). Raw outputs
       stay outside the repo. No verbatim vendor copyright/trademark
       strings, no screenshots, no disassembly (RT-5).
7. [ ] Write `docs/research/teracopy-reference-study.md` (scope header,
       methodology, acquisition hashes, **Track A owner checklist** with
       co-instrumented KetraCopy v1.2.1 scenarios) and
       `docs/research/teracopy-findings.md` ([A]/[B] provenance tags;
       [B] facts from the inventory; candidate mapping with measured-delta
       columns pending Track A owner data).
8. [ ] Update ROADMAP S10 status cell (in progress — plan 1 done) and
       this plan's checkboxes.
9. [ ] **Final step (mandatory)**: `bash harness/scripts/backup.sh "<desc>"`.

## Handed to plan 2 (S10 exit)

- Owner runs the Track A checklist on their Windows rig and reports the
  side-by-side numbers (or plan 2 proceeds without them, explicitly).
- Findings gate: per-candidate verdicts (adopt → S11 row / defer / reject),
  tree contamination check, ≤ 2-plan time-box audit.
