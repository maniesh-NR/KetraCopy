#!/usr/bin/env bash
# =============================================================================
# harness/scripts/backup.sh — KetraCopy event-driven backup (owner-mandated).
#
#   harness/scripts/backup.sh "description of what this backup captures"
#
# Commits the whole working tree and pushes it to GitHub (origin/main).
# This is the FINAL step of every execution plan (harness/WORKFLOW.md §1).
#
# Never commit secrets: the GitHub PAT lives OUTSIDE the repo
# (/home/z/my-project/.secrets/ in the sandbox). If credentials are lost
# after a sandbox recycle, see harness/state/MEMORY.md for the setup.
# =============================================================================
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

DESC="${1:-incremental backup}"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Guard: the project repo must be rooted at the workspace itself. The
# sandbox snapshot automation once DELETED download/ketracopy/.git
# (2026-11-13), which silently redirected backups into the OUTER
# local-only sandbox repo (whose origin was removed and whose pre-push
# hook blocks pushes — see MEMORY.md). Abort loudly instead.
GIT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ "$GIT_ROOT" != "$ROOT" ]; then
  echo "backup: FATAL — git repo root is '${GIT_ROOT:-none}', expected '$ROOT'."
  echo "backup: the project .git was probably deleted again — recover per"
  echo "backup: harness/state/MEMORY.md ('snapshot automation' note) before committing."
  exit 1
fi

git add -A

# --- Gate 3 vendor-material guard (ADR-0003 guardrail 7; record:
# harness/reviews/2026-11-13-gate-3-s10-charter.md). Discipline alone has
# failed this repo before (PAT incident), so this is a MECHANICAL abort:
# no staged file may look like vendor research material — denylisted
# extensions, executable/OLE magic bytes, OR a known vendor-artifact
# SHA-256 (S10 acquisition, 2026-11-13 — see docs/research/teracopy-
# reference-study.md §1). Layer 2 of 2 (layer 1 = .gitignore).
V317_EXE=ac03db52bb68a013d3a8bf4db703ec11976fa1d0aa557eb988e3f926a26656cf
V405_EXE=22f050b51c80a344cbdaf368c7f6a9266efca4d95b5109d606466906a421ff7e
V317_MSI=caf5ac31c758788e7df7cf1332afed8d1c367806703fa9338345a90f2f98738a
VCUR_MSI=255891c95914c4649a61815365526e769e54f311fc709cbba8835bef40095bdc
GUARD_BAD=""
shopt -s nocasematch
while IFS= read -r f; do
  [ -f "$f" ] || continue
  case "$f" in
    *.exe|*.dll|*.msi|*.sys|*.pml|*.etl|*.cab|*.scr|*.com)
      GUARD_BAD="${GUARD_BAD}
  - $f (denylisted extension)"
      continue ;;
  esac
  magic="$(od -An -tx1 -N2 -- "$f" 2>/dev/null | tr -d ' \n')"
  case "$magic" in
    4d5a|d0cf)
      GUARD_BAD="${GUARD_BAD}
  - $f (executable/OLE magic bytes: ${magic})" ;;
  esac
  f_hash="$(sha256sum -- "$f" 2>/dev/null | cut -d' ' -f1 || true)"
  case "$f_hash" in
    "$V317_EXE"|"$V405_EXE"|"$V317_MSI"|"$VCUR_MSI")
      GUARD_BAD="${GUARD_BAD}
  - $f (known vendor-artifact SHA-256)" ;;
  esac
done < <(git diff --cached --name-only --diff-filter=ACMR)
shopt -u nocasematch
if [ -n "$GUARD_BAD" ]; then
  echo "backup: FATAL — vendor-material guard tripped on staged files:${GUARD_BAD}"
  echo "backup: that material belongs OUTSIDE the repo (ADR-0003). Unstage/relocate it and retry."
  exit 1
fi

if git diff --cached --quiet; then
  echo "backup: nothing to commit"
else
  git commit -q -m "backup: ${DESC}"
fi

if git push -q origin HEAD 2>/dev/null; then
  PUSH=ok
else
  PUSH=failed
  echo "backup: WARNING — commit saved locally, push FAILED (network/credentials?)"
fi

H="$(git rev-parse --short HEAD)"
printf '| %s | `%s` | %s (push: %s) |\n' "$TS" "$H" "$DESC" "$PUSH" >> harness/backups/LOG.md
git add harness/backups/LOG.md
git commit -q -m "backup: log ${TS}" || true
git push -q origin HEAD || echo "backup: WARNING — log-entry push failed"

echo "backup: ${TS} ${H} — ${DESC} (push: ${PUSH})"
