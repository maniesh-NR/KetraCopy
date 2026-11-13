# Plan 01 — 2026-11-13: Workflow bootstrap + full re-verification (S4–S6, stretch S7+)

**Trigger**: owner mandate (2026-11-13) — backups to GitHub as the last
step of every execution plan, harness-like memory & infrastructure in a
section of the GitHub repo, adversarial-agent consensus gates between
stages — plus "continue" (resume the pending roadmap).

**Discovery during planning**: the stale hand-off summary was wrong about
project state — the workspace is already at **1.2.0** (LAN, Android,
shell integration, clipboard, and the optimization pass are all in the
tree per `CHANGELOG.md`). What was genuinely missing: the entire harness
workflow (no remote, no tags, no harness/, junk git history).

## Steps

1. [x] Probe sandbox/toolchain; reinstall rustup (recycle had wiped it).
2. [x] Clean the repo: untrack 88 `target/` files, delete stale duplicate
       crates (`crates/ketra-net`, `crates/ketra-ffi` — identical to
       `attic/`), rewrite `.gitignore`, preserve old history locally as
       `archive/pre-workflow` (ADR-0002).
3. [x] Author `harness/` (WORKFLOW, MEMORY, ROADMAP, ADRs, backup script,
       backup log, review template, this plan).
4. [ ] Fresh clean baseline commit on `main`; create the GitHub repo
       (public), configure credentials, push. **Backup #1.**
5. [ ] S5 evidence pack: `cargo test --workspace`; `cargo build --release`;
       `scripts/test_integration.sh`; `scripts/test_interop.sh`;
       cross `cargo check` (win x64/x86, mac x64/aarch64); logs to
       `harness/evidence/2026-11-13-S5/`. **Backup #2.**
6. [ ] S6 Gate 1: three independent adversarial agents (Red Team /
       Auditor / Strategist) on work-so-far + roadmap; record consensus
       in `harness/reviews/`. **Backup #3.**
7. [ ] (stretch, gated on S6) S7: fix findings, zig cross-builds, rebuild
       dist, hash baseline. **Backup.**
8. [ ] (stretch, gated) S8 Gate 2 + S9 release: tag `v1.2.0`, GitHub
       release with packages for all targets, ATTRIBUTIONS refresh.

**Final step of this plan (mandatory)**: `bash harness/scripts/backup.sh`
— every prompt output ends with a backup, event-driven, never on a timer.
