# KetraCopy packaging & integrations

Targets from the product spec: **Windows 7+ (x86 & x64)**, **macOS 10.15+
(x86_64)**, **Linux Mint (Cinnamon/MATE/Xfce) + Debian**, **Flatpak**, and
**portable builds of all of them**. Version is **1.2.0** everywhere and the
app-id/metadata id is **`io.github.ketracopy.KetraCopy`** consistently.

Binary names (do not rename): `ketra` (CLI, `crates/ketra-cli`) and
`ketra-gui` (GUI, `crates/ketra-gui` — added in parallel; every script here
checks for its existence and degrades gracefully when only the CLI exists).

## Build matrix

| Target | Script / file | Artifact | Prerequisites |
|---|---|---|---|
| Windows installer (x86/x64, per-user + per-machine) | `packaging/windows/ketracopy.iss` | `dist/windows/KetraCopy-1.2.0-setup.exe` | Inno Setup 6 (`ISCC.exe`), `target/release/ketra.exe` built for Windows |
| Windows Explorer context menu (standalone) | `integrations/windows/context-menu-install.ps1` (+`-uninstall.ps1`, `context-menu.ps1`, `README.md`) | registry verbs | Windows PowerShell 2.0+ (stock on Win7); no elevation for `-Scope user` |
| Windows portable | `portable/portable_windows.cmd` | `KetraCopy-portable\` + `dist/KetraCopy-1.2.0-portable-windows.zip` | Rust (MSVC or mingw), PowerShell 5+ for the zip step (folder itself works without it) |
| macOS app + DMG | `packaging/macos/make-app.sh` | `dist/macos/KetraCopy.app`, `dist/KetraCopy-1.2.0.dmg` | macOS + Xcode CLT (`hdiutil`); `CODESIGN_IDENTITY` to sign |
| macOS Finder Quick Action | `packaging/macos/make-quick-action.sh` (doc: `packaging/macos/finder-quick-action.md`) | `~/Library/Services/KetraCopy-{copy,move}.workflow` | macOS (Automator runtime, `plutil`) |
| macOS portable | `portable/portable_macos.sh` | `dist/KetraCopy-1.2.0-portable-macos.zip` | macOS (calls `make-app.sh` with `KETRA_SKIP_DMG=1`) |
| Debian / Mint .deb | `packaging/linux/deb/build-deb.sh` | `dist/ketracopy_1.2.0_amd64.deb` | `dpkg-deb` (dpkg-dev), release build; desktop/metainfo/icon come from `packaging/linux/` |
| Flatpak | `packaging/flatpak/io.github.ketracopy.KetraCopy.yml` | `io.github.ketracopy.KetraCopy` app | `flatpak-builder`, Freedesktop **24.08** runtime/SDK + `org.freedesktop.Sdk.Extension.rust-stable//24.08` |
| Linux portable | `portable/portable_linux.sh` | `KetraCopy-portable/` + `dist/ketracopy-1.2.0-portable-linux-<arch>.tar.gz` | Rust (build on the oldest supported glibc, see below) |
| Linux file-manager actions (Nemo/Caja/Thunar/Nautilus/Dolphin) | `integrations/linux/install.sh` + `ketra-pick.sh` | per-FM actions in `~/.local/...` | `zenity` **or** `kdialog` **or** `yad` for the destination picker; `python3` for the Thunar uca.xml merge |
| Everything (build + current-OS portable + artifact list) | `scripts/build-all.sh` | see script output | Rust; extra: `dpkg-deb` when `--deb` is passed |

Portable-mode mechanics: an empty **`portable.flag`** next to the executable
makes the engine keep its settings in **`ketra.json`** next to the binary
instead of the per-user config dir — confirmed in
`crates/ketra-core/src/settings.rs`. All portable bundles drop that file.

## Windows notes (Win7)

