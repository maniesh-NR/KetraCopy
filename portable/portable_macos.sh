#!/usr/bin/env bash
# =============================================================================
# portable/portable_macos.sh — build + bundle the portable macOS version.
#
# Produces:
#   dist/macos/KetraCopy.app                              (via make-app.sh)
#     Contents/MacOS/{ketra, ketracopy-gui?}, portable.flag, README-portable.txt
#   dist/KetraCopy-1.2.0-portable-macos.zip
#
# Portable-mode note (being honest about the spec):
#   crates/ketra-core/src/settings.rs looks for portable.flag NEXT TO THE
#   EXECUTABLE (Contents/MacOS). We therefore drop it there so the engine is
#   actually portable, and additionally place a copy in Contents/Resources
#   (as the product spec asks) so the layout documents intent either way.
#
# Cross-run guard: exits gracefully when not executed on macOS.
# =============================================================================
set -euo pipefail

VERSION="1.2.0"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
DIST="$ROOT/dist"

OS="$(uname -s)"
if [ "$OS" != "Darwin" ]; then
    echo "ERROR: portable_macos.sh is for macOS (uname -s = '$OS')." >&2
    echo "Use portable_windows.cmd (Windows) or portable_linux.sh (Linux)." >&2
    exit 1
fi

# Build + assemble the app (skip the DMG — this is the *portable* bundle).
export KETRA_SKIP_DMG=1
bash "$ROOT/packaging/macos/make-app.sh"
unset KETRA_SKIP_DMG

APP="$DIST/macos/KetraCopy.app"
if [ ! -d "$APP" ]; then
    echo "ERROR: $APP was not produced." >&2
    exit 1
fi

# ---- portable markers ----------------------------------------------------------
# Next to the executable (what settings.rs actually checks):
: > "$APP/Contents/MacOS/portable.flag"
# Also in Resources, per the product spec / for humans poking around:
: > "$APP/Contents/Resources/portable.flag"

cat > "$APP/Contents/Resources/README-portable.txt" <<EOF
KetraCopy $VERSION — portable (macOS)

Portable mode: an empty "portable.flag" sits next to the executable in
Contents/MacOS (that is where crates/ketra-core/src/settings.rs looks), so
ketra keeps its settings in Contents/MacOS/ketra.json instead of
~/Library/Application Support/KetraCopy/.

First run without a developer signature (Gatekeeper):

    xattr -cr /path/to/KetraCopy.app

or right-click the app once and choose "Open". Fully unsigned builds may be
blocked on recent macOS — build locally with packaging/macos/make-app.sh.

Make the CLI available in Terminal:

    sudo mkdir -p /usr/local/bin
    sudo ln -sf /Applications/KetraCopy.app/Contents/MacOS/ketra /usr/local/bin/ketra
EOF

# ---- zip (ditto keeps mac metadata; fallback to plain zip) ----------------------
ZIP="$DIST/KetraCopy-$VERSION-portable-macos.zip"
rm -f "$ZIP"
if command -v ditto >/dev/null 2>&1; then
    ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"
else
    (cd "$DIST/macos" && zip -qry "$ZIP" KetraCopy.app)
fi

echo ""
echo "App bundle: $APP"
echo "Zip:        $ZIP"
echo "Gatekeeper note: recipients run 'xattr -cr KetraCopy.app' on first use,"
echo "                 or you sign/notarize (see packaging/README.md)."
exit 0
