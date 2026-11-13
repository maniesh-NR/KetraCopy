#!/usr/bin/env bash
# =============================================================================
# scripts/package-dist.sh — assemble every release artifact in dist/.
#
# Prerequisites:
#   * Linux binaries:   target/release/{ketra,ketracopy-gui}      (cargo build --release)
#   * Windows binaries: target/{x86_64,i686}-pc-windows-gnu/release/*.exe
#                       (cargo zigbuild --release --target <triple>)
#   * macOS binaries:   target/{aarch64,x86_64}-apple-darwin/release/ketra
#                       (cargo zigbuild --release -p ketra-cli --target <triple>)
#   * deb/portable:     built via portable/portable_linux.sh and
#                       packaging/linux/deb/build-deb.sh (invoked here)
#
# Produces in dist/:
#   linux-x86_64/ + ketracopy-<v>-linux-x86_64.tar.gz
#   ketracopy-<v>-portable-linux-x86_64.tar.gz
#   ketracopy_<v>_amd64.deb
#   ketracopy-<v>-windows-x86_64.zip / -i686.zip   (CLI + GUI)
#   ketracopy-<v>-macos-aarch64.tar.gz / -x86_64.tar.gz  (CLI; GUI needs Xcode)
#   ketracopy-<v>-src.tar.gz / -src.zip
#   SHA256SUMS.txt
# =============================================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
VERSION="1.2.0"
DIST="$ROOT/dist"
DOCS=(README.md LICENSE ATTRIBUTIONS.md SHELL-INTEGRATION.md)

cd "$ROOT"
mkdir -p "$DIST"

echo "==> 1. portable bundle + deb (Linux)"
bash "$ROOT/portable/portable_linux.sh"
bash "$ROOT/packaging/linux/deb/build-deb.sh"

echo "==> 2. linux-x86_64/ dir + linux tar.gz"
rm -rf "$DIST/linux-x86_64"
mkdir -p "$DIST/linux-x86_64"
install -m 755 target/release/ketra          "$DIST/linux-x86_64/ketra"
install -m 755 target/release/ketracopy-gui  "$DIST/linux-x86_64/ketracopy-gui"
cp "$DIST/README.txt" "$DIST/linux-x86_64/README.txt" 2>/dev/null || true
STAGE="$DIST/ketracopy-${VERSION}-linux-x86_64"
rm -rf "$STAGE"; mkdir -p "$STAGE"
install -m 755 target/release/ketra          "$STAGE/ketra"
install -m 755 target/release/ketracopy-gui  "$STAGE/ketracopy-gui"
for f in "${DOCS[@]}"; do cp "$ROOT/$f" "$STAGE/"; done
tar -czf "$DIST/ketracopy-${VERSION}-linux-x86_64.tar.gz" -C "$DIST" "ketracopy-${VERSION}-linux-x86_64"
rm -rf "$STAGE"

echo "==> 3. windows zips (x86_64 + i686, CLI + GUI)"
for arch in x86_64 i686; do
    triple="${arch}-pc-windows-gnu"
    exe_dir="target/${triple}/release"
    [ -x "$exe_dir/ketra.exe" ] || { echo "   skip ${arch} (no ketra.exe)"; continue; }
    stage="$DIST/ketracopy-${VERSION}-windows-${arch}"
    rm -rf "$stage"; mkdir -p "$stage"
    install -m 755 "$exe_dir/ketra.exe" "$stage/ketra.exe"
    [ -x "$exe_dir/ketracopy-gui.exe" ] && install -m 755 "$exe_dir/ketracopy-gui.exe" "$stage/ketracopy-gui.exe" || true
    for f in "${DOCS[@]}"; do cp "$ROOT/$f" "$stage/"; done
    (cd "$DIST" && python3 -m zipfile -c "ketracopy-${VERSION}-windows-${arch}.zip" "ketracopy-${VERSION}-windows-${arch}")
    rm -rf "$stage"
done

echo "==> 4. macos tar.gz (CLI-only; GUI requires an Apple SDK/Xcode build)"
cat > "$DIST/README-macos.txt" <<'EOF'
KetraCopy for macOS — command-line build
========================================

This archive contains the `ketra` CLI, cross-compiled for Apple Silicon
(aarch64) or Intel (x86_64) Macs. It includes the full engine: copy/move
with inline & manifest verification, resume journals, sparse handling,
LAN transfer (`ketra peers | send | receive`) and clipboard integration
(`ketra clip put|get|paste` via AppleScript).

The Material-3 GUI (`ketracopy-gui`) is NOT included: it links AppKit /
OpenGL frameworks and must be built on a Mac with Xcode installed:

    git clone <repo> && cd ketracopy
    cargo build --release        # then run target/release/ketracopy-gui

First run on macOS may require right-click > Open (unsigned binary).
EOF
for arch in aarch64 x86_64; do
    exe="target/${arch}-apple-darwin/release/ketra"
    [ -f "$exe" ] || { echo "   skip ${arch} (no binary)"; continue; }
    stage="$DIST/ketracopy-${VERSION}-macos-${arch}"
    rm -rf "$stage"; mkdir -p "$stage"
    install -m 755 "$exe" "$stage/ketra"
    cp "$DIST/README-macos.txt" "$stage/README-macos.txt"
    for f in LICENSE ATTRIBUTIONS.md SHELL-INTEGRATION.md; do cp "$ROOT/$f" "$stage/"; done
    tar -czf "$DIST/ketracopy-${VERSION}-macos-${arch}.tar.gz" -C "$DIST" "ketracopy-${VERSION}-macos-${arch}"
    rm -rf "$stage"
done
rm -f "$DIST/README-macos.txt"

echo "==> 5. source archives"
src_stage="$DIST/ketracopy-${VERSION}-src"
rm -rf "$src_stage"; mkdir -p "$src_stage"
tar -cf - \
    --exclude='target' --exclude='dist' --exclude='KetraCopy-portable' \
    --exclude='.git' --exclude='attic' --exclude='android/build' \
    --exclude='*.deb' --exclude='*.tar.gz' --exclude='*.zip' \
    -C "$ROOT" . | tar -xf - -C "$src_stage"
tar -czf "$DIST/ketracopy-${VERSION}-src.tar.gz" -C "$DIST" "ketracopy-${VERSION}-src"
(cd "$DIST" && python3 -m zipfile -c "ketracopy-${VERSION}-src.zip" "ketracopy-${VERSION}-src")
rm -rf "$src_stage"

echo "==> 6. exec bits + SHA256SUMS.txt"
find "$DIST" -type f \( -name 'ketra' -o -name 'ketracopy-gui' \) -exec chmod 755 {} \; 2>/dev/null || true
(cd "$DIST" && rm -f SHA256SUMS.txt SHA256SUMS.txt.tmp \
    && find . -maxdepth 1 -type f -not -name 'SHA256SUMS.txt' -exec sha256sum {} + \
    | sed 's|\./||' | sort -k2 > SHA256SUMS.txt)

echo "==> done. dist/ contents:"
ls -la "$DIST"
