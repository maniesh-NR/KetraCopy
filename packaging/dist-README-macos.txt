KetraCopy for macOS — command-line build
========================================

This archive contains the `ketra` CLI, built natively for this Mac
architecture. It includes the full engine: copy/move with inline & manifest
verification, resume journals, sparse handling, LAN transfer
(`ketra peers | send | receive`) and clipboard integration
(`ketra clip put|get|paste` via AppleScript).

The Material-3 GUI (`ketracopy-gui`) is NOT included in 1.2.x — it needs a
GUI-stack refresh for modern Rust on macOS. Build it from source on a Mac:

    git clone https://github.com/maniesh-NR/KetraCopy && cd KetraCopy
    cargo build --release -p ketra-gui    # then run target/release/ketracopy-gui

Quick start:

    ./ketra copy <src> -d <dst> --verify inline
    ./ketra manifest --root <dir>     # write a SHA-256 manifest
    ./ketra verify --root <dir>       # detect bit rot

First run may require right-click > Open (unsigned binary).
License: MIT. Attributions: ATTRIBUTIONS.md. Docs: docs/.
