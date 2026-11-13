#!/usr/bin/env bash
# =============================================================================
# Caja script: copy the selected items with KetraCopy.
# Installed by integrations/linux/install.sh into ~/.local/share/caja/scripts/
# (Caja uses the same script protocol as Nautilus:
#  $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS = newline-separated absolute paths).
# =============================================================================
set -u

PICK="$HOME/.local/bin/ketra-pick"
if [ ! -x "$PICK" ]; then
    echo "KetraCopy: $PICK not found (run integrations/linux/install.sh)." >&2
    exit 1
fi

ARGS=()
while IFS= read -r p; do
    if [ -n "$p" ]; then
        ARGS+=("$p")
    fi
done <<EOF
${NAUTILUS_SCRIPT_SELECTED_FILE_PATHS:-}
EOF

if [ "${#ARGS[@]}" -eq 0 ]; then
    exit 0
fi

exec "$PICK" copy "${ARGS[@]}"
