---
type: doc
title: "CO-110 Spec — Pairing Handshake"
status: phase-1-reviewed
---

# CO-110 Spec — Pairing Handshake (the trust ceremony)

> Frame wire format: [`co-110-frame-format.md`](./co-110-frame-format.md).
> Crypto-library choice: [`co-110-crypto-review.md`](./co-110-crypto-review.md).
> Threat model: [`../threat-models/co-110-filesystem-as-web.md`](../threat-models/co-110-filesystem-as-web.md).

A **pairing** is a `(page_url, browser_pubkey, agent_pubkey, scope, expires_at)` record bound to
a single `user_id`. It is established **once** per `(page, browser, agent)` with explicit human
consent on both the browser and the terminal, verified by a **6-word Short Authentication String
(SAS)** derived from the X25519 ECDH shared secret. After establishment the relay can multiplex
encrypted frames between the two peers.

## 1. Roles

| Role | Holds | Generates |
|---|---|---|
| **Browser** | ephemeral X25519 keypair (per page session) | the pairing *request* |
| **co-agent** | long-term X25519 identity keypair | the pairing *confirmation* |
| **Co server** | nothing secret | the `pairing_id`, relays pubkeys |

## 2. State machine

```
                 ┌──────────────────────────────────────────────────────┐
                 │                                                        │
   (browser)     ▼                                                        │
  ┌────────┐  POST /pairings   ┌──────────┐  agent pair <id>   ┌──────────┴──┐
  │  IDLE  │ ───────────────►  │ REQUESTED │ ───────────────►  │  CONFIRMING  │
  └────────┘                   └──────────┘                    └──────┬───────┘
                                    │ expires_at passed               │ user confirms (SAS match)
                                    │ (no agent claim)                ▼
                                    ▼                          ┌──────────────┐
                               ┌─────────┐   revoke / expiry   │    ACTIVE    │
                               │ EXPIRED │ ◄────────────────── │  (relayable) │
                               └─────────┘                     └──────┬───────┘
                                                                      │ revoke (either side)
                                                                      ▼
                                                                ┌──────────┐
                                                                │ REVOKED  │
                                                                └──────────┘
```

States: `REQUESTED → CONFIRMING → ACTIVE → {REVOKED | EXPIRED}` (plus the `REQUESTED → EXPIRED`
shortcut when no agent ever claims the pairing).

## 3. Happy-path sequence

1. **Visit.** Browser loads a page whose frontmatter declares `mount: { type: agent, scope: … }`.
   The SPA reads the capability and renders a **"Pair with my computer"** button. A page without
   the capability never shows it and can never pair.
2. **Request.** On click the browser generates an ephemeral X25519 keypair and calls
   `POST /api/v1/agent/pairings` with `{ page_url, browser_pubkey, scope }`. The server validates
   the caller is authenticated, the page declares `mount: agent`, and the scope is a subset of the
   page's declared scope. It creates a `REQUESTED` pairing and returns `{ pairing_id, expires_at }`.
3. **Claim.** The user runs `co-agent pair <pairing_id>` on the PC. The agent fetches the pairing
   (`GET /api/v1/agent/pairings/{id}`), reads the `browser_pubkey` + `page_url` + `scope`, and
   computes the ECDH shared secret with its own identity key. From the shared secret it derives the
   **6-word SAS** (see §5) and prints:

   ```
   Pair "co.artelonga.com.br/co/artelonga" — scope: read-write, expires in 30 days.
   Verify this phrase matches the one on the web page:

       harbor · cedar · violet · anchor · meadow · ember

   Allow this page to mount your filesystem? [y/N]
   ```

4. **Verify (the ceremony).** The same 6-word phrase is shown on the page (the browser derived it
   from *its* shared secret). The human checks that **both halves match**. A MITM that swapped a
   pubkey yields two *different* phrases — the mismatch is the abort signal.
5. **Confirm.** The user types `y`. The agent `PUT /api/v1/agent/pairings/{id}/confirm` with its
   `agent_pubkey` + a detached Ed25519 signature over `(pairing_id ‖ browser_pubkey ‖ scope ‖ expires_at)`.
   The server moves the pairing to `ACTIVE`.
6. **Relay.** Either peer connects `GET /api/v1/agent/relay?pairing_id=…&direction=browser|agent`
   and the server multiplexes opaque frames between them (see frame-format spec).

## 4. Failure modes

| # | Failure | Detection | Resolution |
|---|---|---|---|
| F1 | Page lacks `mount: agent` | server at `POST /pairings` | `403 mount_not_allowed`; button never rendered |
| F2 | Requested scope exceeds page scope | server at `POST /pairings` | `400 scope_too_broad` |
| F3 | Caller not authenticated | server (JWT/cookie missing) | `401` |
| F4 | Agent claims a pairing it doesn't own (wrong user) | server at `GET`/`confirm` (pairing.user_id ≠ caller) | `404` (existence hidden) |
| F5 | **SAS mismatch** (MITM) | human, step 4 | user answers `N`; agent never confirms; pairing expires |
| F6 | User declines on terminal | agent | pairing stays `REQUESTED`, expires |
| F7 | `expires_at` passed before confirm | server / agent | `410 pairing_expired`; re-request |
| F8 | Confirm signature invalid | server at `/confirm` | `400 bad_signature`; pairing stays `REQUESTED` |
| F9 | Peer not connected at relay | relay | frame queued ≤ 60 s, then `peer_absent` close |
| F10 | Pairing revoked mid-session | relay close frame | both sockets closed within 5 s |
| F11 | Replayed confirm | server (pairing already `ACTIVE`/`REVOKED`) | idempotent no-op / `409` |
| F12 | Clock skew on expiry | server is authoritative on time | agent trusts server `expires_at`, displays it |

## 5. SAS derivation (the 6-word phrase)

Both peers independently compute, from the **same** ECDH shared secret `s`:

```
sas_bytes = HKDF-SHA256(ikm = s, salt = "co-110-sas", info = pairing_id)  [take 6 bytes]
phrase    = wordlist[sas_bytes[0]] · … · wordlist[sas_bytes[5]]
```

- `wordlist` is a fixed **256-word** list (one word per byte value) shipped identically in the
  Rust agent (`co-agent/src/fsmount/wordlist.rs`) and the browser (`agent-client.js`). 256 words
  ⇒ 48 bits of SAS entropy — a MITM must guess 1 in 2⁴⁸.
- The phrase is purely a *human* comparison artifact; it is **not** a key. Even if observed it
  reveals nothing about `s` (HKDF is one-way).
- Binding `info = pairing_id` ensures two concurrent pairings never collide on the same phrase.

## 6. Why this is safe

- The shared secret `s` is never transmitted; the server only relays *public* keys. ECDH means
  only the two endpoints can compute `s`.
- A MITM swapping a pubkey necessarily produces a different `s` on at least one side ⇒ different
  SAS ⇒ caught by the human (§4 F5).
- The Ed25519 confirm signature binds the agent's acceptance to the exact `(browser_pubkey, scope,
  expires_at)` it saw, so the server cannot later alter scope/expiry without invalidating it.

## Acceptance

- [ ] Full pairing state machine specified (`REQUESTED → CONFIRMING → ACTIVE → REVOKED/EXPIRED`)
- [ ] Happy-path handshake sequence specified end to end
- [ ] Failure modes F1–F12 enumerated with detection + resolution
- [ ] SAS (6-word phrase) derivation specified and shown identical on both sides
- [ ] Confirm step binds agent acceptance via detached signature
