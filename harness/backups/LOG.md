# Backup log (append-only)

One row per backup. Backups are **event-driven** (`WORKFLOW.md` §1): the
final step of every execution plan — never a fixed time interval.
`harness/scripts/backup.sh` appends a row on every run.

| UTC timestamp | Commit | Trigger / description |
|---|---|---|
| 2026-11-13T10:27:11Z | `e453057` | S4 complete: harness bootstrapped; clean 1.2.0 baseline force-pushed (pre-clean history had leaked the PAT file; remote v1.1.0 tag+release deleted; PAT rotation advised) (push: failed) |
| 2026-11-13T10:30:10Z | `a8f67c2` | S4 complete: harness bootstrapped; clean 1.2.0 baseline force-pushed over pre-clean remote history (which had leaked the owner PAT file); remote v1.1.0 tag+release deleted; PAT rotation advised; outer sandbox repo defused (no remote, push blocked) (push: ok) |
| 2026-11-13T10:46:24Z | `6854c44` | S5 complete: 1.2.0 re-verified — 129 unit / 39 integration / 103-of-106 interop green; win+mac-CLI cross-checks green; mac-GUI objc2 finding + packaging nits recorded for Gate 1; dist 1.2.0 rebuilt with hash baseline (push: ok) |
| 2026-11-13T11:58:53Z | `ac9481e` | S7 complete: all Gate-1 findings fixed (LAN reservation CRITICAL fix, Error::Shrank, --no-times, Content-Length cap, journal sync-before-record, minors) + NEW latent bug fixed (--conflict overwrite / GUI overwrite-all EEXIST); 1.2.1: 133 unit / 41 integration / 105-of-105 interop green; cross-checks green; docs+CI honesty pass; dist rebuilt with hash baseline; Windows/macOS binaries moved to native CI builds (zigbuild abandoned with evidence) (push: failed) |
| 2026-11-13T12:02:09Z | `1f074c8` | S7 recovery: project .git recreated on top of remote head after the sandbox snapshot automation deleted it (MEMORY.md note added; backup.sh now aborts when the repo root is not the workspace); outer sandbox repo defused again (push: ok) |
