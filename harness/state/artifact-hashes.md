# Artifact hash baseline

sha256 baseline of the current `dist/` release artifacts (gitignored;
artifacts ship via GitHub Releases). Full per-run logs and context:
`../evidence/2026-11-13-S5/README.md`.

## 2026-11-13 — KetraCopy 1.2.0 (Linux + src; rustc 1.100.0, 2 vCPU sandbox)

Windows/macOS cross binaries pending S7 (zigbuild); macOS GUI additionally
blocked by the objc2 finding (Gate 1).

```
a817676af4716cc034e3323ffb95349757e79c57c1e700dca69af62524534553  ketracopy-1.2.0-linux-x86_64.tar.gz
5145b0c881252c6244d9d67f07bfcd4971bc2af66576faff0d410d24583fd3e9  ketracopy-1.2.0-portable-linux-x86_64.tar.gz
2c4b8019ba9e1a7e3e430682f4f44b20629d8f2ba549122dc477800893fc769a  ketracopy-1.2.0-src.tar.gz
afb8bc7b0debe7178e2cf60e432c7a01fe517f500ea3074f65e546df89b15173  ketracopy-1.2.0-src.zip
aff2437dfd3b76e43efd2529b5d0767c734040c98d352d75b42ca5af930e75df  ketracopy_1.2.0_amd64.deb
```
