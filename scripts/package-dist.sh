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
VERSION="1.2.1"
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
cp "$ROOT/packaging/dist-README.txt" "$DIST/linux-x86_64/README.txt"
STAGE="$DIST/ketracopy-${VERSION}-linux-x86_64"
rm -rf "$STAGE"; mkdir -p "$STAGE"
install -m 755 target/release/ketra          "$STAGE/ketra"
install -m 755 target/release/ketracopy-gui  "$STAGE/ketracopy-gui"
for f in "${DOCS[@]}"; do cp "$ROOT/$f" "$STAGE/"; done
tar -czf "$DIST/ketracopy-${VERSION}-linux-x86_64.tar.gz" -C "$DIST" "ketracopy-${VERSION}-linux-x86_64"
rm -rf "$STAGE"

echo "==> 3./4. windows zips + macos tar.gz — NOT built here (single-producer rule)"
cat <<'NOTE'
Single-producer rule (Gate 1 / Gate 2): every release asset has exactly
ONE producer. Windows (.zip, msvc x86_64 + i686, CLI + GUI) and macOS
(.tar.gz, CLI, x86_64 + aarch64) release binaries are built NATIVELY by
the CI release workflow (.github/workflows/release.yml). Packaging them
here from any cross binaries found in target/ is deliberately disabled —
same-named assets would collide with the CI outputs and break the
hash-baselined release page (harness/state/artifact-hashes.md).
NOTE

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
