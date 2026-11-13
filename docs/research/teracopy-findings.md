# TeraCopy reference study — findings & optimization-candidate mapping

> **Scope header — what this is / is not.** Functional findings from the
> S10 TeraCopy reference study (charter: `harness/state/ROADMAP.md` §S10;
> ADR-0003; Gate 3 record
> `harness/reviews/2026-11-13-gate-3-s10-charter.md`). Only functional
> interface facts and behavioral observations appear here, each with a
> provenance tag (**[A]** behavioral trace / **[B]** static inventory —
> see `teracopy-reference-study.md` for methodology, acquisition and the
> Track-A kit). No vendor code, strings, resources or binaries are
> reproduced. Candidate rows become adoptable ONLY after the findings
> gate issues per-candidate verdicts (S10 plan 2).

## 1. Version identification [B]

- TeraCopy 3.17: acquired from the vendor CDN; embedded manifest
  `ProductVersion=3.17`; payload built 2023-11/12-2023.
- TeraCopy 4.0.5: byte-identical to the vendor's current release
  (`teracopy.exe`); embedded manifest `ProductVersion=4.0.5`; payload
  built 2026-10-01. The 3.17↔4.0.5 pair spans the vendor's 4.x rewrite.

## 2. Payload inventories (names / sizes / dates) [B]

**3.17** (FILES.7z: 122 files, 50 folders, 19,111,918 bytes unpacked,
dated 2023-12-03) — root = 64-bit binaries plus a `32-bit/` mirror:

