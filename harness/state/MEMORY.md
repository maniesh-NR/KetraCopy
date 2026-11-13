# KetraCopy — Harness Memory (persistent)

> **Read this FIRST at every session start.** This is the project's memory:
> environment, commands, state, gotchas. Update it whenever state changes
> (the update rides along with the next backup).

## Identity

- **Project**: KetraCopy — fast, verifiable, resumable file transfer
  (TeraCopy-style clean-room engine), MIT license.
- **Version**: 1.2.1 (full history in `CHANGELOG.md`).
- **Local workspace**: `/home/z/my-project/download/ketracopy`
  (git repo, branch `main`).
- **GitHub**: `https://github.com/maniesh-NR/KetraCopy` (public; canonical
  URL — the lowercase spelling redirects to it).
- **Governing rules**: `harness/WORKFLOW.md` (owner-mandated: backups end
  every execution plan; adversarial consensus gates between stages).
- **Progress**: `harness/state/ROADMAP.md` — single source of truth for
  stage/gate status.

## Sandbox environment (re-verify after every recycle)

- 2 vCPU, ~4 GB RAM, **no sudo**; `Xvfb` present; GitHub API reachable.
- Sandboxes get recycled: everything under `/home/z/my-project/` persists;
  `$HOME` toolchains do **not**. After a recycle:
  1. `curl -sSfL https://sh.rustup.rs -o /tmp/rustup-init.sh && sh /tmp/rustup-init.sh -y --profile minimal --no-modify-path`
  2. `export PATH="$HOME/.cargo/bin:$PATH"` (needed in EVERY bash call)
  3. Cross-builds need **zig + cargo-zigbuild** (see below).
- Rust: stable (1.100.0 as of 2026-11-13). MSRV: 1.75. egui pinned: 0.28.1.
- **Secrets**: GitHub PAT lives OUTSIDE the repo at
  `/home/z/my-project/.secrets/github_token`; git credential store at
  `/home/z/my-project/.secrets/git-credentials` (format
  `https://<login>:<token>@github.com`, login = repo owner). NEVER commit
  them. If lost, the owner re-provides the token; then re-run:
  `git config --local credential.helper 'store --file=/home/z/my-project/.secrets/git-credentials'`
  and rewrite the credentials file.

## Security incident (2026-11-13, resolved at the refs level)

- A pre-workflow push (2026-11-05) had published this repo WITH the owner's
  PAT file in its tree (`upload/Ketracopy github access token for GLM.txt`)
  plus tool junk, and a `v1.1.0` tag + release pointing into that history.
- 2026-11-13: the clean 1.2.0 baseline was force-pushed over that history;
  the remote `v1.1.0` tag and its release (9 assets) were deleted.
- Dangling commits may remain accessible by SHA on GitHub until purged —
  **the owner must rotate/regenerate the PAT** (advisory issued 2026-11-13).
  GitHub push protection also confirmed the token string.
- **Outer sandbox repo warning**: `/home/z/my-project/.git` is a SECOND,
  local-only git repo (root = the whole sandbox directory). It contains
  secrets in its history; its `origin` remote was removed and a pre-push
  hook now blocks all pushes. NEVER push it or re-add a remote. The public
  project repo is `download/ketracopy/.git` (root = the workspace itself).
- Local record of the leaked remote history: branch `archive/remote-pre-clean`
  in the OUTER sandbox repo.

## Commands (from the workspace root)

| Task | Command |
|---|---|
| Unit tests | `cargo test --workspace` |
| Release binaries | `cargo build --release --workspace` |
| Integration suite | `bash scripts/test_integration.sh` |
| Interop suite (needs Xvfb) | `bash scripts/test_interop.sh` |
| Package all dist artifacts | `bash scripts/package-dist.sh` (cross targets skip gracefully if their binaries are absent) |
| Backup — END OF EVERY PLAN | `bash harness/scripts/backup.sh "<what changed>"` |
| Live mDNS tests | `KETRA_MDNS_LIVE=1 cargo test` |

### Cross builds (Windows / macOS)