* **Toolchain:** MSVC is the default (`cargo build --release`). For Win7,
  prefer a toolset that still targets it (e.g. VS2019 v142); otherwise use
  mingw-w64: `rustup target add x86_64-pc-windows-gnu` (and/or
  `i686-pc-windows-gnu`), then
  `cargo build --release --target x86_64-pc-windows-gnu` and point the
  `.iss` [Files] entries at `target\x86_64-pc-windows-gnu\release\`.
  `RUSTFLAGS=-C target-feature=+crt-static` avoids needing the VC
  redist on boxen without it.
* **Installer:** Inno Setup 6; `MinVersion=6.1`; `ArchitecturesInstallIn64BitMode=x64`
  (Inno 6.3+ accepts `x64compatible`); `PrivilegesRequiredOverridesAllowed`
  gives the per-user/per-machine choice in one binary.
* **Context menu:** registry verbs → PowerShell helper → folder picker →
  `ketra copy|move|test "%1" -d "<picked>"`. All helpers are PS **2.0**
  safe (no `$PSScriptRoot`, no ternary, no `&&`). Multi-select fires the
  verb once per item (legacy behaviour) — acceptable; batch jobs via GUI/CLI.
* **Zip step:** `Compress-Archive` needs PS 5+ (Win10). On Win7 the folder
  `KetraCopy-portable\` is itself the portable app.

## macOS notes (codesign / notarization outline)

1. **Local/ad-hoc:** `CODESIGN_IDENTITY="-" ./packaging/macos/make-app.sh`
   (runs an ad-hoc signature; enough for personal use, still Gatekeeper-prompted).
2. **Developer ID:** 
   `CODESIGN_IDENTITY="Developer ID Application: <name> (<teamid>)" ./packaging/macos/make-app.sh`
   — the script signs with `--options runtime --timestamp`.
3. **Notarize the DMG:**
   ```console
   $ xcrun notarytool submit dist/KetraCopy-1.2.0.dmg \
         --keychain-profile AC_PASSWORD --wait
   $ xcrun stapler staple dist/KetraCopy-1.2.0.dmg
   ```
4. Unsigned/portable builds: recipients run `xattr -cr KetraCopy.app`
   (documented in the bundled README-portable.txt and printed by the scripts).
5. `LSMinimumSystemVersion` is 10.15; build with a macOS SDK that still
   targets it (Xcode 12/13 era) for Catalina compatibility.

## Linux notes

* **glibc:** binaries link the build host's glibc. For the widest reach
  (Debian 10 / Mint 19 era), build in the oldest container you intend to
  support, or ship a `--target x86_64-unknown-linux-musl` build in the
  portable tarball.
* **.deb:** `dpkg-deb --root-owner-group` (no root needed); `Depends:
  libc6 (>= 2.17)` on purpose — extend once `ketra-gui`'s toolkit is final.
* **Flatpak:** the manifest already contains
  `--talk-name=org.freedesktop.Flatpak` so host file-manager actions can run
  `flatpak-spawn --host ketra ...` (the integration wrapper does this
  automatically when `FLATPAK_ID`/`/.flatpak-info` is present). Build:
  `flatpak-builder --user --install --force-clean dist/flatpak-build
  packaging/flatpak/io.github.ketracopy.KetraCopy.yml`.
* **Desktop/metainfo/icon:** `packaging/linux/io.github.ketracopy.KetraCopy.desktop`
  (Exec `ketra-gui %U`, `TryExec` hides the launcher if the GUI is absent),
  `packaging/linux/io.github.ketracopy.KetraCopy.metainfo.xml` (AppStream,
  `provides/binary: ketra`, release 1.2.0) and `packaging/linux/icon.svg`
  (MD3-purple placeholder; replace with a real .icns/.png/.ico asset set when
  design lands — no binary icon asset exists yet).

## Known simplifications / honest caveats

* Thunar `uca.xml` is **merged** with python3 (backup `uca.xml.bak`); without
  python3 the installer prints manual steps instead of editing the file.
* The macOS Quick Action `document.wflow` is generated and only
  plist-validated; a manual Automator recipe is in
  `finder-quick-action.md` as fallback.
* Windows context-menu multi-select opens one picker per selected item
  (legacy registry-verb behaviour, kept for Win7/PS2.0 compatibility).
* `icon.svg` is a placeholder; no `.icns`, `.png`, or `.ico` assets exist yet,
  so `Info.plist` declares no `CFBundleIconFile` and the `.iss` sets no
  `SetupIconFile`.
