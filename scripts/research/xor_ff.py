#!/usr/bin/env python3
"""XOR a file with 0xFF (Advanced Installer bootstrapper stream de-obfuscation).

Usage: python3 xor_ff.py <in> <out>
Reads <in>, writes <out> with every byte XORed by 0xFF.
Chosen for stream `[0]` extracted from TeraCopy 3.17/4.0.5 bootstrappers:
their embedded payload is a 7-Zip archive with the header magic and body
XORed by 0xFF (verified: 37 7A BC AF 27 1C ^ FF = c8 85 43 50 d8 e3).
"""
import sys

def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    src, dst = sys.argv[1], sys.argv[2]
    with open(src, "rb") as f, open(dst, "wb") as g:
        while True:
            chunk = f.read(1 << 20)
            if not chunk:
                break
            g.write(bytes(b ^ 0xFF for b in chunk))
    return 0

if __name__ == "__main__":
    sys.exit(main())
