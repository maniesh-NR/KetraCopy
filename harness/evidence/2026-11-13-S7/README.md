# S7 evidence pack — 2026-11-13: Gate-1 fixes + docs/CI honesty pass + 1.2.1

Environment: Linux x86-64 sandbox, 2 vCPU / 4 GB RAM, rustc 1.100.0
(2026-11-09) — reinstalled once mid-stage after a sandbox recycle (see
MEMORY.md). Version: **1.2.1**.

## Results (all logs in this directory)

| Check | Result | Log |
|---|---|---|
| `cargo test --workspace` | **133 unit tests, 0 failed** (89 core + 20 CLI + 24 MD3; +4 regression tests: reserve-path ×1, prepare-collision ×1, source-shrink ×1, huge-Content-Length ×1, overwrite-fallthrough ×1 — net +5 incl. replaced unique_path test → 89 core) | `unit-tests-final.log` |
| `cargo build --release --workspace` | OK — ketra + ketracopy-gui (1.2.1) | `release-build-final.log` |
| `scripts/test_integration.sh` | **41/41 PASS** (+2 new: `--no-times` re-copy must not size-skip) | `integration-suite-final.log` |
| `scripts/test_interop.sh` | **105/105 PASS** (windows/macOS cross-binary checks now explicitly skip: those packages are produced natively by the CI release workflow — single-producer rule) | `interop-suite-final.log` |
| bench (interop S12, 128 MiB) | write ~2.7–2.9 GiB/s, read ~4.3–4.8 GiB/s | `interop-suite-final.log` |
| `cargo check --workspace` → win x86_64 / i686 | OK (warnings only) | `check-*-pc-windows-gnu.log` |
| `cargo check -p ketra-cli` → mac x86_64 / aarch64 | OK | `check-*-apple-darwin-cli.log` |
| `cargo update -p objc2@0.4.1 --dry-run` | **"Locking 0 packages"** — no semver-compatible fix exists (the E0277 fix landed in the objc2 0.5 line; `icrate 0.0.4` pins `^0.4`) → macOS GUI stays deferred; option (b) closed with evidence | (this README) |
| `scripts/package-dist.sh` | 1.2.1 Linux + src artifacts + sums | `package-dist-final.log` |
| Sandbox zigbuild → windows-gnu | **ABANDONED after 4 attempts** (zig 0.13.0 + cargo-zigbuild 0.24.1): zig's linker rejects rustc's mingw link line (`--disable-auto-image-base`, then strict dynamic-system-library resolution of `-Bdynamic`-scoped `msvcrt`/`gcc_eh`/`synchronization`). Windows release binaries are now built NATIVELY by the CI release workflow (msvc) instead — single-producer rule: sandbox = Linux + src; CI = Windows + macOS. | `zigbuild-x86_64-pc-windows-gnu.log` |

## Fixes applied (Gate 1, round 1 → this round)

Engine: F1 LAN receiver reservation (CRITICAL) + orphan cleanup on session
expiry; F2 `Error::Shrank` on mid-copy source shrink (no zero-pad, no
`done` journal); F3 `--no-times` never size-skips; F4 8 MiB client
response cap; F5 sync-before-journal-record + probe verifies chunk below
a trusted bound; F6 manifest orphan removal, reserved-name prefixing,
paste-smart `..`/absolute-title rejection, dead code removal.
**Follow-up bug found and fixed by the new regression tests**:
`--conflict overwrite` (and the GUI's overwrite-all) failed with
`File exists` on existing destinations — the reflink fast path opens with
`create_new` and EEXISTed instead of falling through to the streaming
copy (masked until now by the size-only skip). Fixed + unit and
end-to-end regression tests.

Docs/CI: README (repo link, 1.2.1 framing, macOS/Windows rows, LAN
honesty, Known limitations), LAN-PROTOCOL (TLS clearly "planned — attic
prototype", stale crate paths fixed), ARCHITECTURE (attic prototypes
marked superseded, HTTP not HTTPS, verification section updated), BUILD
(rfd removed, counts fixed, binary name), ANDROID (status banner),
ATTRIBUTIONS (adversarial-review panel row), CHANGELOG 1.2.1 section,
Cargo.toml (1.2.1, canonical repo URL), release.yml (native Windows msvc
+ macOS CLI builds, workflow_dispatch, single-producer asset names),
build-apk.yml (dispatch-only until the Android repair), deb/iss/metainfo
URLs, tracked `packaging/dist-README.txt` + `dist-README-macos.txt`
templates, portable script versions, version-derived names in the interop
suite + deb/portable builders.

## dist/ artifact hashes (1.2.1, this machine, rustc 1.100.0)

```
3004efc90558cf538bf58170606f516410a25939fc9d63d5fd69cb0e171205f3  ketracopy-1.2.1-linux-x86_64.tar.gz
72bf261ad74385eaa37b757ee2e4e1889d878455caa1dbf2aa188e17f3fa72bc  ketracopy-1.2.1-portable-linux-x86_64.tar.gz
0b8a6194f9fd067264e447d5d888d34b7627543e019392b94dfe719dbcf4b066  ketracopy-1.2.1-src.tar.gz
ce5e1649a031977e98aa7300ad889e31098d07b86634b3ce002808d255685b87  ketracopy-1.2.1-src.zip
58c53bae8302ca13a72e6992b8baf0e3c9349febf32184d151870414e68c8359  ketracopy_1.2.1_amd64.deb
```

Windows zips + macOS tarballs are produced by the CI release workflow
(`release.yml`) and must hash-match the release page at S9.
