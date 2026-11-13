# ADR-0002: Clean-history restart for the public repository

Date: 2026-11-13 · Status: ACCEPTED

## Context

The repository's pre-workflow history (6 commits) was created without
discipline: commit messages were bare UUIDs, and 88 files under `target/`
(build fingerprints, incremental-compile `.lock` files) were tracked
because they were committed before `.gitignore` covered them. The history
was unsuitable for a public repository.

## Decision

- Publish a fresh clean history: one baseline commit ("KetraCopy 1.2.0")
  on `main`, created from the exact working tree after cleanup.
- The old history is preserved locally in branch `archive/pre-workflow`
  (never pushed) in case anything needs to be recovered.
- Stale working-tree duplicates `crates/ketra-net` and `crates/ketra-ffi`
  (byte-identical to the `attic/` prototypes, not workspace members) were
  deleted from disk.
- `git rm -r --cached target` untracked the build junk; the new
  `.gitignore` covers `/target/`, `/dist/`, `/KetraCopy-portable/`,
  Android build dirs, session/tooling junk.

## Consequences

- The public repo contains only source, docs, harness and CI config.
- History from 2026-11-13 onward is real work history: every backup
  commit carries a meaningful message and lands on GitHub.
- Version tags start at the 1.2.0 baseline (`v1.2.0` at release time);
  the pre-workflow history is not part of the public record.
