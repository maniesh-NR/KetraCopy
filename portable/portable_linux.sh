#!/usr/bin/env bash
# =============================================================================
# portable/portable_linux.sh — build + bundle the portable Linux version.
#
# Produces:
#   KetraCopy-portable/                        (self-contained folder)
#     ketra, ketracopy-gui (if built), LICENSE, portable.flag, README-portable.txt
#   dist/ketracopy-1.2.1-portable-linux-<arch>.tar.gz
#
# The empty file "portable.flag" next to the ketra binary switches the engine
# into portable mode: settings are read/written as ketra.json next to the
# binary instead of ~/.config/ketra/ketra.json
# (see crates/ketra-core/src/settings.rs).
#
# Cross-run guard: exits gracefully when not executed on Linux.
# =============================================================================
set -euo pipefail

VERSION="1.2.1"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
OUT="$ROOT/KetraCopy-portable"
DIST="$ROOT/dist"

OS="$(uname -s)"
if [ "$OS" != "Linux" ]; then
    echo "ERROR: portable_linux.sh is for Linux (uname -s = '$OS')." >&2
    echo "Use portable_windows.cmd (Windows) or portable_macos.sh (macOS)." >&2
    exit 1
fi

ARCH="$(uname -m)"

# ---- build (or reuse an existing release build) ------------------------------
if [ ! -x "$ROOT/target/release/ketra" ]; then
    echo "==> cargo build --release -p ketra-cli"
    (cd "$ROOT" && cargo build --release -p ketra-cli)
fi
if [ ! -x "$ROOT/target/release/ketra" ]; then
    echo "ERROR: target/release/ketra missing and build did not produce it." >&2
    exit 1
fi

# ---- assemble -----------------------------------------------------------------
echo "==> assembling $OUT"
rm -rf "$OUT"
mkdir -p "$OUT"
install -m 755 "$ROOT/target/release/ketra" "$OUT/ketra"
if [ -x "$ROOT/target/release/ketracopy-gui" ]; then
    install -m 755 "$ROOT/target/release/ketracopy-gui" "$OUT/ketracopy-gui"
else
    echo "NOTE: ketracopy-gui not built - bundling CLI only."
fi
if [ -f "$ROOT/LICENSE" ]; then
    install -m 644 "$ROOT/LICENSE" "$OUT/LICENSE"
fi
# Empty portable.flag -> portable mode (settings next to the binary):
: > "$OUT/portable.flag"

cat > "$OUT/README-portable.txt" <<EOF
KetraCopy $VERSION — portable (Linux)

This folder is self-contained. The empty file "portable.flag" tells
KetraCopy to keep its settings in ketra.json next to the binaries instead of
~/.config/ketra/ketra.json (see crates/ketra-core/src/settings.rs).

Run:
  ./ketra copy SRC -d DEST --verify inline
  ./ketra --help
  ./ketracopy-gui                (if present)

Requires glibc 2.17+ (roughly Debian 8 / Mint 17.3 / anything from 2014 on).
Nothing is installed; delete the folder to remove KetraCopy completely.
EOF

# ---- tarball -------------------------------------------------------------------
mkdir -p "$DIST"
TARBALL="$DIST/ketracopy-$VERSION-portable-linux-$ARCH.tar.gz"
rm -f "$TARBALL"
tar -czf "$TARBALL" -C "$ROOT" KetraCopy-portable

echo ""
echo "Portable dir: $OUT"
echo "Tarball:      $TARBALL"
exit 0
