#!/usr/bin/env python3
"""PE functional inventory for the TeraCopy reference study (Track B, S10).

Per binary, prints: architecture/machine, bits, subsystem, timestamp,
section summary, imported DLLs, I/O- and hashing-relevant imported API
names, and the numeric FileVersion. Per S10 guardrails: functional
interface facts only — no vendor strings/copyright text are emitted.

Usage: python3 pe_inv.py <file> [file ...]
"""
import sys
import datetime

try:
    import pefile
except ImportError:
    sys.exit("pip install pefile first")

IO_KEYS = ("ReadFile", "WriteFile", "CopyFile", "MoveFile", "TransmitFile",
           "CreateFile", "Overlapped", "Completion", "DeviceIoControl",
           "FlushFileBuffers", "SetFilePointer", "SetFileValidData",
           "SetEndOfFile", "GetFileSizeEx", "Scatter", "Gather",
           "SetFileInformationByHandle", "CancelIo", "LockFileEx",
           "OpenFileById", "ReOpenFile")
HASH_KEYS = ("BCrypt", "Crypt", "Hash")


def fmt_ts(ts: int) -> str:
    try:
        return datetime.datetime.fromtimestamp(ts, datetime.timezone.utc).strftime("%Y-%m-%d")
    except (OverflowError, OSError, ValueError):
        return "?"


def inventory(path: str) -> None:
    print(f"### {path}")
    try:
        pe = pefile.PE(path, fast_load=True)
    except pefile.PEFormatError as e:
        print(f"  not a PE: {e}")
        return
    mach = pe.FILE_HEADER.Machine
    bits = 64 if pe.OPTIONAL_HEADER.Magic == 0x20B else 32
    names = {0x14C: "x86", 0x8664: "x64", 0xAA64: "ARM64", 0x1C0: "ARM"}
    subs = {2: "GUI", 3: "CUI"}
    print(f"  machine: {names.get(mach, hex(mach))} ({bits}-bit), "
          f"subsystem: {subs.get(pe.OPTIONAL_HEADER.Subsystem, '?')}, "
          f"compiled: {fmt_ts(pe.FILE_HEADER.TimeDateStamp)}")
    secs = []
    for s in pe.sections:
        try:
            nm = s.Name.rstrip(b"\x00").decode(errors="replace")
        except Exception:
            nm = "?"
        secs.append(f"{nm or '(unnamed)'}={max(s.SizeOfRawData // 1024, 1) if s.SizeOfRawData else 0}K")
    print(f"  sections: {', '.join(secs)}")
    pe.parse_data_directories(directories=[
        pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"],
        pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_EXPORT"],
        pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT"],
        pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_RESOURCE"],
    ])
    try:
        pe.parse_data_directories(directories=[
            pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_RESOURCE"]])
    except Exception:
        pass
    if hasattr(pe, "FileInfo") and pe.FileInfo:
        for finfo in pe.FileInfo:
            for entry in finfo:
                if entry.Key == b"FixedFileInfo":
                    ff = entry.Value
                    print(f"  FixedFileVersion: {(ff.ProductVersionMS >> 16)}."
                          f"{ff.ProductVersionMS & 0xFFFF}."
                          f"{ff.ProductVersionLS >> 16}.{ff.ProductVersionLS & 0xFFFF}")
                    break
    for tag in ("DIRECTORY_ENTRY_IMPORT", "DIRECTORY_ENTRY_DELAY_IMPORT"):
        entries = getattr(pe, tag, None)
        if not entries:
            continue
        label = "imports" if tag.endswith("IMPORT") else "delay-imports"
        print(f"  {label}:")
        for e in entries:
            dll = e.dll.decode(errors="replace")
            funcs = []
            by_ord = 0
            for imp in e.imports:
                if imp.name:
                    funcs.append(imp.name.decode(errors="replace"))
                else:
                    by_ord += 1
            hits = [f for f in funcs
                    if any(k in f for k in IO_KEYS + HASH_KEYS)]
            note = f", {by_ord} by-ordinal" if by_ord else ""
            print(f"    {dll} ({len(funcs)}{note}): "
                  f"{', '.join(hits) if hits else '(no I/O-hash-relevant imports)'}")
    if hasattr(pe, "DIRECTORY_ENTRY_EXPORT") and pe.DIRECTORY_ENTRY_EXPORT:
        try:
            exp = [s.name.decode(errors="replace")
                   for s in pe.DIRECTORY_ENTRY_EXPORT.symbols if s.name][:6]
            print(f"  exports (first 6): {', '.join(exp)}")
        except Exception as e:
            print(f"  exports: unreadable ({e})")
    pe.close()
    print()


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    for p in sys.argv[1:]:
        inventory(p)
    return 0


if __name__ == "__main__":
    sys.exit(main())
