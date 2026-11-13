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

git add -A
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
