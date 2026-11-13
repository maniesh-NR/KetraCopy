# Backup log (append-only)

One row per backup. Backups are **event-driven** (`WORKFLOW.md` §1): the
final step of every execution plan — never a fixed time interval.
`harness/scripts/backup.sh` appends a row on every run.

| UTC timestamp | Commit | Trigger / description |
|---|---|---|
| 2026-11-13T10:27:11Z | `e453057` | S4 complete: harness bootstrapped; clean 1.2.0 baseline force-pushed (pre-clean history had leaked the PAT file; remote v1.1.0 tag+release deleted; PAT rotation advised) (push: failed) |
| 2026-11-13T10:30:10Z | `a8f67c2` | S4 complete: harness bootstrapped; clean 1.2.0 baseline force-pushed over pre-clean remote history (which had leaked the owner PAT file); remote v1.1.0 tag+release deleted; PAT rotation advised; outer sandbox repo defused (no remote, push blocked) (push: ok) |
