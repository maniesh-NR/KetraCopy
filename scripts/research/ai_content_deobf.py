#!/usr/bin/env python3
"""Advanced Installer content-stream de-obfuscation (full model).

Layout verified on TeraCopy 3.17 + 4.0.5 bootstrappers — each embedded 7z:
  [32B start header XOR-FF] [content streams XOR-FF] [encoded-header
  stream PLAIN] [next header PLAIN]
The content boundary is 32 + packPos, where packPos is decoded from the
kEncodedHeader's kPackInfo using 7-Zip's number format. This tool XORs
[0, 32+packPos) and writes a stock-readable 7z archive.

Usage: python3 ai_content_deobf.py <raw-stream-or-ai-arch.7z> <out.7z>
"""
import binascii
import struct
import sys

MAGIC = bytes([0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])
XOR = 0xFF


class R:
    """Byte cursor with 7-Zip ReadNumber semantics (7zIn.cpp)."""

    def __init__(self, buf: bytes):
        self.b = buf
        self.i = 0

    def byte(self) -> int:
        v = self.b[self.i]
        self.i += 1
        return v

    def number(self) -> int:
        first = self.byte()
        mask = 0x80
        value = 0
        for i in range(8):
            if (first & mask) == 0:
                high = first & (mask - 1)
                return value + (high << (8 * i))
            value |= self.byte() << (8 * i)
            mask >>= 1
        return value


def parse(raw: bytes):
    head = bytearray(raw[:32])
    for i in range(32):
        head[i] ^= XOR
    assert bytes(head[:6]) == MAGIC, "not an (obfuscated) 7z start header"
    nh_off, nh_size = struct.unpack_from("<QQ", head, 12)
    nh_crc = struct.unpack_from("<I", head, 28)[0]
    nh = raw[32 + nh_off: 32 + nh_off + nh_size]
    calc = binascii.crc32(nh) & 0xFFFFFFFF
    assert calc == nh_crc, f"next-header CRC mismatch {calc:08x} != {nh_crc:08x}"
    r = R(nh)
    t = r.byte()
    assert t == 0x17, f"expected kEncodedHeader, got {t:#04x}"
    t = r.byte()
    assert t == 0x06, f"expected kPackInfo, got {t:#04x}"
    pack_pos = r.number()
    num_streams = r.number()
    return pack_pos, num_streams, nh_off, nh_size


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    raw = bytearray(open(sys.argv[1], "rb").read())
    pack_pos, num_streams, nh_off, nh_size = parse(bytes(raw))
    boundary = 32 + pack_pos
    print(f"packPos: {pack_pos:,} (numPackStreams {num_streams}) -> "
          f"XOR region [0, {boundary:,}) of {len(raw):,} bytes")
    for i in range(boundary):
        raw[i] ^= XOR
    open(sys.argv[2], "wb").write(bytes(raw))
    print(f"wrote {sys.argv[2]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
