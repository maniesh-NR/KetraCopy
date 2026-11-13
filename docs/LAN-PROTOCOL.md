# KetraCopy LAN Protocol

Wire-format specification for the LAN transfer feature implemented in
`crates/ketra-core/src/lan`. The protocol is JSON over **HTTP/1.1** on the
canonical LocalSend port, designed to be API-compatible with the open
**LocalSend v2** protocol while adding a Ketra-specific accelerated
extension that engages only when both endpoints are KetraCopy. (HTTPS —
required by the official LocalSend apps — is a planned transport upgrade;
see "TLS (planned)" below.)

## Overview

KetraCopy speaks the LocalSend v2 REST surface over plain HTTP/1.1:
Ketra-to-Ketra and plain-HTTP clients (`curl`, scripts) work out of the
box; official LocalSend apps require HTTPS and will interoperate once the
planned TLS transport lands. When
two KetraCopy peers discover each other, they negotiate a **Ketra accelerated
extension** that adds:

- parallel ranged upload streams (controlled by the same AIMD throttle used
  by the on-disk engine),
- per-chunk integrity checksums (`sha256` / `xxh64` / `crc32c`, the same
  primitives implemented in `ketra-core::hash`),
- `Range:`-based resume on interrupted chunks, and
- a Phase-3 manifest exchange so the receiver ends up with the same
  `<dest>/.ketra/manifest.jsonl` the desktop CLI writes — enabling
  cross-device long-term bit-rot detection.

When the remote peer is a plain LocalSend app, KetraCopy falls back to the
sequential single-stream upload path with no chunk checksums. The wire
format in that case is exactly LocalSend v2.

## Discovery

Peers find each other via mDNS (multicast DNS-SD). The service type is
`_localsend._tcp.local.` and the canonical port is **53317** (the LocalSend
constant; `ketra_net::LOCALSEND_PORT`).

TXT record fields:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `alias` | string | yes | Human-friendly device name shown in the peer list. |
| `version` | integer | yes | LocalSend protocol version. Always `2`. |
| `deviceModel` | string | no | Free-form model string ("Pixel 8", "ThinkPad T14"). |
| `deviceType` | string | yes | One of `mobile`, `desktop`, `web`, `server`, `headless`. |
| `fingerprint` | string | yes | Device fingerprint / identity hash (see TLS (planned) for the cert-based design). |
| `port` | integer | yes | Port the receiver listens on. Default `53317`. |
| `download` | string | no | `"true"` if the device accepts incoming files. Default `"true"`. |
| `ketra` | string | no | `"1"` if the peer supports the Ketra accelerated extension, `"0"` otherwise. Absent = `0`. |

The peer's IP is learned from the mDNS A/AAAA record. The full
`DeviceInfo` (including the Ketra extension block) is fetched lazily via
`GET /api/localsend/v2/info` when the user picks a peer — the TXT record is
intentionally a small subset.

KetraCopy advertises `ketra=1` only when it is willing to accept accelerated
transfers; the field is omitted (or `"0"`) when the user has disabled the
extension in settings.

## Implementation status (1.2.1)

Ketra-to-Ketra transfers, discovery, and the REST endpoints in this
document are fully implemented and exercised end-to-end (loopback:
discover → send → receive → SHA-256 verify). The transport is plain
HTTP/1.1 on the canonical port 53317; sessions are protected by
one-time upload tokens. Strict LocalSend v2 compatibility requires
HTTPS with self-signed certificates — that transport upgrade is on the
roadmap (see `attic/ketra-net/src/tls.rs` for the prepared rustls
design). Tools that expect HTTPS (a real LocalSend peer) will not
connect to a 1.1.x receiver yet; `curl` works with
`http://127.0.0.1:53317/api/localsend/v2/info`.

## TLS (planned — `attic/ketra-net` prototype design; NOT shipped in 1.2.x)

Everything from this section down describes the prepared TLS design; none
of it is in the shipped 1.2.x receiver (plain HTTP/1.1 above).

All LAN traffic is HTTPS (HTTP/1.1 over TLS 1.2 or 1.3) on port 53317. The
TLS stack is **rustls** (no OpenSSL, no platform native trust store); the
self-signed certificate is generated once per device using `rcgen` with an
**Ed25519** keypair. The certificate's `CommonName` is the device alias.

Trust model: there is no CA. Each device displays its **fingerprint**, the
hex SHA-256 of its certificate's SubjectPublicKeyInfo (`SPKI`). The human
confirms the receiver's fingerprint visually before the transfer; the
sender's TLS client then **pins** that fingerprint and refuses any peer
whose presented cert does not hash to it. The pinning logic lives in
`ketra_net::tls::PinningVerifier`.

```
fingerprint = lower( hex( SHA-256( SubjectPublicKeyInfo ) ) )
```

- Server config: `tls::server_config(cert_pem, key_pem)` presents the
  self-signed cert with no client auth.
