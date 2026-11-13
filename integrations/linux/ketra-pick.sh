#!/usr/bin/env bash
# =============================================================================
# integrations/linux/ketra-pick.sh — destination-folder picker wrapper used
# by all KetraCopy file-manager integrations (Nemo actions, Caja/Nautilus
# scripts, Thunar custom actions, Dolphin service menu).
#
# Usage:
#   ketra-pick copy|move|test <selected path> [<selected path> ...]
#
# Behaviour:
#   1. locates the ketra binary (see order below),
#   2. asks for a destination folder (zenity, kdialog or yad — first found;
#      override with KETRA_PICKER=zenity|kdialog|yad),
#   3. runs:  ketra <action> <paths...> -d <picked folder>
#      (the ketra CLI requires -d for copy, move AND test — see
#       crates/ketra-cli/src/args.rs)
#
# ketra lookup order:
#   1. $KETRA_BIN (explicit override)
#   2. inside a Flatpak sandbox: flatpak-spawn --host ketra
#      (requires --talk-name=org.freedesktop.Flatpak on the app)
#   3. ketra in PATH
#   4. /usr/local/bin/ketra
#   5. ~/.cargo/bin/ketra
#
# Exit codes: 0 ok / canceled, 1 ketra missing, 2 bad usage, 3 no paths.
# =============================================================================
set -u

ACTION="${1:-copy}"
shift 2>/dev/null || true
case "$ACTION" in
    copy|move|test) ;;
    *) ACTION="copy" ;;
esac

if [ "$#" -eq 0 ]; then
    echo "ketra-pick: no paths given (usage: ketra-pick copy|move|test PATH...)" >&2
    exit 3
fi

# ---- locate the ketra binary ------------------------------------------------
KETRA_CMD=()
if [ -n "${KETRA_BIN:-}" ]; then
    KETRA_CMD=("$KETRA_BIN")
elif [ -n "${FLATPAK_ID:-}" ] || [ -f /.flatpak-info ]; then
    # Running inside a Flatpak sandbox: execute the CLI on the host.
    KETRA_CMD=(flatpak-spawn --host ketra)
elif command -v ketra >/dev/null 2>&1; then
    KETRA_CMD=(ketra)
elif [ -x /usr/local/bin/ketra ]; then
    KETRA_CMD=(/usr/local/bin/ketra)
elif [ -x "$HOME/.cargo/bin/ketra" ]; then
    KETRA_CMD=("$HOME/.cargo/bin/ketra")
fi

if [ "${#KETRA_CMD[@]}" -eq 0 ]; then
    if command -v notify-send >/dev/null 2>&1; then
        notify-send "KetraCopy" "The 'ketra' binary was not found (PATH, /usr/local/bin, ~/.cargo/bin)."
    fi
    echo "KetraCopy: the 'ketra' binary was not found." >&2
    exit 1
fi

# ---- destination folder picker ----------------------------------------------
DEST=""
TITLE="KetraCopy $ACTION: choose destination folder"
PICKER="${KETRA_PICKER:-}"
if [ -n "$PICKER" ] && command -v "$PICKER" >/dev/null 2>&1; then
    case "$PICKER" in
        zenity)  DEST="$(zenity --file-selection --directory --title "$TITLE" 2>/dev/null)" ;;
        kdialog) DEST="$(kdialog --getexistingdirectory "$HOME" --title "$TITLE" 2>/dev/null)" ;;
        yad)     DEST="$(yad --file --directory --title "$TITLE" 2>/dev/null)" ;;
    esac
elif command -v zenity >/dev/null 2>&1; then
    DEST="$(zenity --file-selection --directory --title "$TITLE" 2>/dev/null)"
elif command -v kdialog >/dev/null 2>&1; then
    DEST="$(kdialog --getexistingdirectory "$HOME" --title "$TITLE" 2>/dev/null)"
elif command -v yad >/dev/null 2>&1; then
    DEST="$(yad --file --directory --title "$TITLE" 2>/dev/null)"
fi

if [ -z "$DEST" ]; then
    # User canceled, or no picker tool is installed. Say so (when possible),
    # but do not treat a cancel as an error.
    if command -v notify-send >/dev/null 2>&1; then
        notify-send "KetraCopy" "No destination folder chosen (needs zenity, kdialog or yad)."
    fi
    exit 0
fi

exec "${KETRA_CMD[@]}" "$ACTION" "$@" -d "$DEST"
