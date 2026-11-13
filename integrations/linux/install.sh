#!/usr/bin/env bash
# =============================================================================
# integrations/linux/install.sh — install/uninstall KetraCopy actions for the
# detected file managers (Nemo, Caja, Thunar, Nautilus, Dolphin).
#
# Usage:
#   ./install.sh                 # install for every DETECTED file manager
#   ./install.sh --force         # install for ALL supported file managers
#   ./install.sh --uninstall     # remove everything this script installs
#   ./install.sh --help
#
# What gets installed:
#   ~/.local/bin/ketra-pick                              picker wrapper (always)
#   ~/.local/share/nemo/actions/nemo-action.ketra-*.nemo_action   Nemo
#   ~/.local/share/caja/scripts/ketra-{copy,move}.sh     Caja
#   ~/.local/share/nautilus/scripts/ketra-{copy,move}.sh Nautilus
#   ~/.config/Thunar/uca.xml                             merged (backup kept)
#   ~/.local/share/kio/servicemenus/ketracopy.desktop    Dolphin
#
# Every action ends up calling:  ketra copy|move|test <selection> -d <folder>
# via ~/.local/bin/ketra-pick (zenity/kdialog/yad picker, Flatpak-aware).
# =============================================================================
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PICK_SRC="$SCRIPT_DIR/ketra-pick.sh"
PICK_DST="$HOME/.local/bin/ketra-pick"
BIN_DIR="$HOME/.local/bin"

FORCE=0
UNINSTALL=0
for arg in "$@"; do
    case "$arg" in
        --force)     FORCE=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help)
            sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option: $arg (try --help)" >&2
            exit 2
            ;;
    esac
done

have() { command -v "$1" >/dev/null 2>&1; }

fill_placeholders() {
    # $1 = source template, $2 = destination (repo files are never modified)
    sed "s|__KETRA_PICK__|$PICK_DST|g" "$1" > "$2"
}

# ---------------------------------------------------------------------------
# Wrapper
# ---------------------------------------------------------------------------
install_wrapper() {
    mkdir -p "$BIN_DIR"
    install -m 755 "$PICK_SRC" "$PICK_DST"
    echo "installed wrapper:   $PICK_DST"
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) echo "NOTE: $BIN_DIR is not in PATH — add it (e.g. in ~/.profile)." ;;
    esac
}

# ---------------------------------------------------------------------------
# Nemo
# ---------------------------------------------------------------------------
NEMO_DIR="$HOME/.local/share/nemo/actions"