- Client config (after pinning): `tls::client_config_pinned(expected)`
  accepts only the pinned cert; all other peers are rejected with
  `rustls::Error::General("peer fingerprint mismatch: ...")`.
- Client config (probe): `tls::client_config_unverified_for_probe()` uses a
  `NoVerifier` that accepts any cert. **This is used only for the initial
  `GET /info` probe**, never for file transfer.

Optional AES-256-GCM PIN encryption of the payload (LocalSend's
`/prepare-upload` carries a `pin` field) is **not yet implemented**; the
field is parsed if present but ignored. Tracked as a TODO in
`attic/ketra-net/src/server.rs` (prototype).

## REST endpoints

All endpoints are under the path prefix `/api/localsend/v2/` on port 53317.
Request and response bodies are JSON (`application/json; charset=utf-8`)
unless noted. In the shipped 1.2.x receiver all requests are plain
HTTP/1.1; the HTTPS behavior described in the TLS sections is the planned
design.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/localsend/v2/info` | Receiver's identity (no body needed). Probe only. |
| `POST` | `/api/localsend/v2/register` | Sender announces; receiver responds with its info + accept/decline. |
| `POST` | `/api/localsend/v2/prepare-upload` | Sender describes files; receiver returns per-file accept/decline + `sessionId`. |
| `POST` | `/api/localsend/v2/upload?sessionId=&fileId=&token=` | Sender streams one file's body. Accelerated mode adds `Range:` and `X-Ketra-Checksum:`. |
| `POST` | `/api/localsend/v2/cancel?sessionId=` | Either side cancels the session. |

### `GET /api/localsend/v2/info`

No request body. Response:

```json
{
  "alias": "Alice",
  "version": 2,
  "deviceModel": "Pixel 8",
  "deviceType": "mobile",
  "fingerprint": "ab12cd34...",
  "port": 53317,
  "download": true,
  "ketra": {
    "accelerated": true,
    "checksum": "sha256",
    "parallelStreams": 4,
    "resumeSupported": true,
    "manifestExchange": true
  }
}
```

The `ketra` block is omitted by plain LocalSend peers. KetraCopy always
includes it. The fingerprint returned here **must** match the SHA-256 of
the cert SPKI presented in the TLS handshake (it is the trust anchor).

### `POST /api/localsend/v2/register`

Sender announces its identity to a specific receiver. Request body:

```json
{ "info": { /* same shape as /info */ } }
```

Response: the receiver's own `DeviceInfo` plus a top-level `accept` boolean
(not used by KetraCopy, which always accepts and relies on the human
confirmation step).

### `POST /api/localsend/v2/prepare-upload`

Sender describes the files it wants to send. Request body:

```json
{
  "info": { /* sender's DeviceInfo */ },
  "files": {
    "1": {
      "id": "1",
      "fileName": "report.pdf",
      "size": 1234567,
      "fileType": "file",
      "sha256": "abcd...optional whole-file hash",
      "ketra": { "chunkSize": 4194304 }
    }
  }
}
```

`fileType` is one of `file`, `image`, `video`, `text`, `audio`, `other`.
`sha256` and `ketra.chunkSize` are optional; `chunkSize` advertises the
byte size the sender will use for chunked uploads (0 = sequential).

Response:

```json
{
  "sessionId": "b3f0c1a2-...",
  "files": { "1": "accept", "2": "decline" },
  "ketra": {
    "accelerated": true,
    "checksum": "sha256",
    "parallelStreams": 4,
    "resumeSupported": true,
    "manifestExchange": true
  }
}
```

`files` is a flat map of `id -> "accept" | "decline"`. The optional `ketra`
block finalizes the transfer mode: when `accelerated` is `true` on **both**
peers, the session uses the Ketra extension; otherwise the session falls
back to plain sequential upload. The `ketra` block is omitted entirely
when the receiver is not KetraCopy.

### `POST /api/localsend/v2/upload?sessionId=<id>&fileId=<id>&token=<tok>`

Streams one file's body. Query parameters:

- `sessionId` — the session id returned by `prepare-upload`.
- `fileId` — the id from the prepare-upload `files` map.
- `token` — optional session token (LocalSend optional; KetraCopy does not
  require it).

In **plain LocalSend mode** the body is the raw file content, sent
sequentially in one request. The receiver streams it to disk.

In **Ketra accelerated mode** the request carries additional headers:

- `Range: bytes=START-END` — inclusive byte range of this chunk within the
  file. Multiple concurrent `POST /upload` requests with disjoint ranges
  may be in flight (up to `parallelStreams`). The receiver writes each
  range at the correct offset.
- `X-Ketra-Checksum: <hex>` — checksum of the chunk body, using the
  algorithm negotiated in `prepare-upload` (`sha256` / `xxh64` /
  `crc32c`). The receiver recomputes the checksum on the received bytes
  and rejects the chunk with HTTP 422 if it does not match. A mismatched
  chunk is retried by the sender (up to the engine's retry budget).
- `Content-Range` (alternative) — accepted as an alias for `Range` for
  interop with HTTP-aware clients.

When `resumeSupported` is `true` the receiver tolerates a chunk that
arrives after a previous failure: the sender re-issues the same `Range`
and the receiver overwrites the affected bytes. The file is only marked
complete when all declared ranges have been written and the (optional)
whole-file `sha256` from `prepare-upload` verifies.

### `POST /api/localsend/v2/cancel?sessionId=<id>`

Either side may POST to this endpoint to abort a session. The receiver
deletes any partial file(s) for that session. No body required; an empty
`200 OK` is returned.

## Ketra extension

The Ketra accelerated extension engages when **both** peers report
`ketra.accelerated == true`. The negotiation is implicit in the
`prepare-upload` response: if the receiver echoes back a `ketra` block
with `accelerated: true`, the sender switches to the parallel chunked
upload mode; otherwise it falls back to sequential LocalSend v2.

Extension block (lives under the top-level `ketra` key in both `/info`
responses and `/prepare-upload` responses):

| Field | Type | Default | Meaning |
|---|---|---|---|
| `accelerated` | bool | false | True when the peer supports parallel ranged upload + chunk checksums. |
| `checksum` | string | `"sha256"` | One of `"sha256"`, `"xxh64"`, `"crc32c"`. Used for `X-Ketra-Checksum`. |
| `parallelStreams` | integer | 1 | Max concurrent `/upload` requests the receiver will accept. 1..=8. |
| `resumeSupported` | bool | false | Receiver tolerates `Range:`-based resume on `/upload`. |
| `manifestExchange` | bool | false | Receiver wants a final `KetraCopy-manifest.jsonl` pseudo-file uploaded. |

When `manifestExchange` is `true`, the sender uploads a final pseudo-file
named **`KetraCopy-manifest.jsonl`** as the last `fileId` of the session.
The body is a newline-delimited JSON stream with one record per sent file:

```json
{"t":"f","p":"report.pdf","s":1234567,"sha256":"abcd..."}
{"t":"f","p":"photos/cat.jpg","s":2048576,"sha256":"ef01..."}
```

The receiver writes this verbatim to `<dest>/.ketra/manifest.jsonl` so
that `ketra verify` (the desktop CLI) and the Android app's bit-rot
detection see the same shape they would for a local copy. The `t`, `p`,
`s`, `sha256` keys match the schema produced by
`ketra_core::manifest::write`.

The accelerated path uses the same AIMD chunk-size controller and shared
token-bucket throttle as the on-disk engine: each parallel stream draws
from one per-task `throttle::Shared`, so the aggregate rate equals the
configured speed limit even with multiple concurrent chunks.

## Interop

Against a plain LocalSend peer (no `ketra` block in `/info` or
`/prepare-upload`), KetraCopy:

- advertises its own `ketra` block but does not require the peer to
  understand it (extra JSON fields are ignored by compliant LocalSend
  apps),
- sends files sequentially via plain `POST /upload` with no `Range:` or
  `X-Ketra-Checksum:` headers,
- skips the manifest-exchange pseudo-file,
- still honors the receiver's `sha256` whole-file hash in
  `prepare-upload` `files[id]` if present.

The result is full interop: a KetraCopy sender can push files to an
iPhone running LocalSend, and a Mac running LocalSend can push files to
an Android device running KetraCopy. The accelerated extension is purely
additive.

## Security notes

- The **fingerprint is the only trust anchor.** There is no CA, no chain,
  no revocation. Humans must compare the displayed fingerprints before
  allowing a transfer; the TLS client enforces the pin after that.
- The `GET /info` probe uses an unverified TLS config (`NoVerifier`) so
  that discovery works before the human has picked a peer. **The probe is
  never used for file transfer.** Only after the user confirms a peer's
  fingerprint does the sender construct a pinned `ClientConfig` and
  proceed to `prepare-upload` / `upload`.
- mDNS is plaintext by design (it is a discovery protocol). The
  fingerprint in the TXT record is matched against the cert presented in
  the subsequent HTTPS handshake; a spoofed TXT record that does not
  match the attacker's cert SPKI is rejected at the TLS layer.
- The optional PIN (`prepare-upload.pin`) would encrypt the payload
  end-to-end with AES-256-GCM; this is **not yet implemented** and the
  field is ignored. Until it ships, transfers rely solely on TLS for
  confidentiality.

## References

- LocalSend protocol repository and v2 schema:
  https://github.com/localsend/protocol
- FIPS 180-4, Secure Hash Standard (SHA-256):
  https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf
- RFC 8446, TLS 1.3 (rustls default): https://www.rfc-editor.org/rfc/rfc8446
- RFC 6763, DNS-Based Service Discovery (mDNS-SD):
  https://www.rfc-editor.org/rfc/rfc6763
