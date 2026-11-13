KetraCopy — binary bundle
=========================

`ketra`            the command-line tool: copy/move/test with inline or
                   manifest verification, resume journals, speed limits,
                   conflict policies, filters, bench, LAN transfer
                   (peers/send/receive), clipboard (put/get/paste) and
                   shell integration.
`ketracopy-gui`    the Material Design 3 desktop app (where included).

Quick start:

    ./ketra copy <src> -d <dst> --verify inline
    ./ketra manifest --root <dir>      # write a SHA-256 manifest
    ./ketra verify --root <dir>        # detect bit rot
    ./ketra --help

Docs: README.md, CHANGELOG.md, SHELL-INTEGRATION.md, docs/.
License: MIT (LICENSE). Attributions: ATTRIBUTIONS.md.
