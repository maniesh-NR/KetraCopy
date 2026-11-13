# Building KetraCopy

## Prerequisites

- Rust 1.75 or newer (`rust-version = "1.75"` in `crates/ketra-core/Cargo.toml`; workspace edition 2021). The development sandbox used rustc 1.100.0 on Linux x86-64.
- `ketra-core` and `ketra-cli` have **zero external dependencies** — building the CLI (`cargo build --release -p ketra-cli`) needs no crates.io access; the root `Cargo.lock` additionally pins the GUI crates' `egui`/`eframe` tree.
- The GUI crates are workspace members and depend on crates.io: `ketra-md3` (Material Design 3 design system; dependency `egui =0.28.1`) and `ketra-gui` (binary `ketracopy-gui`; `eframe =0.28.1` with the `glow` OpenGL renderer). Both compile (`cargo check --workspace`); `ketra-gui` also release-builds and links (`cargo build --release -p ketra-gui`). Display behavior is exercised under Xvfb by the interop suite.
- Tests: `bash` and `python3` for the integration suite (`scripts/test_integration.sh`); `stat(1)` with GNU options is used for the sparse-file check, so the suite is POSIX-only.

## Linux

```sh
cargo build --release -p ketra-cli     # produces target/release/ketra
cargo test --workspace                 # 132 unit tests
scripts/test_integration.sh            # 41 end-to-end checks (builds the release binary if needed)
scripts/test_interop.sh                # feature-interoperability suite (needs Xvfb)
```

No system libraries are required by the CLI/core: the platform layer is hand-maintained FFI against libc (`copy_file_range`, `statvfs`, `lseek`, `ioctl`). Only a C link environment (any glibc/musl toolchain) is needed. The same binary works on Mint (Cinnamon/MATE/Xfce), Debian, and other distributions; the engine never touches the display.

The GUI (`ketracopy-gui`) needs OpenGL and X11 development packages at build time (`libgl1-mesa-dev`, `libx11-dev` on Debian-family); the CLI/core need neither.

### GUI (Linux)

```sh
cargo build --release -p ketra-gui    # builds ketra-md3 + ketracopy-gui (egui/eframe from crates.io)
```

The eframe feature set is `glow` (plain OpenGL 2.1+/ES) with `x11` and `wayland`, so no GTK development headers are needed. By the crate's own design notes this is the renderer path intended to work unchanged on Win7 (WGL/OpenGL), macOS 10.15+ and Linux.

## Windows

Two supported routes; both produce the same dependency-free `ketra.exe`.

### Modern Windows (10/11) — MSVC

```sh
rustup target add x86_64-pc-windows-msvc   # default on Windows hosts
cargo build --release -p ketra-cli
```

For 32-bit builds add the `i686-pc-windows-msvc` target (the core crate stubs `statvfs` on 32-bit and uses `isize` handles, so the code is 32-bit-clean). Requires the Visual Studio Build Tools (C workload). Note: since Rust 1.78 the stable MSVC toolchain no longer runs on Windows 7.

### Windows 7 route

The engine has no special ABI needs (no CRT dependencies beyond the default, all FFI is `extern "system"`/`extern "C"` against kernel32/libc), so two options work:

1. **GNU toolchain with mingw-w64** (works on stable):

   ```sh
   rustup target add x86_64-pc-windows-gnu    # or i686-pc-windows-gnu
   cargo build --release -p ketra-cli --target x86_64-pc-windows-gnu
   ```

   Build/link with an mingw-w64 toolchain (on Debian-family: `apt install mingw-w64`, plus a `[target.x86_64-pc-windows-gnu]` linker entry such as `x86_64-w64-mingw32-gcc` in `.cargo/config.toml`).

