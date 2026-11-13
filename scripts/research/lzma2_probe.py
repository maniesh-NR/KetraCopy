#!/usr/bin/env python3
"""Probe: directly LZMA2-decode a 7z archive's content streams.

Reads the packed-streams region starting right after the 32-byte start
header and feeds it to Python's LZMA2 raw decoder, counting how many bytes
decode before an error/end. LZMA2 carries its own props in-band (control
0x91-style chunks), so no external props are needed.

Usage: python3 lzma2_probe.py <file.7z>
"""
import sys
import lzma


def probe(path: str) -> int:
    data = open(path, "rb").read()
    # skip the 32-byte 7z start header
    stream = data[32:]
    dec = lzma.LZMADecompressor(format=lzma.FORMAT_RAW,
                                filters=[{"id": lzma.FILTER_LZMA2}])
    total = 0
    pos = 0
    CHUNK = 1 << 20
    try:
        while pos < len(stream) and not dec.eof:
            buf = stream[pos: pos + CHUNK]
            out = dec.decompress(buf)
            total += len(out)
            pos += len(buf)
        print(f"decoded {total:,} bytes from {pos:,} input bytes "
              f"(eof={dec.eof})")
    except lzma.LZMAError as e:
        print(f"decoded {total:,} bytes, then LZMAError after consuming "
              f"~{pos:,} input bytes: {e}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    sys.exit(probe(sys.argv[1]))
