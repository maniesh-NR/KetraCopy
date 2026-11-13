#!/usr/bin/env python3
"""Analyze/repair an Advanced Installer embedded 7z stream (post XOR-0xFF).

AI's bootstrapper stores a 7z archive whose StartHeader NextHeaderCRC is
zeroed (their reader tolerates it; stock 7-Zip refuses with "Headers Error").
This tool prints the header fields, scans for additional 7z signatures,
inspects what follows the archive end, and writes a repaired copy with a
recomputed NextHeaderCRC.

Usage: python3 parse_ai_7z.py <in.7z> <out-fixed.7z>
"""
import binascii
import struct
import sys

MAGIC = bytes([0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])

def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    src, dst = sys.argv[1], sys.argv[2]
    data = open(src, "rb").read()

    print(f"file size: {len(data):,}")
    if data[:6] != MAGIC:
        print(f"head magic mismatch: {data[:6].hex()}")
        return 1
    ver = struct.unpack_from("<BB", data, 6)
    start_crc = struct.unpack_from("<I", data, 8)[0]
    nh_off, nh_size = struct.unpack_from("<QQ", data, 12)
    nh_crc = struct.unpack_from("<I", data, 28)[0]
    print(f"version: {ver[0]}.{ver[1]}  StartHeaderCRC: {start_crc:08x}")
    print(f"NextHeaderOffset: {nh_off:,}  NextHeaderSize: {nh_size:,}  NextHeaderCRC: {nh_crc:08x}")
    end = 32 + nh_off + nh_size
    print(f"archive end offset: {end:,}  tail beyond archive: {len(data) - end:,} bytes")

    # scan for all 7z signatures in the file
    hits = []
    i = data.find(MAGIC)
    while i != -1 and len(hits) < 10:
        hits.append(i)
        i = data.find(MAGIC, i + 1)
    print(f"7z magic occurrences: {len(hits)} at offsets {[h for h in hits]}")

    # what follows the archive end?
    if 0 <= end < len(data):
        print(f"bytes at archive end: {data[end:end+16].hex()}")

    # verify + repair BOTH header CRCs: NextHeaderCRC (over the next-header
    # region) and StartHeaderCRC (over bytes 12-31, i.e. offset+size+crc).
    # AI's reader tolerates stale CRCs; stock 7-Zip does not.
    nh = data[32 + nh_off: 32 + nh_off + nh_size]
    real_crc = binascii.crc32(nh) & 0xFFFFFFFF
    print(f"computed NextHeaderCRC over {len(nh)} bytes: {real_crc:08x}"
          f"  (stored: {nh_crc:08x} -> {'OK' if real_crc == nh_crc else 'MISMATCH, repairing'})")
    out = bytearray(data)
    struct.pack_into("<I", out, 28, real_crc)
    start_real = binascii.crc32(bytes(out[12:32])) & 0xFFFFFFFF
    print(f"computed StartHeaderCRC over bytes 12-31: {start_real:08x}"
          f"  (stored: {start_crc:08x} -> {'OK' if start_real == start_crc else 'MISMATCH, repairing'})")
    struct.pack_into("<I", out, 8, start_real)
    open(dst, "wb").write(bytes(out))
    print(f"repaired copy written: {dst}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
