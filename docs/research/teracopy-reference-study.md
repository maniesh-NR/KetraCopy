# TeraCopy reference study — methodology, acquisition record & Track-A instrumentation kit

> **Scope header — what this is / is not.** This is a functional study of
> the reference product's packaging and behavior, executed under the S10
> charter (`harness/state/ROADMAP.md` §S10, decision record
> `harness/decisions/ADR-0003-s10-teracopy-reference-study.md`) and the
> Gate 3 conditions (`harness/reviews/2026-11-13-gate-3-s10-charter.md`).
> It is **not** circumvention research. This repository records only
> functional interface facts (file names/sizes/versions, PE format
> metadata, operating-system API import names) and behavioral
> observations. Every entry carries a provenance tag: **[A]** =
> behavioral trace-derived (observing instrumentation cited) or **[B]** =
> static-inventory-derived. No decompiled code, disassembly listings,
> vendor strings, resources, screenshots, or binaries live in this
> repository; vendor artifacts stay outside it. The methodology and its
> bounds are disclosed in `docs/CLEANROOM.md` §"Post-1.2.1 reference
> study". Facts not attributable to [A] or [B] are excluded here and may
> not inform any KetraCopy implementation.

## 1. Sources and acquisition record [B]

| Artifact | Vendor URL | Size (bytes) | SHA-256 |
|---|---|---|---|
| TeraCopy 3.17 bootstrapper | `codesector.com/files/teracopy.exe` → (redirects to current); actual 3.17: `https://codesector.nyc3.cdn.digitaloceanspaces.com/teracopy3.17.exe` | 12,403,216 | `ac03db52bb68a013d3a8bf4db703ec11976fa1d0aa557eb988e3f926a26656cf` |
| TeraCopy 4.0.5 bootstrapper (= current release) | `https://codesector.com/files/teracopy.exe` and `codesector.com/files/teracopy4.0.5.exe` (both 302 to the same file) | 15,405,824 | `22f050b51c80a344cbdaf368c7f6a9266efca4d95b5109d606466906a421ff7e` |
| TeraCopy 3.17 standalone MSI (x64) | `https://codesector.nyc3.cdn.digitaloceanspaces.com/teracopy3.17.msi` | 14,525,440 | `caf5ac31c758788e7df7cf1332afed8d1c367806703fa9338345a90f2f98738a` |
| Vendor "current" standalone MSI (same-era 3.17 build) | `https://codesector.nyc3.cdn.digitaloceanspaces.com/teracopy.msi` | 14,536,192 | `255891c95914c4649a61815365526e769e54f311fc709cbba8835bef40095bdc` |

Acquisition notes [B]:

- The owner-provided `codesector.com/files/teracopy.exe` and
  `/files/teracopy4.0.5.exe` both 302-redirect to the vendor CDN's
  `teracopy.exe`, which at acquisition time (2026-11-13) IS TeraCopy
  4.0.5 — byte-identical to the CDN's `teracopy4.0.5.exe`. Both owner
  URLs therefore yielded 4.0.5.
- The requested historical version is still hosted by the vendor as
  `teracopy3.17.exe` on the same CDN; its embedded manifest states
  `ProductVersion=3.17` [B], and its payload binaries are dated
  2023-12-03 [B].
- The vendor also publishes standalone MSIs (`teracopy3.17.msi`,
  `teracopy.msi`). Both are x64;1033 MSI Installer databases created
  2023-12-08/09 [B] — i.e. the MSI channel predates 4.0.5 and no 4.0.5
  MSI is published there.
- All four artifacts are stored OUTSIDE this repository (sandbox research
  directory), per the S10 guardrails. Their SHA-256 hashes are denylisted
  in `harness/scripts/backup.sh` (vendor-material guard, Gate 3 RT-4).

## 2. Distribution packaging analysis [B]

Functional facts about the installer container (Advanced Installer,
Caphyon — the installer technology, recorded here because it gated our
Track B tooling and is reproducible with the tools in §3):

- Both bootstrappers are PE32 (i386) GUI executables wrapping an
  MSI-based installation; internal markers identify Advanced Installer,
  and the bootstrapper's own footprint is ~12–15 MB with a 7-section PE
  layout [B].
- The embedded payload container is a stream inside the bootstrapper
  consisting of **two back-to-back 7-Zip archives** plus a UTF-16 INI
  trailer (~1.3–1.5 KB) that states `ProductVersion` and internal
  archive names (`FILES.7z`, `TeraCopy.7z`) [B].
- In the raw stream, **only each 7z archive's 32-byte start header is
  obfuscated** (XOR 0xFF — verified: de-obfuscated headers carry valid
  StartHeaderCRC and NextHeaderCRC against the plain bodies) [B]. The
  next-headers and the compressed metadata header stream are plain, so
  standard tools can list the archives after a 32-byte de-obfuscation.
- The **file-content streams** (LZMA2, 24 MiB dictionary, per the archive
  metadata) are protected by an additional transform that is NOT a
  single-byte XOR (verified empirically — both polarities fail LZMA2
  decoding) [B]. This is a packaging-protection fact, recorded as a
  boundary of this study: it was **not** circumvented. 3.17's payload
  binaries were obtained instead through the vendor's own MSI channel
  (§1); 4.0.5's binaries are deferred to the owner-rig export (§4.1) per
  the charter's degradation path.
- Trailer manifests: 3.17 → `ProductVersion=3.17`; 4.0.5 →
  `ProductVersion=4.0.5` [B].

## 3. Track B tooling (reproducibility) [B]