**Decision (2026-11-13): release binaries for Windows (msvc, x86_64 +
i686, CLI + GUI) and macOS (CLI, both darwin targets) are built NATIVELY
by the CI release workflow (`.github/workflows/release.yml`) —
single-producer rule; the sandbox produces only Linux + src artifacts.**
A sandbox zigbuild cross-compile was attempted (zig 0.13.0 +
cargo-zigbuild 0.24.1, rootless under `/home/z/my-project/tools/`) and
abandoned after 4 attempts: zig's linker rejects rustc's mingw link line
(evidence: `harness/evidence/2026-11-13-S7/zigbuild-*.log`). The
filtering linker wrappers survive at `/home/z/my-project/tools/zigwrap-*.sh`
if the attempt is ever resumed. Sandbox-side verification of the
cross targets: `cargo check --workspace --target <windows triple>` (green)
and `cargo check -p ketra-cli --target <darwin triple>` (green; the macOS
GUI check FAILS on modern rustc — locked `objc2 0.4.1`, no semver fix
exists per `cargo update -p objc2@0.4.1 --dry-run`; macOS GUI deferred).

## Structure map

- `crates/ketra-core` — std-only engine: 3-phase pipeline, AIMD + token
  bucket throttle, SHA-256/XXH64/CRC32C/MD5 (all streaming), sparse files,
  resume journal, conflict policies, filters, `lan/` (mDNS + HTTP
  LocalSend v2 + Ketra accelerated streams), `jni.rs` (Android bridge),
  `platform/` FFI (Windows/Linux/macOS).
- `crates/ketra-cli` — the `ketra` CLI: copy/move/manifest/verify/bench/
  peers/send/receive/clip/shell/settings.
- `crates/ketra-md3` + `crates/ketra-gui` — Material Design 3 egui GUI.
- `android/` — Kotlin/Compose file manager; **APK is built by CI**
  (`.github/workflows/build-apk.yml`) — the sandbox has no Android SDK.
- `integrations/`, `packaging/`, `portable/` — file-manager menus, deb /
  Inno Setup / flatpak / app-bundle packaging, portable bundle scripts.
- `attic/` — prototype crates (`ketra-net` rustls HTTPS design, `ketra-ffi`);
  NOT workspace members, kept as reference. (Stale duplicates in `crates/`
  were deleted 2026-11-13 — do not resurrect.)
- `.github/workflows/` — `build-apk.yml` (APK + tests on push),
  `release.yml` (native Windows/Linux/macOS release builds attached to
  `v*` tag releases).
- `harness/` — this memory + governance (see `harness/README.md`).

## Semantics & gotchas

- Directory copy = TeraCopy "paste into folder": the source's CONTENTS land
  in the destination root.
- Verify levels: `off | inline | manifest` (default inline).
- `ketra verify` exits **4** on corruption (tests rely on this).
- `dist/` and `KetraCopy-portable/` are gitignored — release artifacts ship
  via GitHub Releases, not git. sha256 baseline:
  `harness/state/artifact-hashes.md`.
- mDNS live tests are opt-in (`KETRA_MDNS_LIVE=1`).
- The dist packager expects `dist/README.txt` to pre-exist (only used to
  refresh `dist/linux-x86_64/README.txt`); if missing it is skipped —
  track as a packaging nit.
- **GUI smoke under Xvfb needs `/tmp/xkblibs`** (rootless runtime libs the
  recycled sandbox lacks): `apt-get download libxkbcommon-x11-0 libxcb-xkb1`
  → `dpkg -x` both into `/tmp/xkblibs` → symlink
  `libxkbcommon-x11.so -> libxkbcommon-x11.so.0` and
  `libxkbcommon.so -> /usr/lib/x86_64-linux-gnu/libxkbcommon.so.0` in
  `/tmp/xkblibs/usr/lib/x86_64-linux-gnu` (`scripts/test_interop.sh`
  prepends that path itself). Recreate after every sandbox recycle.

## Session bootstrap checklist

1. Read this file, `harness/state/ROADMAP.md`, the newest
   `harness/plans/*` and the latest `harness/reviews/*`.
2. Reinstall the toolchain if the sandbox was recycled (see above); check
   `git status`, `git remote -v`, `git log --oneline -5`.
3. Continue the first PENDING stage in `ROADMAP.md` — **its adversarial
   gate runs first if the previous stage is not yet gated**.
4. End every execution plan with `bash harness/scripts/backup.sh "<desc>"`.