install_nemo() {
    mkdir -p "$NEMO_DIR"
    local f base
    for f in "$SCRIPT_DIR"/nemo/*.nemo_action; do
        if [ ! -e "$f" ]; then
            return 0
        fi
        base="$(basename "$f")"
        fill_placeholders "$f" "$NEMO_DIR/$base"
        echo "installed Nemo:      $NEMO_DIR/$base"
    done
}

uninstall_nemo() {
    if [ -d "$NEMO_DIR" ]; then
        rm -f "$NEMO_DIR"/nemo-action.ketra-*.nemo_action
        echo "removed Nemo actions"
    fi
}

# ---------------------------------------------------------------------------
# Caja (scripts; same protocol as Nautilus)
# ---------------------------------------------------------------------------
CAJA_DIR="$HOME/.local/share/caja/scripts"

install_caja() {
    mkdir -p "$CAJA_DIR"
    install -m 755 "$SCRIPT_DIR/caja/ketra-copy.sh" "$CAJA_DIR/ketra-copy.sh"
    install -m 755 "$SCRIPT_DIR/caja/ketra-move.sh" "$CAJA_DIR/ketra-move.sh"
    echo "installed Caja:      $CAJA_DIR/ketra-{copy,move}.sh"
}

uninstall_caja() {
    if [ -d "$CAJA_DIR" ]; then
        rm -f "$CAJA_DIR/ketra-copy.sh" "$CAJA_DIR/ketra-move.sh"
        echo "removed Caja scripts"
    fi
}

# ---------------------------------------------------------------------------
# Nautilus (scripts)
# ---------------------------------------------------------------------------
NAUTILUS_DIR="$HOME/.local/share/nautilus/scripts"

install_nautilus() {
    mkdir -p "$NAUTILUS_DIR"
    install -m 755 "$SCRIPT_DIR/nautilus/ketra-copy.sh" "$NAUTILUS_DIR/ketra-copy.sh"
    install -m 755 "$SCRIPT_DIR/nautilus/ketra-move.sh" "$NAUTILUS_DIR/ketra-move.sh"
    echo "installed Nautilus:  $NAUTILUS_DIR/ketra-{copy,move}.sh"
}

uninstall_nautilus() {
    if [ -d "$NAUTILUS_DIR" ]; then
        rm -f "$NAUTILUS_DIR/ketra-copy.sh" "$NAUTILUS_DIR/ketra-move.sh"
        echo "removed Nautilus scripts"
    fi
}

# ---------------------------------------------------------------------------
# Thunar (uca.xml merge via python3; manual instructions otherwise)
# ---------------------------------------------------------------------------
UCA_DIR="$HOME/.config/Thunar"
UCA="$UCA_DIR/uca.xml"
UCA_SNIPPET="$SCRIPT_DIR/thunar/uca-snippet.xml"

install_thunar() {
    mkdir -p "$UCA_DIR"
    if have python3; then
        python3 - "$UCA" "$UCA_SNIPPET" "$PICK_DST" <<'PYEOF'
import os, shutil, sys
import xml.etree.ElementTree as ET

uca_path, snippet_path, pick = sys.argv[1], sys.argv[2], sys.argv[3]

if not os.path.exists(uca_path):
    tree = ET.parse(snippet_path)
    for cmd in tree.getroot().iter("command"):
        if cmd.text:
            cmd.text = cmd.text.replace("__KETRA_PICK__", pick)
    tree.write(uca_path, encoding="utf-8", xml_declaration=True)
    print("installed Thunar:    %s (created)" % uca_path)
    sys.exit(0)

tree = ET.parse(uca_path)
root = tree.getroot()
if root.tag != "actions":
    print("Thunar uca.xml has unexpected root <%s>; SKIPPING merge." % root.tag)
    print("Manual: merge %s into %s yourself." % (snippet_path, uca_path))
    sys.exit(1)

existing = set()
for a in root.findall("action"):
    existing.add((a.findtext("name") or "").strip())

snippet = ET.parse(snippet_path).getroot()
added = 0
for a in snippet.findall("action"):
    name = (a.findtext("name") or "").strip()
    if name in existing:
        continue
    for cmd in a.iter("command"):
        if cmd.text:
            cmd.text = cmd.text.replace("__KETRA_PICK__", pick)
    root.append(a)
    added += 1

if added:
    shutil.copyfile(uca_path, uca_path + ".bak")
    tree.write(uca_path, encoding="utf-8", xml_declaration=True)
    print("installed Thunar:    %s (+%d action(s), backup: uca.xml.bak)" % (uca_path, added))
else:
    print("installed Thunar:    already present in %s" % uca_path)
PYEOF
    else
        # Be honest: merging XML without python3 is risky, print instructions.
        echo "Thunar: python3 not found — refusing to edit $UCA blindly."
        echo "  Manual merge: open $UCA_SNIPPET and copy the two <action>"
        echo "  blocks into the <actions> root of $UCA, replacing"
        echo "  __KETRA_PICK__ with $PICK_DST (keep a backup first)."
    fi
}

uninstall_thunar() {
    if [ ! -f "$UCA" ]; then
        return 0
    fi
    if have python3; then
        python3 - "$UCA" <<'PYEOF'
import os, sys
import xml.etree.ElementTree as ET
path = sys.argv[1]
if not os.path.exists(path):
    sys.exit(0)
tree = ET.parse(path)
root = tree.getroot()
removed = 0
for a in list(root.findall("action")):
    if (a.findtext("name") or "").startswith("KetraCopy:"):
        root.remove(a)
        removed += 1
if removed:
    shutil_bak = path + ".bak"
    import shutil
    shutil.copyfile(path, shutil_bak)
    tree.write(path, encoding="utf-8", xml_declaration=True)
    print("removed Thunar:      %d action(s) from uca.xml (backup kept)" % removed)
else:
    print("removed Thunar:      nothing to remove")
PYEOF
    else
        echo "Thunar: python3 not found — remove the two 'KetraCopy:' actions"
        echo "  from $UCA manually."
    fi
}

# ---------------------------------------------------------------------------
# Dolphin (KDE service menu)
# ---------------------------------------------------------------------------
DOLPHIN_DIR="$HOME/.local/share/kio/servicemenus"

install_dolphin() {
    mkdir -p "$DOLPHIN_DIR"
    fill_placeholders "$SCRIPT_DIR/dolphin/ketracopy.desktop" "$DOLPHIN_DIR/ketracopy.desktop"
    echo "installed Dolphin:   $DOLPHIN_DIR/ketracopy.desktop"
}

uninstall_dolphin() {
    if [ -f "$DOLPHIN_DIR/ketracopy.desktop" ]; then
        rm -f "$DOLPHIN_DIR/ketracopy.desktop"
        echo "removed Dolphin service menu"
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
if [ "$UNINSTALL" -eq 1 ]; then
    echo "Removing KetraCopy file-manager integrations..."
    if [ -f "$PICK_DST" ]; then
        rm -f "$PICK_DST"
        echo "removed wrapper:     $PICK_DST"
    fi
    uninstall_nemo
    uninstall_caja
    uninstall_thunar
    uninstall_nautilus
    uninstall_dolphin
    echo "Done."
    exit 0
fi

install_wrapper

DETECTED=0
install_for() {
    # $1 = binary to detect, $2 = installer function
    if have "$1"; then
        DETECTED=$((DETECTED + 1))
        echo "--- $1 detected"
        "$2"
    fi
}

install_for nemo     install_nemo
install_for caja     install_caja
install_for thunar   install_thunar
install_for nautilus install_nautilus
install_for dolphin  install_dolphin

if [ "$FORCE" -eq 1 ]; then
    echo "--- --force: installing for ALL supported file managers"
    install_nemo
    install_caja
    install_thunar
    install_nautilus
    install_dolphin
elif [ "$DETECTED" -eq 0 ]; then
    echo "No supported file manager found in PATH (nemo/caja/thunar/nautilus/dolphin)."
    echo "Re-run with --force to install for all of them anyway."
fi

echo ""
echo "Done. Restart the file manager to see the new actions (e.g. 'nemo -q')."
echo "Picker tools used by the wrapper: zenity, kdialog or yad (first found)."
echo "In Flatpak sandboxes the wrapper calls 'flatpak-spawn --host ketra'."
exit 0
