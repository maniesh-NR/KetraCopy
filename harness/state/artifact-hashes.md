# Artifact hash baseline

sha256 baseline of the current release artifacts. **Single-producer
rule (Gate 1/2)**: the sandbox builds the Linux + src artifacts
(`scripts/package-dist.sh`); the Windows zips and macOS tarballs are
built natively by the CI release workflow
(`.github/workflows/release.yml`) — their hashes are appended below at
release time. Full context: `../evidence/2026-11-13-S7/README.md`.

## 2026-11-13 — KetraCopy 1.2.1 (sandbox artifacts; rustc 1.100.0, 2 vCPU)

```
960cf707ffabb161726b7656010f7f74e307eb4c23c89073d3849dc251f90edf  ketracopy-1.2.1-linux-x86_64.tar.gz
79a1cb0fb0823ac12419866207a6f0c39fa9cb08374db0510349311fdf7ed3f9  ketracopy-1.2.1-portable-linux-x86_64.tar.gz
606da494b0981010bc247c8dd17b3c1996e7afcf944c5de647058dd6a5734abd  ketracopy-1.2.1-src.tar.gz
aacd973efdd4c7fd867cb14a4e2a0fd31140e4287e9c00371ff165f197a07ff7  ketracopy-1.2.1-src.zip
e73845c8572a99b815af6aadec9d104f30892fb749be94703fb1b2ccc603d152  ketracopy_1.2.1_amd64.deb
```

## v1.2.1 release — CI-produced assets (hashes appended at release time)

Downloaded + hashed from the release page 2026-11-13 (single producer:
the CI release workflow; sandbox-verified by download + sha256):

```
d36cb44319608713befe17f32d538257954ae8957f652d94de928064b20c5667  ketracopy-1.2.1-windows-x86_64.zip
499a49e49e9fabc64eb3f5f649f3d8fafd05f25853d2110eb102b62662677165  ketracopy-1.2.1-windows-i686.zip
fa1ea441595b4c76b258b85783a963bc27f0301b928da19a30b71d5e77df29c7  ketracopy-1.2.1-macos-x86_64.tar.gz
249624d3a27a23540324d875e4703a60a6d154bdfc9091f61a59c8b8043ea4a3  ketracopy-1.2.1-macos-aarch64.tar.gz
```

Release page (9 assets, notes + known limitations + the all-asset SHA-256
table): https://github.com/maniesh-NR/KetraCopy/releases/tag/v1.2.1