2. **Tier-3 `x86_64-win7-windows-msvc`** (nightly):

   ```sh
   rustup toolchain install nightly
   cargo +nightly build -Z build-std=std,panic_abort \
       --target x86_64-win7-windows-msvc -p ketra-cli
   ```

   This target exists specifically to keep Windows 7 compatibility on the MSVC side; being tier-3, it needs `-Z build-std` and is not covered by Rust CI guarantees.

For a GUI shell on Windows 7, the renderer question is already answered by the dependency choice: `eframe`'s `glow` feature renders through plain OpenGL (2.1+/ES), which the `crates/ketra-gui/Cargo.toml` notes identify as the path that works on Win7, macOS 10.15+ and Linux alike. The GUI compiles and release-builds (verified on Linux); Win7 GUI operation is untested.

## macOS 10.15+

```sh
rustup target add x86_64-apple-darwin
cargo build --release -p ketra-cli
```

- Universal binary: build both `x86_64-apple-darwin` and `aarch64-apple-darwin`, then `lipo -create -output ketra target/x86_64-apple-darwin/release/ketra target/aarch64-apple-darwin/release/ketra`.
- Ad-hoc codesign for local distribution: `codesign --force --sign - target/release/ketra`.
- Caveat, verified by reading the code: the macOS capability layer (`crates/ketra-core/src/platform/macos.rs`) implements `clonefile`, `F_NOCACHE`, `F_RDADVISE` and `F_PREALLOCATE`, but `platform/mod.rs` currently dispatches preallocation and readahead advice through `platform/unix.rs`, whose `fallocate`/`posix_fadvise` implementations are Linux-oriented. macOS binaries from this snapshot have never been compiled or executed; expect to wire `macos.rs` into the dispatchers before shipping.

## Cross-compiling from Linux

Windows targets with mingw-w64 and cargo:

```sh
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu i686-pc-windows-gnu
cargo build --release -p ketra-cli --target x86_64-pc-windows-gnu
cargo build --release -p ketra-cli --target i686-pc-windows-gnu
```

macOS cross-compilation from Linux requires an external SDK setup (e.g. osxcross) and is not covered here; none of it has been exercised for this repository.

## Flatpak

`packaging/flatpak/io.github.ketracopy.KetraCopy.yml` ships a manifest (runtime `org.freedesktop.Platform//24.08`). The standard workflow applies:

```sh
flatpak install flathub org.freedesktop.Platform//24.08 org.freedesktop.Sdk//24.08
flatpak-builder --force-clean build-dir packaging/flatpak/com.ketracopy.KetraCopy.json
flatpak-builder --repo=repo --export-only build-dir
flatpak build-bundle repo ketracopy.flatpak com.ketracopy.KetraCopy
```

Because the CLI/core have no dependencies, the Flatpak build needs only the Freedesktop runtime and the Rust SDK extension.

## Running the tests

```sh
cargo test -p ketra-core --lib      # unit tests (hash vectors, aimd, throttle,
                                    # controller, journal, manifest, resume, glob)
scripts/test_integration.sh         # end-to-end: copy/verify/manifest/bit-rot,
                                    # sparse, move, resume-after-interrupt,
                                    # speed limit, filters, flatten, conflicts,
                                    # dry-run, bench
```

The integration script builds `target/release/ketra` if missing, works in a `mktemp -d` sandbox, and prints `PASS: n FAIL: m` (exit status reflects failures). It requires `bash`, `python3`, and GNU coreutils; on this snapshot's Linux x86-64 sandbox it reports 22 passing checks.

## Release profile and binary notes

The workspace release profile (`Cargo.toml`) is `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `strip = true` — small, self-contained binaries. A static Linux build via the musl target should work with no extra link flags since the FFI is plain libc:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release -p ketra-cli --target x86_64-unknown-linux-musl
```

This musl path has not been exercised in the environment where this documentation was written. Portable deployment of the CLI needs only the binary; settings are read from a `ketra.json` (or a `portable.flag` marker) next to the executable, falling back to the per-user config directory (`crates/ketra-core/src/settings.rs`).
