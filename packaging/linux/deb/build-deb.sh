#!/usr/bin/env bash
# =============================================================================
# packaging/linux/deb/build-deb.sh — assemble a Debian .deb for KetraCopy.
#
# Usage:
#   ./build-deb.sh                 # requires target/release/ketra (cargo build --release -p ketra-cli)
#
# Layout produced:
#   /usr/bin/ketra                                            CLI (required)
#   /usr/bin/ketracopy-gui                                        GUI (if built)
#   /usr/share/applications/io.github.ketracopy.KetraCopy.desktop
#   /usr/share/metainfo/io.github.ketracopy.KetraCopy.metainfo.xml
#   /usr/share/icons/hicolor/scalable/apps/io.github.ketracopy.KetraCopy.svg
#   /usr/share/doc/ketracopy/copyright                        (machine-readable, MIT)
#
# Output:  dist/ketracopy_1.2.0_amd64.deb   (built with dpkg-deb --root-owner-group,
#           so files are owned root:root even when the script runs unprivileged)
#
# NOTE: Depends is intentionally minimal (libc6). If ketracopy-gui links a GUI
#       toolkit, add its runtime libs to Depends here once the GUI crate's
#       stack is final. Version is kept at 1.2.0 everywhere.
# =============================================================================
set -euo pipefail

VERSION="1.2.0"
DEB_ARCH="amd64"
PACKAGE="ketracopy"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
DIST="$ROOT/dist"
STAGING="$(mktemp -d /tmp/ketracopy-deb.XXXXXX)"
trap 'rm -rf "$STAGING"' EXIT

KETRA="$ROOT/target/release/ketra"
KETRA_GUI="$ROOT/target/release/ketracopy-gui"

if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "ERROR: dpkg-deb not found — this script needs a Debian/Ubuntu host (or dpkg-dev)." >&2
    exit 1
fi
if [ ! -x "$KETRA" ]; then
    echo "ERROR: $KETRA not found — run 'cargo build --release -p ketra-cli' first." >&2
    exit 1
fi

# ---- directory skeleton -----------------------------------------------------
mkdir -p \
    "$STAGING/DEBIAN" \
    "$STAGING/usr/bin" \
    "$STAGING/usr/share/applications" \
    "$STAGING/usr/share/metainfo" \
    "$STAGING/usr/share/icons/hicolor/scalable/apps" \
    "$STAGING/usr/share/doc/$PACKAGE"

install -m 755 "$KETRA" "$STAGING/usr/bin/ketra"
if [ -x "$KETRA_GUI" ]; then
    install -m 755 "$KETRA_GUI" "$STAGING/usr/bin/ketracopy-gui"
else
    echo "NOTE: ketracopy-gui not built — shipping the CLI only."
fi

install -m 644 "$ROOT/packaging/linux/io.github.ketracopy.KetraCopy.desktop" \
    "$STAGING/usr/share/applications/io.github.ketracopy.KetraCopy.desktop"
install -m 644 "$ROOT/packaging/linux/io.github.ketracopy.KetraCopy.metainfo.xml" \
    "$STAGING/usr/share/metainfo/io.github.ketracopy.KetraCopy.metainfo.xml"
install -m 644 "$ROOT/packaging/linux/icon.svg" \
    "$STAGING/usr/share/icons/hicolor/scalable/apps/io.github.ketracopy.KetraCopy.svg"
install -m 644 "$ROOT/LICENSE" "$STAGING/usr/share/doc/$PACKAGE/LICENSE"

# ---- DEBIAN/control ---------------------------------------------------------
cat > "$STAGING/DEBIAN/control" <<EOF
Package: $PACKAGE
Version: $VERSION
Section: utils
Priority: optional
Architecture: $DEB_ARCH
Depends: libc6 (>= 2.17)
Maintainer: KetraCopy contributors <ketracopy@example.invalid>
Homepage: https://example.invalid/ketracopy
Description: Verified file copy/move tool with pause, resume and SHA-256 manifests
 KetraCopy (ketra) copies and moves files with inline or full SHA-256
 verification, pause/resume journals, speed limits, conflict policies
 and filter support. Includes the ketra CLI and (when built) the
 ketracopy-gui desktop application.
EOF

# Installed-Size is optional but keeps lintian quiet (KiB, without DEBIAN/).
INSTALLED_SIZE="$(du -sk --exclude=DEBIAN "$STAGING" | cut -f1)"
printf 'Installed-Size: %s\n' "$INSTALLED_SIZE" >> "$STAGING/DEBIAN/control"

# ---- DEBIAN/copyright (Debian machine-readable format, MIT) -----------------
cat > "$STAGING/usr/share/doc/$PACKAGE/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: KetraCopy
Source: https://example.invalid/ketracopy

Files: *
Copyright: 2026 KetraCopy contributors
License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
EOF
chmod 644 "$STAGING/usr/share/doc/$PACKAGE/copyright"

# ---- build ------------------------------------------------------------------
mkdir -p "$DIST"

# Normalize modes for the data tarball (dirs 755, files 644, executables 755)
# and the package root itself (mktemp -d creates it as 0700).
chmod -R u+rwX,go+rX,go-w "$STAGING"
chmod 755 "$STAGING"

OUT="$DIST/${PACKAGE}_${VERSION}_${DEB_ARCH}.deb"
rm -f "$OUT"
dpkg-deb --root-owner-group --build "$STAGING" "$OUT"

echo ""
echo "Built: $OUT"
dpkg-deb --info "$OUT" | sed -n '1,12p'
exit 0