| File | Bytes (x64) | Notes [B] |
|---|---|---|
| TeraCopy.exe | 4,825,824 | GUI main; packed (see §3) |
| TeraCopy.dll | 1,869,544 | engine/COM server (see §3) |
| Context.dll | 1,755,872 | shell context-menu COM server |
| TeraCopyService.exe | 317,672 | elevation service (VSS + overlapped I/O imports) |
| updater.exe | 1,213,664 | auto-updater |
| Blake3.dll | 162,816 | streaming BLAKE3 (Blake3Init/Update/Reset/Finalize/Free) |
| xxHashAVX2.dll | 287,464 | XXH32_* + XXH3_128bits_* exports |
| xxHashSSE2.dll | 299,744 | XXH32_* + XXH3_128bits_* exports |
| App/Locale/*.po | 135 locale rows | Gettext-based localization |
| AppDataFolder/TeraCopy/Scripts | 3 files | PowerShell user scripts (IFTTT/Ntfy notifications, eject) |

**4.0.5** (FILES.7z: 126 files, 53 folders, 26,444,636 bytes unpacked,
dated 2026-10-01) — reorganized into `x86/`, `x64/`, `ARM64X/` trees
plus a legacy `32-bit/` alias tree:

| File | Bytes (x64) | Delta vs 3.17 [B] |
|---|---|---|
| TeraCopy.exe | 6,115,648 | grew +27% (4.83 MB → 6.12 MB) |
| TeraCopy.dll | 261,440 | **shrank −86%** (1.87 MB → 0.26 MB); also built for x86 (1,197,376) and ARM64X (451,392) |
| Context.dll | 205,120 | **shrank −88%** (1.76 MB → 0.21 MB); x86 1,098,560; ARM64X 381,248 |
| TeraCopyService.exe | — | **removed** (no service in 4.0.5) |
| updater.exe | 2,057,024 | grew +69% |
| Blake3.dll | 174,400 | still shipped (x86 build 102,208) |
| xxHashAVX2.dll / xxHashSSE2.dll | 287,552 / 299,840 | still shipped (rebuilt ~88 bytes apart from 3.17's) |
| TeraCopy.appx | 429,534 | **new** — packaged-companion artifact (Win11 context-menu packaging support) |
| ARM64X tree | — | **new** — ARM64/ARM64X support |

## 3. PE inventory — 3.17 (via the vendor's MSI channel) [B]

- **TeraCopy.exe** (x64, GUI, 2023-12-03): **packed** — unnamed sections,
  one-symbol-per-DLL import stubs (shell32, kernel32, advapi32, user32),
  real imports resolved at runtime. Exported symbols include
  `__dbk_fcall_wrapper`/`dbkFCallWrapperAddr` — markers of an
  Embarcadero **Delphi** toolchain. Consequence: the main GUI binary
  resists static import analysis (functional fact about the vendor's
  packaging, not a study target).
- **TeraCopy.dll** (x64, 2023-11-13): Delphi-style section set
  (.text 1452K, .idata, .didata delay-imports, .edata). kernel32 imports
  (98 symbols) include `ReadFile`, `WriteFile`, `SetFilePointer`,
  `SetEndOfFile`, `CreateFileW` — and **no** `CopyFileEx*`/`MoveFileEx*`,
  **no** `CreateIoCompletionPort`/`GetQueuedCompletionStatus`, **no**
  `ReadFileEx`/`WriteFileEx`. The 3.17 engine therefore runs **manual,
  synchronous read/write loops with explicit seeking** — the same design
  class as KetraCopy's pipeline, not an OS-copy shortcut.
- **Context.dll** (x64, 2023-09-04): shell COM server
  (DllCanUnloadNow/DllGetClassObject/DllRegisterServer/
  DllUnregisterServer); kernel32 imports include ReadFile/WriteFile/
  MoveFileW — the shell hook itself performs I/O.
- **TeraCopyService.exe** (x64, 2023-06-29): kernel32 imports include
  `GetOverlappedResult`, `CancelIo`, `GetFileSizeEx`; imports
  `vssapi.dll` — **VSS shadow-copy + overlapped I/O** in the elevation
  service (locked-file copying in 3.17 ran through this service).
- **Blake3.dll / xxHashAVX2.dll / xxHashSSE2.dll**: clean streaming
  hash-library APIs (`Blake3Init/Update/Finalize`; `XXH32_*`;
  `XXH3_128bits_*`).
- **No CNG/BCrypt/Crypt* imports** in any shipped 3.17 binary — hashing
  is entirely in the vendor's own SIMD libraries / in-process code.

**4.0.5 PE inventory: pending** — its payload binaries sit behind the
bootstrapper's content-stream protection (not circumvented — see
`teracopy-reference-study.md` §2); obtainable via the charter's
degradation path (owner-rig export, study §4.1) or already covered by
Track A behavior runs. The [B] size deltas (§2) already evidence a
significant engine/UI re-architecture in 4.x.

## 4. Optimization-candidate mapping

Per-candidate verdicts are issued by the **S10 findings gate (plan 2)**
— adopt (→ S11 row: expected effect + benchmark + effort) / defer / reject.
"Measured delta" columns fill from Track-A data (or stay N/A if the owner
provides none).

| # | Observation (provenance) | KetraCopy v1.2.1 today | Candidate change | Measured delta (Track A) | Expected effect | Risk / effort | Gate verdict |
|---|---|---|---|---|---|---|---|
| C1 | Vendor ships CPU-dispatched SIMD hash libraries (XXH3-128 AVX2 + SSE2, BLAKE3) [B] | Portable std-only SHA-256/XXH64/CRC32C; no CPU-feature specialization | Optional SIMD fast paths for the inline-verify hash (core::arch intrinsics, no new deps, runtime CPU dispatch; keep portable fallback) | TBD — scenario 1/9 hash-throughput share | Lower verify CPU overhead on fast NVMe where disk speed ≥ hash speed; small-file storms benefit most | Unsafe-code review burden; MSRV discipline; medium effort | pending |
| C2 | 3.17 used a VSS+overlapped elevation service for locked files; **4.0.5 removed it** [B] | `--wait-lock` sharing-violation retries | None (keep retry approach); record vendor's own 4.x removal as evidence against the complexity | TBD — scenario 8 | — | — | pending |
| C3 | 3.17 engine = manual synchronous ReadFile/WriteFile loops; no CopyFileEx shortcut [B] | 3-phase pipeline, manual chunked I/O — same class | Confirmation only (no change) | TBD — scenario 1 | validates current design | — | pending |
| C4 | 4.x async/queue-depth structure unknown; service removal implies engine I/O rework [B→A] | Windows path uses the same chunked engine as Linux | IF Track A shows 4.0.5 with deeper queue depth at equal throughput: deepen the Windows overlapped pipeline in `platform/windows.rs` | TBD — scenarios 1/2/6 | Possible large-file throughput ↑ on NVMe; risks Win7 regression | High effort; needs A/B evidence first | pending |
| C5 | 4.0.5 slimmed Context.dll to 205 KB (x64) + ships a packaged-companion `.appx` [B] | Registry-based menus + Inno Setup scripts (`integrations/`, `packaging/`) | Note for a future integration stage (Win11 packaged menus); not a perf item | n/a | — | — | pending |
| C6 | 4.0.5 adds ARM64/ARM64X trees [B] | Win x86/x64 only (Win7+) | Portability roadmap note: aarch64-pc-windows-msvc CI target | n/a | — | — | pending |
| C7 | Vendor packs its GUI exe (anti-tamper) [B] | Unpacked release binaries + all-asset SHA-256 table | None (our hash-published, CI-built distribution is the release policy) | n/a | — | — | pending |
| C8 | Gettext `.po` localization, 135 locale rows in both versions [B] | English-only UI/CLI | Confirms the existing i18n roadmap candidate; no new action | n/a | — | — | pending |

**Highest-yield open questions for Track A** (in priority order):

1. 4.0.5 I/O structure vs 3.17 — overlapped/queue depth (scenarios 1, 2, 6):
   the 4.x rewrite is publicly undocumented; C4 hinges on it.
2. Many-small-files scheduling — TeraCopy's historical headline strength
   vs our pipeline (scenario 2).
3. Per-media chunk/request-size adaptation — ours is fixed 1 MiB
   default; detect whether the vendor adapts to slow media (scenario 6).
4. Verification timing — inline single-pass (our design) vs post-copy
   second pass (scenario 3); likely confirms our approach is superior as
   documented justification.

## 5. Study boundaries observed

- No decompilation, disassembly, or memory inspection was performed; no
  vendor code, strings (beyond functional version/manifest numbers),
  resources, or binaries entered this repository.
- The bootstrapper content-stream protection (§2 of the study doc) was
  mapped at the container level only and **not** circumvented; 3.17's
  binaries were obtained through the vendor's own public MSI channel,
  4.0.5's via the owner-rig export path.
- Vendor artifacts live outside the repo; their SHA-256 hashes are
  denylisted in the backup guard.
