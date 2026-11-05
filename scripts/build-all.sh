#!/usr/bin/env bash
# =============================================================================
# scripts/build-all.sh — one-shot release orchestrator.
#
#   1. cargo build --release for the whole workspace
#   2. build the PORTABLE bundle for the current OS
#      (Linux -> portable/portable_linux.sh, macOS -> portable_macos.sh,
#       Windows/Git-Bash -> portable_windows.cmd via cmd.exe)
#   3. optional extras:  --deb   also build the Debian package (Linux only)
#   4. print every artifact path that exists
#
# Other targets (not in this default flow) are documented in
# packaging/README.md: Inno Setup installer, macOS .app/DMG, Flatpak,
# file-manager integrations.
# =============================================================================
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
VERSION="1.1.0"

WANT_DEB=0
for arg in "$@"; do
    case "$arg" in
        --deb) WANT_DEB=1 ;;
        -h|--help)
            sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option: $arg (supported: --deb, --help)" >&2
            exit 2
            ;;
    esac
done

echo "==> cargo build --release (workspace)"
(cd "$ROOT" && cargo build --release)

OS="$(uname -s)"
case "$OS" in
    Linux)
        bash "$ROOT/portable/portable_linux.sh"
        if [ "$WANT_DEB" -eq 1 ]; then
            echo "==> building Debian package"
            bash "$ROOT/packaging/linux/deb/build-deb.sh"
        fi
        ;;
    Darwin)
        bash "$ROOT/portable/portable_macos.sh"
        ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
        echo "==> running portable_windows.cmd via cmd.exe"
        cmd //c "$ROOT\\portable\\portable_windows.cmd"
        ;;
    *)
        echo "ERROR: unsupported OS '$OS' (expected Linux, Darwin or Windows/MSYS)." >&2
        exit 1
        ;;
esac

echo ""
echo "==> artifacts"
ARTIFACTS="
$ROOT/dist/KetraCopy-$VERSION-portable-windows.zip
$ROOT/dist/ketracopy-$VERSION-portable-linux-$(uname -m 2>/dev/null || echo x86_64).tar.gz
$ROOT/dist/KetraCopy-$VERSION-portable-macos.zip
$ROOT/dist/KetraCopy-$VERSION.dmg
$ROOT/dist/ketracopy_${VERSION}_amd64.deb
$ROOT/KetraCopy-portable
$ROOT/dist/macos/KetraCopy.app
"
FOUND=0
while IFS= read -r f; do
    if [ -e "$f" ]; then
        echo "  $f"
        FOUND=1
    fi
done <<EOF
$ARTIFACTS
EOF
if [ "$FOUND" -eq 0 ]; then
    echo "  (none found — check the build logs above)"
fi

echo ""
echo "Next steps (manual targets):"
echo "  Windows installer : ISCC packaging/windows/ketracopy.iss"
echo "  macOS .app + DMG  : packaging/macos/make-app.sh"
echo "  Flatpak           : flatpak-builder --user --install dist/flatpak-build \\"
echo "                      packaging/flatpak/io.github.ketracopy.KetraCopy.yml"
echo "  FM integrations   : integrations/linux/install.sh"
exit 0
