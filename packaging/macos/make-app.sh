#!/usr/bin/env bash
# =============================================================================
# packaging/macos/make-app.sh — build KetraCopy.app (and a DMG) for macOS.
#
# Usage:
#   ./make-app.sh                  # build + assemble + (optional) sign + DMG
#   ./make-app.sh --assemble-only  # only assemble the .app layout (any OS;
#                                  # useful on Linux/CI to inspect the bundle)
#
# Environment:
#   CODESIGN_IDENTITY   codesign identity, e.g. "Developer ID Application: ..."
#                       (optional, macOS only; "-"" does an ad-hoc signature)
#   KETRA_SKIP_DMG=1    skip the DMG step (used by portable/portable_macos.sh)
#
# Artifacts:
#   dist/macos/KetraCopy.app
#   dist/KetraCopy-1.2.0.dmg            (macOS only)
#
# Requires macOS with Xcode Command Line Tools for the build/sign/DMG steps.
# Fails gracefully (with a clear message) when run on Linux/Windows.
# =============================================================================
set -euo pipefail

VERSION="1.2.0"
APP_ID="io.github.ketracopy.KetraCopy"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BUILD_DIR="$ROOT/dist/macos"
APP_DIR="$BUILD_DIR/KetraCopy.app"

ASSEMBLE_ONLY=0
if [ "${1:-}" = "--assemble-only" ]; then
    ASSEMBLE_ONLY=1
fi

OS="$(uname -s)"
if [ "$OS" != "Darwin" ] && [ "$ASSEMBLE_ONLY" -eq 0 ]; then
    echo "ERROR: make-app.sh needs macOS to build, sign and image the bundle" >&2
    echo "       (uname -s = '$OS')." >&2
    echo "Hint:  run with --assemble-only to inspect the bundle layout on this OS," >&2
    echo "       or use portable/portable_linux.sh / portable_windows.cmd instead." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 1. Build (CLI required; GUI best-effort — the ketra-gui crate is added in
#    parallel and may not be wired into the workspace yet).
# ---------------------------------------------------------------------------
if [ "$ASSEMBLE_ONLY" -eq 0 ]; then
    echo "==> cargo build --release -p ketra-cli"
    (cd "$ROOT" && cargo build --release -p ketra-cli)
    if [ -f "$ROOT/crates/ketra-gui/Cargo.toml" ]; then
        echo "==> cargo build --release -p ketra-gui"
        if ! (cd "$ROOT" && cargo build --release -p ketra-gui); then
            echo "WARNING: ketra-gui build failed; bundling the CLI only." >&2
        fi
    else
        echo "NOTE: crates/ketra-gui does not exist yet; bundling the CLI only."
    fi
fi

KETRA="$ROOT/target/release/ketra"
KETRA_GUI="$ROOT/target/release/ketracopy-gui"
if [ ! -x "$KETRA" ]; then
    echo "ERROR: $KETRA not found — run 'cargo build --release -p ketra-cli' first." >&2
    exit 1
fi
HAS_GUI=0
if [ -x "$KETRA_GUI" ]; then
    HAS_GUI=1
fi

# ---------------------------------------------------------------------------
# 2. Assemble KetraCopy.app/Contents/{MacOS,Resources}
# ---------------------------------------------------------------------------
echo "==> assembling $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

if [ "$HAS_GUI" -eq 1 ]; then
    cp "$KETRA_GUI" "$APP_DIR/Contents/MacOS/ketra-gui"
    chmod 755 "$APP_DIR/Contents/MacOS/ketra-gui"
    cp "$HERE/Info.plist" "$APP_DIR/Contents/Info.plist"
else
    # Bundle the CLI as the main executable so the app still launches;
    # Info.plist's CFBundleExecutable is rewritten accordingly.
    echo "WARNING: ketra-gui not built — rewriting CFBundleExecutable to 'ketra'." >&2
    sed 's|<string>ketra-gui</string>|<string>ketra</string>|' "$HERE/Info.plist" \
        > "$APP_DIR/Contents/Info.plist"
fi

cp "$KETRA" "$APP_DIR/Contents/MacOS/ketra"
chmod 755 "$APP_DIR/Contents/MacOS/ketra"
cp "$ROOT/LICENSE" "$APP_DIR/Contents/Resources/LICENSE"

# CLI-symlink note (there is no /usr/local/bin link inside a .app):
cat > "$APP_DIR/Contents/Resources/README-CLI.txt" <<EOF
KetraCopy $VERSION — command line

To make the 'ketra' CLI available in Terminal (after copying the app to
/Applications):

    sudo mkdir -p /usr/local/bin
    sudo ln -sf /Applications/KetraCopy.app/Contents/MacOS/ketra /usr/local/bin/ketra

The CLI binary also lives next to the GUI binary inside the bundle, so the
Finder Quick Action (see packaging/macos/make-quick-action.sh) can use it
directly.
EOF

if [ "$OS" = "Darwin" ]; then
    plutil -lint "$APP_DIR/Contents/Info.plist"
fi

# ---------------------------------------------------------------------------
# 3. Optional code signature (macOS only, guarded by CODESIGN_IDENTITY)
# ---------------------------------------------------------------------------
if [ "$OS" = "Darwin" ] && [ -n "${CODESIGN_IDENTITY:-}" ]; then
    echo "==> codesign (identity: $CODESIGN_IDENTITY)"
    codesign --force --deep --options runtime --timestamp \
        --sign "$CODESIGN_IDENTITY" "$APP_DIR"
    codesign --verify --deep --strict "$APP_DIR"
else
    echo "NOTE: unsigned (set CODESIGN_IDENTITY to sign; see packaging/README.md)."
fi

# ---------------------------------------------------------------------------
# 4. DMG (macOS only, skipped with --assemble-only or KETRA_SKIP_DMG=1)
# ---------------------------------------------------------------------------
if [ "$OS" = "Darwin" ] && [ "$ASSEMBLE_ONLY" -eq 0 ] && [ "${KETRA_SKIP_DMG:-0}" != "1" ]; then
    mkdir -p "$ROOT/dist"
    STAGE="$BUILD_DIR/dmg-stage"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp -R "$APP_DIR" "$STAGE/KetraCopy.app"
    ln -s /Applications "$STAGE/Applications"
    echo "==> hdiutil create dist/KetraCopy-$VERSION.dmg"
    hdiutil create -volname "KetraCopy $VERSION" \
        -srcfolder "$STAGE" -ov -format UDZO \
        "$ROOT/dist/KetraCopy-$VERSION.dmg"
    rm -rf "$STAGE"
fi

echo ""
echo "App bundle: $APP_DIR"
if [ -f "$ROOT/dist/KetraCopy-$VERSION.dmg" ]; then
    echo "DMG:        $ROOT/dist/KetraCopy-$VERSION.dmg"
fi
exit 0
