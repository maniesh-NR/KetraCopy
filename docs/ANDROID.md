# KetraCopy for Android

A developer guide to the KetraCopy Android app: build pipeline, package
layout, the four UI scopes, known limitations, and troubleshooting.

> **Status (1.2.1)**: source-complete but **not currently building** — the
> Gradle build still references `crates/ketra-ffi`/`crates/ketra-net`
> (moved to `attic/` in 1.1.0) and the Kotlin side is being re-pointed at
> the `ketra-core` JNI bridge (`crates/ketra-core/src/jni.rs`). The APK
> workflow is dispatch-only until then; no APK ships with 1.2.1. This
> document describes the intended architecture.

## Overview

The KetraCopy Android app is the mobile face of the same Rust engine that
powers the desktop CLI (`ketra`) and GUI (`ketracopy-gui`). It is a single APK
that combines four things:

- a **file manager** for local storage (via the Android Storage Access
  Framework, accelerated by the `ketra-core` copy/move pipeline),
- **LAN transfer** to and from other KetraCopy devices (and any LocalSend
  v2 peer), using mDNS discovery and plain HTTP/1.1 on port 53317 (HTTPS
  planned),
- an entry point for **Cloud and SMB** destinations through a pluggable
  `CloudProvider` interface, and
- **compression and share-sheet** integration (zip via `java.util.zip` and
  Android's `Intent.ACTION_SEND`).

All four scopes feed through the same Rust core (integrity checks, manifest
generation, resume, throttling) so an Android-to-Android transfer produces a
destination tree that `ketra verify` on the desktop will pass without
modification.

- Application ID / package: `io.ketracopy`
- minSdk: **26** (Android 8.0)
- targetSdk / compileSdk: **35** (Android 15)
- Language: Kotlin 2.0.21, Jetpack Compose (BOM 2024.12.01), Material 3
- Native: `libketraffi.so` (a `cdylib` built from `crates/ketra-ffi` via
  UniFFI 0.28.3 proc-macro scaffolding)

## Architecture

The app follows a "Rust core, Kotlin thin" split the project deliberately
chose: every operation that touches bytes on disk or on the wire lives in
Rust and is exposed to Kotlin through a single UniFFI-generated facade,
`io.ketracopy.uniffi.ketra.KetraEngine`. Kotlin code only orchestrates UI
state, SAF URIs, foreground services, and Android permissions.

```text
                       Jetpack Compose UI
                  (ui/screens, ui/theme, ui/viewmodel)
                              |
                              v
        Kotlin orchestration layer  (io.ketracopy)
        data/   storage/   net/   provider/   MainActivity
                              |
                              v
       io.ketracopy.uniffi.ketra.KetraEngine   (UniFFI-generated Kotlin)
                              |  (JNI)
                              v
                       libketraffi.so   (cdylib, cargo-ndk)
                              |  (in-process)
                              v
        ketra-ffi  ->  ketra-net (mDNS, rustls, HTTP server/client)
                    ->  ketra-core (engine, pipeline, hashing, manifest)
```

The JNI boundary is owned entirely by UniFFI: Kotlin calls a generated
wrapper, the wrapper marshals arguments into C ABI types, and the Rust
`ketra-ffi` crate dispatches them to either `ketra-core` (local copy/move,
hashing) or `ketra-net` (mDNS discovery, HTTPS send/receive). Progress
events flow back through a `ProgressCallback` interface Kotlin implements;
Rust worker threads call `on_event` from inside `std::thread::scope`.

## Build prerequisites

| Tool | Version | Notes |
|---|---|---|
| JDK | 17 | Temurin recommended; AGP 8.7 requires JDK 17. |
| Android Studio | Koala (2024.1.1) or newer | Or just the command-line tools. |
| Android SDK | Platform 35 | `platforms;android-35`. |
| Build tools | 35.0.0 | `build-tools;35.0.0`. |
| NDK | 27.0.12077973 | `ndk;27.0.12077973`. Set `ANDROID_NDK_HOME`. |
| Rust | stable | `rustup default stable`. |
| Android targets | 4 triples | `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android`. |
| cargo-ndk | 3.0.0 | `cargo install cargo-ndk --version 3.0.0 --locked`. |
| uniffi-bindgen-cli | 0.28.3 | `cargo install uniffi-bindgen-cli --version 0.28.3 --locked`. |

Gradle 8.9 (pinned by `android/gradle/wrapper/gradle-wrapper.properties`)
is downloaded automatically by the wrapper script, so you do not need a
system Gradle install.

## Local build

From the workspace root:

```bash
# 1. one-time tool setup (skip if already done)
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
cargo install cargo-ndk --version 3.0.0 --locked
cargo install uniffi-bindgen-cli --version 0.28.3 --locked

# 2. point cargo-ndk at the NDK
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.0.12077973"

# 3. build the debug APK
cd android && ./gradlew assembleDebug
```

The `assembleDebug` task graph automatically runs two Rust-side tasks before
compiling Kotlin (see `android/app/build.gradle.kts` for the wiring):

1. **`buildRustLibs`** — invokes `cargo ndk -t arm64-v8a -t armeabi-v7a -t
   x86_64 build --release -p ketra-ffi`, then moves each
   `target/<triple>/release/libketraffi.so` into
   `android/app/src/main/jniLibs/<abi>/libketraffi.so`.
2. **`generateUniFFIBindings`** — runs `uniffi-bindgen generate --library
   <arm64-v8a>/libketraffi.so --language kotlin --out-dir
   src/main/kotlin/io/ketracopy/uniffi/` to produce `Ketra.kt` from the
   metadata embedded in the compiled `.so`.

Both tasks declare Gradle inputs (the Rust source dirs and `Cargo.toml`) so
they re-run only when the Rust code changes. For pure Compose UI iteration
you can skip them; the app launches but will crash on the first UniFFI call
until the `.so` is present.

Output:

```text
android/app/build/outputs/apk/debug/app-debug.apk
```

Install on a connected device or emulator:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

## CI build

The same pipeline runs on GitHub-hosted Ubuntu via
[`.github/workflows/build-apk.yml`](../.github/workflows/build-apk.yml).
The workflow:

1. checks out the repo,
2. installs JDK 17, Rust stable with the four Android targets,
   `cargo-ndk` 3.0.0, `uniffi-bindgen-cli` 0.28.3,
3. installs the Android SDK (platform-tools, platform 35, build-tools 35,
   NDK 27.0.12077973) via `android-actions/setup-android@v3`,
4. exports `ANDROID_NDK_HOME`,
5. runs `cd android && ./gradlew assembleDebug --no-daemon --stacktrace`,
6. uploads the APK as the `ketracopy-debug-apk` artifact.

A parallel `cargo-test` job runs `cargo test --workspace` and
`bash scripts/test_integration.sh` to keep the Rust engine green on every
push.

## Project structure

The Kotlin sources live under
`android/app/src/main/kotlin/io/ketracopy/`:

```text
io.ketracopy/
  MainActivity.kt          launcher activity + share intent handling
  ui/
    theme/                 Color.kt, Type.kt, Theme.kt, MD3 dynamic colors
    screens/               FileManagerScreen, LanPeersScreen,
                           CloudSmbScreen, SettingsScreen (the four scopes)
    viewmodel/             Compose ViewModels binding KetraEngine to UI state
  data/                    KetraEngineHolder: app-scoped Engine singleton
  storage/                 LocalStorage.kt: SAF DocumentFile adapter
                           (list/copy/move/delete via ketra-core)
  net/                     ReceiveService.kt: foreground service that runs
                           the LAN receiver (mDNS announce + HTTPS server)
  provider/                CloudProvider.kt (interface), WebDavProvider.kt
                           (WebDAV adapter via OkHttp)
```

The UniFFI-generated `io.ketracopy.uniffi.ketra` package (containing
`Ketra.kt` and friends) is produced by the `generateUniFFIBindings` Gradle
task and should not be edited by hand; regenerate by re-running the build.

## The four UI scopes

The app's bottom navigation exposes four scopes, each backed by a Compose
screen and a ViewModel that calls into `KetraEngine`:

1. **Local storage** (`FileManagerScreen`) — browse, copy, move, delete, and
   verify files in SAF-exposed trees (internal storage, SD card, USB OTG).
   Every copy/move goes through `ketra-core`'s pipeline: AIMD chunk sizing,
   inline verification, resume from a partial destination, and a Phase 3
   `.ketra/manifest.jsonl` written under the destination root. Same fast
   paths as the desktop CLI (`copy_file_range` on Linux, reflink where the
   filesystem supports it).

2. **LAN peers** (`LanPeersScreen`) — discover other KetraCopy (and
   LocalSend v2) devices on the same LAN via mDNS `_localsend._tcp.local.`,
   select files via SAF, confirm the receiver's SHA-256 fingerprint, then
   transfer over HTTPS on port 53317. When both endpoints are KetraCopy the
   accelerated extension engages (parallel ranged streams, per-chunk SHA-256
   / XXH64 / CRC32C, manifest exchange). See
   [LAN-PROTOCOL.md](LAN-PROTOCOL.md) for the wire format.

3. **Cloud / SMB** (`CloudSmbScreen`) — browse a remote tree through the
   `CloudProvider` interface. `WebDavProvider` (OkHttp + hand-rolled
   XML parsing) is the working implementation; an SMB provider and a Google
   Drive provider are stubbed. Add a new backend by implementing
   `CloudProvider` and registering it in the screen's view model.

4. **Compression / share** — zip a selection via `java.util.zip` (with
   optional SHA-256 entries in a `.ketra/manifest.jsonl` sidecar) and hand
   the result to Android's share-sheet (`Intent.ACTION_SEND`). Incoming
   shares (`SEND` / `SEND_MULTIPLE` intents) are received by `MainActivity`
   and forwarded to `KetraEngine` for copy into the user-chosen destination.

## Known limitations

- **SMB and Google Drive providers are stubbed.** They exist as
  `CloudProvider` implementations that throw `NotImplementedError` on
  connect. Wire them up (jcifs-ng for SMB, the Google Drive REST API for
  Drive) to enable those scopes.
- **No file-provider for receiving via share on cold start.** The manifest
  declares `SEND`/`SEND_MULTIPLE` intent filters on `MainActivity`, but when
  the app is launched from a share-sheet while cold-started, the incoming
  `Intent` extras are not yet plumbed into `KetraEngine` (only the
  warm-path through `MainActivity.onCreate` reads them). Shares received
  while the app is already running land correctly.
- **Notifications require `POST_NOTIFICATIONS` on Android 13+.** The
  permission is declared in `AndroidManifest.xml`; the receiver foreground
  service posts the transfer status as a notification, but the runtime
  permission request is not yet implemented in the UI. Without it the
  notification silently no-ops on Android 13+ and the foreground service
  still runs (just invisibly).
- **mDNS multicast needs `CHANGE_WIFI_MULTICAST_STATE`** plus a
  `MulticastLock` acquired at runtime; the manifest has the permission, but
  the lock acquisition is best-effort. On some OEM Wi-Fi drivers multicast
  is filtered even with the lock, in which case peers must be entered
  manually by IP.

## Troubleshooting

**`NDK not found` / `cargo-ndk` fails to locate the toolchain.**
`cargo-ndk` honors `ANDROID_NDK_HOME` (or `ANDROID_NDK_ROOT`). Set it
explicitly to `$ANDROID_HOME/ndk/27.0.12077973` (or whichever NDK you
installed). On GitHub Actions the workflow exports this in the
`Export ANDROID_NDK_HOME` step; locally:

```bash
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.0.12077973"
```

**`uniffi-bindgen: command not found`.** The `generateUniFFIBindings` task
shells out to the `uniffi-bindgen` binary on `PATH`. Install it once:

```bash
cargo install uniffi-bindgen-cli --version 0.28.3 --locked
```

If `cargo install` succeeds but Gradle still can't find it, check that
`~/.cargo/bin` is on `PATH` for the shell that launches Gradle.

**`Cargo.lock changed` / `cargo ndk build` fails with a lockfile error.**
The `--locked` flag (used by the CI workflow) refuses to mutate
`Cargo.lock`. If your local edits require a new dependency, run
`cargo update -p <crate>` (or just `cargo build`) without `--locked` first,
commit the updated `Cargo.lock`, then re-run the locked build. If the
failure looks like a stale registry cache, `cargo clean && cargo ndk build`
from a fresh `target/` resolves most one-off glitches.

**Gradle fails with `SDK location not found`.** Either set
`local.properties` with `sdk.dir=/path/to/Android/Sdk`, or export
`ANDROID_HOME` (or `ANDROID_SDK_ROOT`) in your shell before launching
`./gradlew`. The CI workflow uses `android-actions/setup-android@v3`, which
sets `ANDROID_HOME` for the rest of the job.

**Kotlin compile error: unresolved reference `ketra`.** The
`generateUniFFIBindings` task did not run (or did not finish). Check the
Gradle log for the `buildRustLibs` task: if `cargo ndk` failed, there will
be no `.so` and `uniffi-bindgen generate --library <so>` will fail in turn.
Fix the Rust build first; the Kotlin references resolve automatically once
the generated `Ketra.kt` is on the source set.

**App installs but crashes on first `KetraEngine` call.** The native
library is missing for the running ABI (most often when testing on an
`x86_64` emulator without having built that target). Confirm with:

```bash
unzip -l app-debug.apk | grep libketraffi
```

You should see three entries under `lib/arm64-v8a/`, `lib/armeabi-v7a/`,
and `lib/x86_64/`. If one is missing, re-run `./gradlew buildRustLibs` and
verify `cargo ndk` actually built all three targets.
