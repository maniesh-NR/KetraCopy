# S5 evidence pack — 2026-11-13: re-verification of the 1.2.0 baseline

Environment: Linux x86-64 sandbox, 2 vCPU / 4 GB RAM, **rustc 1.100.0**
(2026-11-09), fresh `target/` (debug+release removed first), toolchain
reinstalled after a sandbox recycle.

## Results

| Check | Result | Log |
|---|---|---|
| `cargo test --workspace` (unit + doc tests) | **129 passed, 0 failed, 0 ignored** | `unit-tests.log` |
| `cargo build --release --workspace` | OK — `ketra` 1.35 MB, `ketracopy-gui` 7.45 MB, 2m35s | `release-build.log` |
| `scripts/test_integration.sh` | **39/39 PASS** | `integration-suite.log` |
| `scripts/test_interop.sh` | **103/106 PASS** (3 fails = expected absence of Windows/macOS cross binaries — S7 scope) | `interop-suite-final.log` |
| bench (interop S12, 128 MiB) | write ~2.4–3.0 GiB/s, read ~4.2–5.2 GiB/s | `interop-suite-final.log` |
| `cargo check --workspace` → `x86_64-pc-windows-gnu` | OK (warnings only) | `check-x86_64-pc-windows-gnu.log` |
| `cargo check --workspace` → `i686-pc-windows-gnu` | OK (warnings only) | `check-i686-pc-windows-gnu.log` |
| `cargo check --workspace` → `x86_64-apple-darwin` | **FAIL — `objc2 v0.4.1`** | `check-x86_64-apple-darwin.log` |
| `cargo check -p ketra-cli` → `x86_64-apple-darwin` | OK | `check-x86_64-apple-darwin-cli-only.log` |
| `cargo check --workspace` → `aarch64-apple-darwin` | **FAIL — same `objc2 v0.4.1`** | `check-aarch64-apple-darwin.log` |
| `cargo check -p ketra-cli` → `aarch64-apple-darwin` | OK | `check-aarch64-apple-darwin-cli-only.log` |
| `scripts/package-dist.sh` | 1.2.0 Linux + src artifacts rebuilt; SHA256SUMS verify | `package-dist.log` |

Interop note: the first run failed 5 checks — two were the GUI smoke
(this recycled sandbox lacked the runtime libs `libxkbcommon-x11` /
`libxcb-xkb`; restored rootlessly under `/tmp/xkblibs`, see
MEMORY.md) and three were the expected absence of Windows/macOS cross
binaries (S7: zigbuild). The final run with the libs present is the
log of record above.

## Findings observed during S5 (recorded for Gate 1; S5 applies NO fixes)

1. **[MAJOR] macOS GUI cross-compile is broken on modern rustc.** The
   locked `objc2 v0.4.1` (chain: `eframe 0.28.1` → `glutin 0.31.3` →
   `icrate v0.0.4` → `block2 0.3.0` → `objc2 0.4.1`) fails E0277 on
   rustc 1.100.0 (`check-*-apple-darwin.log`). macOS CLI (std-only) is
   unaffected. This also affects `release.yml` (which uses the stable
   toolchain) for macOS GUI builds. Candidate fixes for S7: pin an
   older toolchain for macOS GUI builds in CI, a semver-compatible dep
   bump if one exists, or an eframe upgrade (breaks the `egui = 0.28.1`
   pin).
2. **[MINOR] `.deb` control metadata uses placeholders** — Homepage
   `https://example.invalid/ketracopy` (should be the GitHub URL).
3. **[MINOR] `scripts/package-dist.sh` expects a pre-existing
   `dist/README.txt`** (currently stale 1.0.0-era content, only used to
   refresh `dist/linux-x86_64/README.txt`); not reproducible from a
   fresh clone.
4. **[MINOR] Interop S11 cross-binary checks fail by design until the
   S7 zigbuild cross binaries exist** (windows zip / macos tar.gz).
5. **[NOTE] Non-fatal rustc warnings**: FFI type-alias naming
   (`c_ulong`, `c_char`), unused-code warnings in ketra-cli/ketra-gui.

## dist/ artifact hashes (1.2.0, this machine, this toolchain)

```
a817676af4716cc034e3323ffb95349757e79c57c1e700dca69af62524534553  ketracopy-1.2.0-linux-x86_64.tar.gz
5145b0c881252c6244d9d67f07bfcd4971bc2af66576faff0d410d24583fd3e9  ketracopy-1.2.0-portable-linux-x86_64.tar.gz
2c4b8019ba9e1a7e3e430682f4f44b20629d8f2ba549122dc477800893fc769a  ketracopy-1.2.0-src.tar.gz
afb8bc7b0debe7178e2cf60e432c7a01fe517f500ea3074f65e546df89b15173  ketracopy-1.2.0-src.zip
aff2437dfd3b76e43efd2529b5d0767c734040c98d352d75b42ca5af930e75df  ketracopy_1.2.0_amd64.deb
```
