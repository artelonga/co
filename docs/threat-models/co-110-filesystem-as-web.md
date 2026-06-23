---
type: doc
title: "CO-110 Threat Model — Filesystem-as-Web"
status: phase-1-reviewed
---

# CO-110 Threat Model — Filesystem-as-Web

> Companion to [`docs/specs/co-110-pairing-handshake.md`](../specs/co-110-pairing-handshake.md),
> [`docs/specs/co-110-frame-format.md`](../specs/co-110-frame-format.md), and
> [`docs/specs/co-110-crypto-review.md`](../specs/co-110-crypto-review.md).

## 1. System under analysis

Three principals exchange data over two hops:

```
Browser (Co page)  ⇄  Co server (/api/v1/agent/relay)  ⇄  co-agent (user PC)
   E2E keypair            opaque relay only                  E2E keypair
```

- **Browser** holds an ephemeral X25519 keypair generated per page session and renders the
  filesystem UI. It is the *initiator* of a pairing.
- **co-agent** is a long-lived daemon on the user's machine. It holds a long-term X25519
  identity keypair and exposes a path-scoped read/write filesystem surface.
- **Co server** is an authenticated **relay**. It multiplexes opaque ciphertext frames between
  a paired (browser, agent) couple, keyed by `pairing_id`. It never holds the ECDH shared
  secret and therefore cannot read or forge payloads.

**Security goal (one sentence):** the operator of the Co server — even a fully malicious one —
can deny service and observe traffic metadata, but **cannot read or alter the bytes** flowing
between a browser and the user's filesystem.

## 2. Trust boundaries

| Boundary | Crosses | Protection |
|---|---|---|
| Browser ↔ server | TLS + WebSocket | TLS for transport; **E2E ChaCha20-Poly1305 inside** so TLS termination is not trusted |
| Server ↔ agent | TLS + WebSocket | same — server sees only ciphertext |
| Browser ↔ agent (logical) | the ECDH channel | X25519 ECDH; shared secret never transmitted |
| agent ↔ disk | filesystem syscalls | path scoping to `allowed_paths.roots`, `../` rejected |

## 3. Assets

1. **File contents** flowing through the relay (highest value — must be confidential + integral).
2. **The pairing secret / ECDH shared key** (compromise ⇒ loss of confidentiality).
3. **The agent's `allowed_paths` configuration** (defines blast radius of any pairing).
4. **Pairing metadata** on the server (`pairing_id`, `page_url`, pubkeys, scope, expiry).
5. **The agent host itself** (out-of-scope to fully defend; see §6).

## 4. Adversaries & STRIDE

### 4.1 Malicious / compromised Co server operator (primary adversary)

| STRIDE | Threat | Mitigation |
|---|---|---|
| **Information disclosure** | Operator reads file contents in transit | **E2E encryption** — server holds only ciphertext + `pairing_id`. The shared key is X25519-ECDH-derived and never sent. |
| **Tampering** | Operator rewrites a `write` payload to corrupt a file | Every frame is AEAD-sealed (Poly1305 tag); a flipped byte fails authentication and is dropped by the receiver. |
| **Spoofing** | Operator injects a forged `write`/`delete` op | Forgery requires the AEAD key; without it the tag check fails. The agent rejects any frame that does not authenticate. |
| **Repudiation** | Operator denies dropping frames | Counter-nonce gaps are detectable by the receiver; the agent keeps a local salted audit log (Phase 5). |
| **DoS** | Operator drops/queues frames, refuses to relay | Accepted residual risk — a relay can always deny service. Mitigated only by the user noticing and re-pairing peer-to-peer (future). |
| **Elevation** | Operator forces a pairing it does not own | Pairings are bound to `user_id`; the relay requires the caller's JWT to match `pairing.user_id`. |

**Conclusion:** confidentiality and integrity hold against a fully malicious operator.
Availability does not — by construction a relay can stop relaying.

### 4.2 Network attacker / MITM at TLS termination

| Threat | Mitigation |
|---|---|
| Substitutes its own pubkey for the agent's during pairing (classic MITM) | The **6-word Short Authentication String** (SAS), derived from the ECDH shared secret, is shown on **both** the browser page and the agent terminal. A MITM produces *two different* shared secrets ⇒ *two different* phrases ⇒ the human notices the mismatch and aborts. This is the Signal/Wire emoji-verification pattern. |
| Replays a captured frame | Per-frame strictly-increasing counter nonce; the receiver rejects any nonce ≤ the highest seen. |
| Downgrade / version rollback | Frame header carries a version byte bound into the AEAD associated data; a rollback changes the AAD and fails the tag. |

