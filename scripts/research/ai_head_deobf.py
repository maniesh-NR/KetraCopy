#!/usr/bin/env python3
"""De-obfuscate Advanced Installer's embedded 7z payload (head-only model).

Verified on TeraCopy 3.17 / 4.0.5 bootstrappers: the AI stream contains one
or more 7z archives back-to-back. ONLY each archive's 32-byte StartHeader is
XOR-0xFF obfuscated (magic, version, CRCs, offsets); the packed bodies and
next headers are plain. This tool:
  1. de-XORs each StartHeader (first 32 bytes only),
  2. verifies NextHeaderCRC/StartHeaderCRC against the PLAIN body,
  3. splits concatenated archives at each header's computed end offset,
  4. writes arch1.7z, arch2.7z, ... and reports any trailing bytes.

Usage: python3 ai_head_deobf.py <raw-stream> <out-prefix>
"""
import binascii
import struct
import sys

MAGIC = bytes([0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])
XOR = 0xFF

def deobf_head(buf: bytearray) -> None:
    for i in range(32):
        buf[i] ^= XOR

def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    src, prefix = sys.argv[1], sys.argv[2]
    data = bytearray(open(src, "rb").read())

    idx, arch = 0, 0
    while idx + 32 <= len(data):
        head = bytearray(data[idx: idx + 32])
        deobf_head(head)
        if bytes(head[:6]) != MAGIC:
            print(f"[{idx:,}] no 7z start header after de-XOR — stop "
                  f"({len(data) - idx:,} trailing bytes: "
                  f"{bytes(data[idx:idx+12]).hex()} ...)")
            break
        nh_off, nh_size = struct.unpack_from("<QQ", head, 12)
        nh_crc = struct.unpack_from("<I", head, 28)[0]
        sh_crc = struct.unpack_from("<I", head, 8)[0]
        end_rel = 32 + nh_off + nh_size
        end_abs = idx + end_rel
        if end_abs > len(data):
            print(f"[{idx:,}] header claims end {end_abs:,} beyond file — stop")
            break
        # CRC check over the PLAIN body bytes
        nh_bytes = bytes(data[idx + 32 + nh_off: end_abs])
        calc_nh = binascii.crc32(nh_bytes) & 0xFFFFFFFF
        calc_sh = binascii.crc32(bytes(head[12:32])) & 0xFFFFFFFF
        print(f"archive #{arch + 1} @ {idx:,}: size {end_rel:,}, "
              f"NextHeaderCRC stored {nh_crc:08x} calc {calc_nh:08x} "
              f"({'OK' if calc_nh == nh_crc else 'BAD'}), "
              f"StartHeaderCRC stored {sh_crc:08x} calc {calc_sh:08x} "
              f"({'OK' if calc_sh == sh_crc else 'BAD'})")
        out = bytearray(data[idx: end_abs])
        out[:32] = head
        name = f"{prefix}-arch{arch + 1}.7z"
        open(name, "wb").write(bytes(out))
        print(f"  wrote {name} ({len(out):,} bytes)")
        idx = end_abs
        arch += 1
    print(f"done: {arch} archive(s) extracted, consumed {idx:,} of {len(data):,} bytes")
    return 0 if arch else 1

if __name__ == "__main__":
    sys.exit(main())
