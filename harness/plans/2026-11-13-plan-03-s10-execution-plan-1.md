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
3. [x] **RT-1 (first commit of S10)**: amend `docs/CLEANROOM.md` —
       time-scoped the absolute claims to ≤ v1.2.1; disclosed the S10
       two-track methodology; stated the private-analysis boundary +
       bar; consistency-swept other docs (only CLEANROOM.md carried
       absolute claims; README/FEATURE-PARITY/ATTRIBUTIONS claims remain
       true and point at CLEANROOM.md). **Backup #1 = `c138d82`** (S10's
       first commit, before any acquisition).
4. [x] **Track B pre-flight (ST-4)**: innoextract 1.9 acquired rootless
       (works — irrelevant: the vendor ships Advanced Installer, not Inno
       Setup); **7-Zip 25.01** acquired rootless from Debian's `7zip`
       package — the actual unpacking tool; pefile via pip. Recorded in
       the study doc §3.
5. [x] **Acquisition (RT-4)**: discovery — the owner's two URLs both
       serve the current 4.0.5 (byte-identical); the real 3.17 was found
       still hosted on the vendor CDN (`teracopy3.17.exe`), plus the
       vendor's own standalone MSIs. Four artifacts acquired to
       `/home/z/my-project/research/teracopy/` (outside the repo); all
       four SHA-256 hashes denylisted in `backup.sh` (commit rides with
       the plan-end backup — nothing else repo-visible changed at
       acquisition time).
6. [x] **Track B inventory**: both bootstrappers' AI container mapped
       (two back-to-back 7z archives, 32-byte start headers XOR-0xFF —
       CRC-verified — content streams protected beyond XOR and NOT
       circumvented); payload inventories for both versions; 3.17
       binaries obtained via the vendor MSI channel → full PE inventory
       (`pe_inv.py`): Delphi-built, packed GUI, manual synchronous
       ReadFile/WriteFile engine (no CopyFileEx), VSS+overlapped service,
       SIMD hash DLLs (BLAKE3/XXH3 AVX2+SSE2). 4.0.5: inventory deltas
       (service dropped, TeraCopy.dll −86%, ARM64X, .appx); PE facts
       deferred to the owner-rig export per the charter degradation
       path.
7. [x] Research docs written with scope headers + provenance tags:
       `docs/research/teracopy-reference-study.md` (methodology,
       acquisition, Track-A owner kit §4) and
       `docs/research/teracopy-findings.md` ([B] facts + candidate map
       C1–C8, verdicts reserved for the findings gate); six reproducible
       tools in `scripts/research/`.
8. [x] ROADMAP S10 status cell updated (in progress — plan 1 done) and
       this plan's checkboxes updated.
9. [ ] **Final step (mandatory)**: `bash harness/scripts/backup.sh "<desc>"`.

## Handed to plan 2 (S10 exit)

- Owner runs the Track A checklist on their Windows rig and reports the
  side-by-side numbers (or plan 2 proceeds without them, explicitly).
- Findings gate: per-candidate verdicts (adopt → S11 row / defer / reject),
  tree contamination check, ≤ 2-plan time-box audit.
