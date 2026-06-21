---
type: doc
title: "CO-110 Spec — Frame Format"
status: phase-1-reviewed
---

# CO-110 Spec — Frame Format (versioned header + ciphertext)

> Pairing: [`co-110-pairing-handshake.md`](./co-110-pairing-handshake.md).
> Crypto choice: [`co-110-crypto-review.md`](./co-110-crypto-review.md).

Every byte the relay forwards is one **frame**. The relay treats a frame as an opaque blob keyed
only by the WebSocket's `pairing_id` query param — it never parses the body. The format below is
agreed **between the two peers** (browser ⇆ agent); it reuses the CO-86 AEAD primitive
(ChaCha20-Poly1305) so the `.co` envelope and this transport share one crypto core.

## 1. Layout

```
 0        1            13                       13+N
 ┌────────┬────────────┬────────────────────────┐
 │ ver(1) │ nonce(12)  │     ciphertext (N)      │
 └────────┴────────────┴────────────────────────┘
   └── cleartext header (AAD) ──┘ └─ AEAD sealed ─┘
```

| Field | Bytes | Meaning |
|---|---|---|
| `ver` | 1 | Frame format version. `0x01` = this spec. |
| `nonce` | 12 | ChaCha20-Poly1305 nonce. **Counter nonce**: big-endian `u96`, strictly increasing per direction per session. |
| `ciphertext` | N | AEAD seal of the plaintext **payload** (§3). Includes the 16-byte Poly1305 tag. |

- **Associated data (AAD)** = the 13-byte cleartext header (`ver ‖ nonce`). Binding the version
  byte into the AAD makes a version-rollback fail authentication.
- `MAX_FRAME = 1 MiB` (matches the API body limit). Larger payloads are chunked at the payload
  layer (§3, `seq`/`eof`).

## 2. Key schedule

```
s          = X25519(my_secret, peer_public)            # ECDH shared secret (never on server)
session_key = HKDF-SHA256(ikm = s, salt = "co-110-frame", info = pairing_id ‖ direction)  [32 bytes]
```

- `direction ∈ {"b2a","a2b"}` so the two directions use **independent** keys (no nonce reuse
  across directions).
- Per the threat model, a new ephemeral browser key per page session gives **forward secrecy**:
  closing the tab discards the key and past frames cannot be re-derived. A future Double-Ratchet
  (Phase 5) adds per-message ratcheting; v1 uses one session key + counter nonce, which is safe as
  long as nonces never repeat under a key (guaranteed by the monotonic counter + per-direction key).

## 3. Payload (plaintext inside the AEAD)

The sealed plaintext is a small JSON object (compact; large file bodies are base64 in `data`,
chunked when needed):

```jsonc
{
  "id":   "req_7Hk2",        // correlation id (request ↔ response)
  "op":   "read",            // list | read | write | delete | move | watch | ack | event | error
  "path": "content/sobre.md",// relative to the negotiated root (agent re-scopes)
  "to":   null,              // destination path for `move`
  "data": "…base64…",        // bytes for read/write payloads
  "seq":  0,                 // chunk index (for payloads > MAX_FRAME)
  "eof":  true,              // last chunk
  "mtime": 1718900000,       // for ack / conflict detection
  "err":  null               // set on op="error": { code, message }
}
```

### Operation summary

| `op` | Direction | Request fields | Response (`op`) |
|---|---|---|---|
| `list` | b→a | `path` | `ack` with `data` = JSON tree slice |
| `read` | b→a | `path` | `ack` with `data` = base64 bytes, `mtime` |
| `write` | b→a | `path`, `data`, `mtime?` | `ack` with new `mtime` (or `error` `conflict`) |
| `delete` | b→a | `path` | `ack` |
| `move` | b→a | `path`, `to` | `ack` |
| `watch` | b→a | `path` | stream of `event` frames |
| `event` | a→b | `path`, change kind | (no response) |
| `ack` | a→b | per-op | terminal |
| `error` | a→b | `err{code,message}` | terminal |

## 4. Versioning policy

- The leading `ver` byte allows additive evolution. A peer that receives a `ver` it does not
  support replies with `error { code: "unsupported_version" }` and closes.
- Because `ver` is in the AAD, an attacker cannot strip/downgrade it without breaking the tag.
- New ops are additive within `ver = 1`; removing/changing an op's shape requires `ver = 2`.

## 5. Replay & ordering

- The receiver tracks the highest nonce seen **per direction**; any frame with `nonce ≤ highest`
  is dropped (replay / reorder protection).
- Counter nonces never repeat under a session key (monotonic), satisfying ChaCha20-Poly1305's
  nonce-uniqueness requirement.

## 6. What the server sees

For any frame the relay can observe **only**: the WebSocket `pairing_id`, the frame length, the
1-byte `ver`, and the 12-byte `nonce`. The `op`, `path`, and `data` are inside the AEAD seal and
are unreadable without `session_key`, which the server never possesses. This is asserted by the
`server_sees_only_ciphertext` test.

## Acceptance

- [ ] Frame layout specified (version byte + nonce + AEAD ciphertext) and versioned
- [ ] Version byte bound into AEAD associated data (downgrade-resistant)
- [ ] Per-direction key schedule specified (HKDF over the ECDH secret, no cross-direction nonce reuse)
- [ ] Payload op-set specified (list/read/write/delete/move/watch/ack/event/error)
- [ ] Replay/ordering rule specified (monotonic counter nonce)
- [ ] "What the server sees" enumerated and tied to a test
