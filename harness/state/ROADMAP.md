# KetraCopy — Roadmap & Gate Status

> Single source of truth for stage progress. Every stage transition
> requires a PASSED adversarial gate (`WORKFLOW.md` §2); gate records live
> in `../reviews/`.

| # | Stage | Status | Gate |
|---|---|---|---|
| S0 | 1.0.0 — core engine, CLI, MD3 GUI, packaging assets | done 2026-11-02 | pre-workflow |
| S1 | 1.1.0 — LAN transfer, clipboard engine, OS shell integration, Android track, GUI rebuild | done 2026-11-05 | pre-workflow |
| S2 | 1.1.1 — interop test campaign (106 checks) + fix sweep | done 2026-11-05 | pre-workflow |
| S3 | 1.2.0 — performance / stability / memory optimization pass | done 2026-11-05 | pre-workflow |
| S4 | Harness workflow + GitHub backup infrastructure (owner mandate 2026-11-13) | done 2026-11-13 | n/a (this stage IS the workflow) |
| S5 | Full re-verification of 1.2.0 — evidence pack (unit / integration / interop / bench / cross-checks) | done 2026-11-13 — findings handed to Gate 1: macOS-GUI objc2 break on modern rustc + packaging nits (`../evidence/2026-11-13-S5/README.md`) | — |
| S6 | **Gate 1** — adversarial consensus: work-so-far + roadmap | pending | requires S5 evidence |
| S7 | Address Gate-1 findings; rebuild 1.2.0 dist artifacts (Linux + zig cross-builds) + artifact-hash baseline | pending | requires S6 PASSED |
| S8 | **Gate 2** — release-readiness consensus | pending | requires S7 |
| S9 | **Release v1.2.0** — tag, GitHub release page with packages for all targets (sandbox assets + `release.yml` native builds), ATTRIBUTIONS refresh, topics | pending | requires S8 PASSED |

## Post-1.2.0 roadmap (candidates, not yet scheduled)

- HTTPS LAN receiver (self-signed certs, strict LocalSend v2 compat) —
  rustls design preserved in `attic/ketra-net`; engine stays std-only via a
  separate `ketra-tls` crate (needs its own adversarial gate).
- Android APK distribution — CI (`.github/workflows/build-apk.yml`) already
  builds it on every push; wire the APK into release assets.
- Flatpak publish (`packaging/flatpak/`).
- Live interop testing against the real LocalSend application.
- GUI/CLI i18n.

## Stage/gate history

- 2026-11-13 — S4 done: harness bootstrapped, clean history (ADR-0002),
  first backup pushed to GitHub. Security: the pre-clean remote history
  (which contained the owner's PAT file) was force-pushed away; remote
  `v1.1.0` tag + release deleted; **PAT rotation advised** (MEMORY.md
  "Security incident").