All tools are committed under `scripts/research/` and run rootless in
this sandbox. Pre-flight record (durable tool inventory, 2026-11-13):
innoextract 1.9 acquired rootlessly via `apt-get download` + `dpkg -x`
(with three boost packages) — works, but is irrelevant here because the
vendor ships Advanced Installer, not Inno Setup; **7-Zip 25.01** (Debian
`7zip` package, rootless) — PE-resource extraction, MSI compound-document
stream extraction, CAB payload extraction; **pefile 2024.8.26** (pip) —
PE inventory; objdump with PE targets present. No sudo needed at any
point.

| Tool | Purpose |
|---|---|
| `xor_ff.py` | XOR-0xFF a stream (first exploration step) |
| `parse_ai_7z.py` | Parse/patch 7z start-header fields; scan for archive boundaries |
| `ai_head_deobf.py` | De-obfuscate + split the two back-to-back 7z archives (head-only model) |
| `ai_content_deobf.py` | Content-stream model probe (packPos decode, boundary XOR test) |
| `lzma2_probe.py` | Direct LZMA2 decode probe of content streams (polarity test) |
| `pe_inv.py` | PE functional inventory: machine/bits/subsystem/sections, import DLLs + I/O- and hash-relevant API names, numeric version |

Raw analysis outputs (listings, PE inventories, trailer manifests) stay
outside the repository; only distilled functional facts are committed.

## 4. Track A — owner-rig instrumentation checklist [A]

Track A runs **on the owner's Windows machine only** (Gate 3 RT-2: no
vendor software in CI). Every scenario must be run identically for
**TeraCopy 3.17**, **TeraCopy 4.0.5** and **KetraCopy v1.2.1**
(co-instrumented side-by-side, per Gate 3 ST-2), and recorded in
`teracopy-findings.md` with [A] tags and the observing instrument
cited. Recommended rig: Windows 10/11 with (a) SSD system disk, (b) a
second fast NVMe volume, (c) a USB HDD for slow-media scenarios. Tools:
Sysinternals Process Monitor + Process Explorer, Windows Performance
Recorder/Analyzer (WPR/WPA) or `xperf`.

Preparation:

1. Snapshot a clean VM (or machine), install TeraCopy 3.17
   (`teracopy3.17.exe`), snapshot again; restore and install TeraCopy
   4.0.5 the same way, so each version is measured in a clean state.
2. KetraCopy v1.2.1 Windows x86_64 zip:
   https://github.com/maniesh-NR/KetraCopy/releases/tag/v1.2.1 (unzip,
   run from its folder; verification default is inline hash — comparable
   to the vendor's verify option).

Scenarios (record duration, throughput and per-instrument observations;
raw `.pml`/`.etl` stay on the rig — only distilled numbers enter the
repo):

1. **Single large file (~10 GB) SSD→SSD, verification ON.** ProcMon
   filtered by process: ReadFile/WriteFile length distribution,
   sequential vs overlapped patterns, queue depth. Process Explorer:
   peak private bytes / working set.
2. **Many small files (10,000 × ~100 KB) SSD→SSD.** Enumeration pattern,
   per-file open/close counts, flush behavior, completion time, peak
   memory.
3. **Verification timing.** From ProcMon/WPA disk totals for scenario 1:
   total bytes read ≈ 2× file size (hash streamed with the copy, single
   pass) or ≈ 3× (separate verification pass after copying).
4. **Interruption & resume.** Kill the process at ~50% of scenario 1;
   note the partial-file/journal layout on disk (names/sizes are
   observable functional facts); relaunch and record whether and how
   resume happens (re-read? skip-ahead? verify-then-continue?).
5. **Throttle behavior.** Set a ~50 MB/s limit in each UI; observe
   smoothness (flat ≈ token bucket vs sawtooth ≈ window-style) and
   adaptation latency when the limit is changed mid-transfer.
6. **Slow destination (USB HDD).** Request-size distribution, queue
   depth, read-ahead pipelining vs write pace; compare with scenario 1
   to detect per-media adaptation.
7. **Sparse file.** Create a file with a large hole (e.g. `fsutil file
   createnew` + `fsutil sparse setflag` + zero a range), copy it,
   compare on-disk allocated size of source vs destination.
8. **Locked file.** Copy a file held open by another app; record waiting
   behavior. (3.17 ships an elevation service importing VSS/overlapped
   I/O [B]; 4.0.5 ships no service [B] — the behavioral difference is
   the interesting datum.)
9. **Hash algorithm exposure.** Which checksum options each UI offers;
   measured throughput of hash verification per algorithm (from
   scenario 1 timings per algorithm).
10. **3.17 vs 4.0.5 delta per scenario** — the 4.x rewrite is the
    highest-novelty target (public sources do not document it).

**4.1 Optional owner-rig export (4.0.5 PE facts — charter degradation
path).** After installing 4.0.5 on the rig, copy (no on-rig analysis
needed) these installed files out for the same PE inventory
(`scripts/research/pe_inv.py`) used for 3.17: `TeraCopy.exe`,
`TeraCopy.dll`, `Context.dll` (x64 and x86), `Blake3.dll`,
`xxHashAVX2.dll`, `xxHashSSE2.dll`, `TeraCopy.appx`. Transfer them to
the sandbox research directory (they stay outside the repo); the
sandbox then runs `pe_inv.py` and records the same functional facts as
for 3.17.

## 5. Status

- **Track B** (2026-11-13): packaging analysis complete for both versions
  (§2); payload inventory complete for both (names/sizes/versions —
  `teracopy-findings.md` §2); PE inventory complete for 3.17 via the
  vendor MSI channel; **4.0.5 PE facts pending the §4.1 owner-rig
  export** (content streams not circumvented, §2).
- **Track A**: kit delivered (§4); awaiting owner-run data.
- Next: S10 plan 2 — findings gate (per-candidate verdicts: adopt → S11
  row / defer / reject; tree contamination check; ≤ 2-plan time-box).