### 4.3 Compromised / malicious Co page (XSS in content)

| Threat | Mitigation |
|---|---|
| A page without authorization mounts the filesystem | The relay only upgrades a connection whose bound page declares `mount: agent` capability **and** the pairing's `page_url` matches. Pages without the capability cannot pair. |
| Injected inline script exfiltrates the shared key | CSP forbids inline scripts; `agent-client.js` is served only from the Co origin. The key lives in a closure, never in `localStorage`. |
| A page mounts a *different* user's agent | Pairing is bound to `(page_url, browser_pubkey, agent_pubkey, user_id)`; a foreign page yields no matching pairing. |

### 4.4 Compromised browser session

| Threat | Mitigation |
|---|---|
| Stolen session continues to read files after the tab closes | The browser keypair is ephemeral and lives only in page memory; closing the tab destroys the symmetric key. |
| Replay of an old session's frames | New ECDH session keys per connection (forward secrecy via per-session ephemeral keys + ratchet); old frames do not authenticate under new keys. |

### 4.5 Agent host compromise

Out of primary scope: full control of the host means the attacker already has the filesystem.
**Defense-in-depth** still applies and bounds the damage *via the agent*:

- Least-privilege: run `co-agent` as an unprivileged user.
- **Path scoping**: only paths under `allowed_paths.roots` are ever exposed, and `../`
  traversal is rejected (canonicalize-then-prefix-check at the agent, see §5).
- Per-pairing **scope** (`read` / `write` / `read-write`) further narrows each pairing.
- Bounded expiry: every pairing has an `expires_at`; a kiosk pairing is e.g. 24h.

### 4.6 Denial of service

| Threat | Mitigation |
|---|---|
| Connection flood per user/agent | Rate limits per `user_id` and per `agent_pubkey` at the relay (CO-80 territory). |
| Oversized frame to exhaust memory | Max frame size enforced at the relay and the agent (default 1 MiB, matches the API body limit). |
| Pairing-table growth | Expired/revoked pairings are swept; `expires_at` is mandatory. |

## 5. Path-scoping invariant (the agent's core guarantee)

A request path `p` is served **iff** there exists a configured root `r` such that
`canonicalize(join(r, p))` is `r` itself or a descendant of `r`. The check is performed on the
**canonicalized** path (symlinks + `..` resolved) so that neither `../../etc/passwd` nor a
symlink escape leaks data outside a root. Any path that fails the check is rejected with a
`scope_denied` error *before* any disk read. This invariant is unit-tested in
`co-agent/src/fsmount` (`rejects_parent_traversal`, `rejects_symlink_escape`, `rejects_absolute_outside_root`).

## 6. Residual risks (accepted)

- **Availability** against a malicious relay — by construction.
- **Full agent-host compromise** — out of scope; mitigated only by least-privilege + scoping.
- **Cryptographic break of X25519 / ChaCha20-Poly1305** — out of scope; best-of-class
  primitives. Reassess on NIST guidance change (see crypto-review note).
- **User ignores the SAS mismatch** — social-engineering residual; the UI makes the phrase
  prominent and requires an explicit confirm on the terminal.

## 7. Verification matrix

| Claim | How it is verified |
|---|---|
| Server cannot decrypt | Test: relay forwards bytes; a third party with only the relayed frame + both *public* keys cannot recover plaintext (`server_sees_only_ciphertext`). |
| Tamper is detected | Test: flip one ciphertext byte ⇒ decrypt fails (`tampered_frame_rejected`). |
| Replay is rejected | Test: re-send a frame with a stale nonce ⇒ rejected (`replayed_nonce_rejected`). |
| Path traversal blocked | Tests in `co-agent/src/fsmount`. |
| MITM produces different SAS | Test: two distinct shared secrets ⇒ two distinct phrases (`sas_differs_under_mitm`). |

## Acceptance

- [ ] Threat model enumerates adversaries (operator, MITM, XSS page, browser, host, DoS)
- [ ] Each adversary mapped to STRIDE with a concrete mitigation
- [ ] Path-scoping invariant stated and tied to tests
- [ ] Residual (accepted) risks listed explicitly
- [ ] Verification matrix ties each security claim to a test
