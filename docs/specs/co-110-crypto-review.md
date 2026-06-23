---
type: doc
title: "CO-110 Spec — Cryptographic Review Note"
status: phase-1-reviewed
---

# CO-110 — Cryptographic Review Note

> Which library, which primitives, and why. Companion to the
> [frame-format](./co-110-frame-format.md) and [pairing](./co-110-pairing-handshake.md) specs.

## 1. Decision

| Concern | Choice | Rust crate | Browser |
|---|---|---|---|
| Key agreement | **X25519 ECDH** | `x25519-dalek` | `libsodium-wrappers` `crypto_scalarmult` |
| AEAD | **ChaCha20-Poly1305** (12-byte nonce) | `chacha20poly1305` (already a dep via CO-86) | `libsodium-wrappers` / WebCrypto |
| KDF | **HKDF-SHA256** | `hkdf` + `sha2` (already in tree) | `libsodium-wrappers` `crypto_kdf` / HKDF |
| Confirm signature | **Ed25519** | `ed25519-dalek` (already a dep via CO-86) | `libsodium-wrappers` `crypto_sign` |
| SAS phrase | **HKDF → 6 words / 256-word list** | `co-agent` `wordlist.rs` | `agent-client.js` |

## 2. Why these, and the alternatives considered

### libsodium vs `age` vs MLS

- **libsodium / RustCrypto (`chacha20poly1305`, `x25519-dalek`)** — *chosen*. Rationale:
  - CO-86 already pulls `chacha20poly1305` and `ed25519-dalek`; reusing them keeps **one crypto
    core** for the `.co` envelope and this transport (the spec's explicit goal). No new audited
    surface beyond `x25519-dalek`, a widely-used, audited curve25519 implementation.
  - The browser counterpart `libsodium-wrappers` is small (the ≤80 KB-gzipped budget) and exposes
    the *exact same* primitives, so the two ends are trivially interoperable.
- **`age`** — rejected for the live transport. `age` is a *file* encryption format (one-shot,
  recipient-stanza oriented). It has no session/ratchet notion and no streaming AEAD with counter
  nonces, so it would have to be bent into a shape it wasn't built for. (It remains a fine choice
  for at-rest snapshots, e.g. the PWA fallback cache in CO-69.)
- **MLS (Messaging Layer Security, RFC 9420)** — rejected for v1 as over-scoped. MLS shines for
  **group** messaging with efficient membership changes. CO-110 v1 is a **two-party** channel
  (one browser, one agent); MLS's TreeKEM machinery is unjustified complexity. **Revisit MLS for
  Phase 5 multi-pairing** if "many browsers ⇆ one agent with shared rooms" becomes a hard
  requirement — at that point a group ratchet is the right tool.

### Why ChaCha20-Poly1305 (not AES-GCM)

- Constant-time in software without AES-NI (matters on mobile / the iPad use case).
- Same primitive as CO-86 ⇒ shared review surface.
- 96-bit counter nonce is sufficient given a **per-direction key** and a monotonic counter
  (no random-nonce collision risk).

## 3. Key-management properties

- **Server never sees a secret.** It relays *public* keys and opaque ciphertext only. The ECDH
  shared secret is computed independently at each endpoint and never transmitted.
- **Forward secrecy (v1).** The browser key is ephemeral per page session; discarding it on tab
  close makes recorded ciphertext undecryptable. Phase 5 adds a Double-Ratchet for per-message FS.
- **Nonce safety.** Per-direction HKDF-derived keys + monotonic counter nonce ⇒ a `(key, nonce)`
  pair never repeats, which is the one hard requirement for ChaCha20-Poly1305 security.
- **Downgrade resistance.** The version byte is in the AEAD associated data.
- **MITM resistance.** The 6-word SAS is HKDF(shared_secret); a substituted key changes it and the
  human aborts.

## 4. Things explicitly NOT rolled by hand

- No custom AEAD, no custom curve arithmetic, no custom KDF — only well-reviewed crates/standards.
- The only bespoke logic is **composition** (key schedule, frame framing, SAS word mapping), all
  of which is unit-tested and uses standard constructions (HKDF salt/info domain separation).

## 5. Residual crypto risk

- A break of X25519 or ChaCha20-Poly1305 is out of scope (best-of-class; reassess on NIST PQ
  guidance — a future PQ-hybrid handshake is the migration path, gated by the version byte).
- Side-channels on the agent host are bounded by least-privilege + the host-compromise carve-out
  in the threat model.

## Acceptance

- [ ] Library decision recorded (RustCrypto + libsodium-wrappers) with rationale
- [ ] `age` and MLS explicitly evaluated and dispositioned (MLS revisit deferred to Phase 5)
- [ ] Primitive choices justified (X25519 / ChaCha20-Poly1305 / HKDF-SHA256 / Ed25519)
- [ ] Key-management properties stated (no server secret, FS, nonce safety, downgrade/MITM resistance)
- [ ] Residual crypto risk + PQ migration path noted
