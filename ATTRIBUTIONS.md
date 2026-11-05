# Attributions & acknowledgements

## Development assistance

KetraCopy was designed and implemented with the assistance of AI systems,
at the direction of the project owner:

| Role | System |
|---|---|
| Primary LLM | **GLM** (large language model by **Z.ai**), via the Z.ai coding-agent workflow |
| Subagents used during development | Z.ai built-in agents — general-purpose research agent, Explore (codebase exploration), Plan (architecture), frontend-styling-expert (MD3 design-system polish), full-stack-developer (project scaffolding) |
| Skill packs consulted (from skills.sh / ClawHub) | `skill-finder-cn` (skill discovery), `android-native-dev` (Material Design 3 / Android guidance), `android-project-generator` (Gradle project scaffolding), `ah-kotlin-specialist` (Kotlin review) — all MIT-0 licensed; the `wshobson/agents` and software-mansion collections were reviewed as reference material only |

All code was reviewed and tested by the contributors before release; the
AI systems are credited as development tooling, not as authors of record.

## Algorithm references (implemented from specifications)

Clean-room implementations — no original source code used. See
`docs/ALGORITHMS.md` for the full list with citations:

* SHA-256 (FIPS 180-4, NIST) — manifest & integrity digests
* MD5 (RFC 1321) — optional verify algorithm
* XXH64 (Yann Collet's xxHash specification) — fast read-test hashing
* CRC32C (Castagnoli polynomial, RFC 3720 §B.4 test vectors) — optional verify algorithm
* Token bucket (J. D. C. Turner, 1986) — transfer throttling
* AIMD (Chiu & Jain, 1989) — adaptive rate control
* mDNS/DNS-SD (RFC 6762/6763) — LAN peer discovery
* LocalSend v2 protocol (public documentation of the LocalSend project) — LAN interoperability

## Third-party dependencies

The engine (`ketra-core`) and CLI (`ketra-cli`) are **std-only** — they link
zero external crates; platform features (CF_HDROP, X11 selection ownership,
mDNS sockets, FICLONE, copy_file_range, FSCTL_SET_SPARSE) are reached
through hand-declared FFI to the operating system.

The GUI uses, per its manifest:

| Crate | Version | License |
|---|---|---|
| `egui` | 0.28.1 (pinned) | MIT OR Apache-2.0 |
| `eframe` | 0.28 (glow renderer) | MIT OR Apache-2.0 |
| `glow` (OpenGL bindings used by eframe) | via eframe | MIT OR Apache-2.0 |
| `winit`, `glutin`, `image`, and transitive GUI-stack crates | via eframe 0.28 | MIT OR Apache-2.0 |

A machine-readable inventory of every transitive crate and its license is
in `Cargo.lock` (`cargo license` on a checkout reproduces it). All GUI
dependencies are dual MIT/Apache-2.0.

`attic/` contains prototype crates (`ketra-net`, `ketra-ffi`) that
referenced `rustls`, `mdns-sd`, `rcgen` and `uniffi` during development;
they are not workspace members and are not distributed in binary form.

## Package formats

* Debian packaging follows the Debian Policy manual layout (dpkg-deb).
* Windows installer uses Inno Setup (JR Inno Setup — free for closed- and
  open-source projects).
* Flatpak manifest targets `io.github.ketracopy.KetraCopy`.

## License

KetraCopy itself is MIT-licensed (see `LICENSE`). The clean-room
methodology is documented in `docs/CLEANROOM.md`.
