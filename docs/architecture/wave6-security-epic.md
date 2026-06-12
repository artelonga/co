# Wave 6 — Security epic: `.co` format + filesystem-as-web + encryption envelope

**Status:** design (decisions, not options) — CO-402
**Targets:** Wave 6 (post-v3.5.0; see [`../roadmap.md`](../roadmap.md))
**Source specs:** [CO-86](../../work/co/CO-86.md) `.co` format · [CO-87](../../work/co/CO-87.md) protocol stack · [CO-110](../../work/co/CO-110.md) filesystem-as-web · [CO-145](../../work/co/CO-145.md) encrypted assets · [CO-148](../../work/co/CO-148.md) encryption envelope

This is the `sala-surface.md` of the security epic — the one architecture
document agreed **before** any implementation agent touches `.co` format,
fs-as-web, or encryption. These are architecture decisions, not features. Each
section states a decision and cites the spec it derives from; where specs
disagree this document is the tie-breaker.

---

## 0. Scope and the one hard precondition

The epic delivers three composable pieces around a single envelope:

1. **`.co` file format** (CO-86) — protobuf-wrapped markdown + assets; the wire
   type that flows through every transport.
2. **Composable protocol stack** (CO-87) — physical → cache → storage → network →
   privacy → security as `Layer` traits; the `.co` envelope is the message at
   every layer.
3. **Encryption envelope** (CO-145 / CO-148) — per-universe ChaCha20-Poly1305
   AEAD with KEK-wrapped DEKs; the same primitives reused by **fs-as-web**
   (CO-110) for live remote file editing.

> **Blocking precondition (load-bearing):** **CO-104/CO-119 (S3 backup + a
> proven restore drill) MUST be DONE before encryption-at-rest ships.** Never
> encrypt what you cannot restore. A lost or corrupted KEK with no restored
> backup is permanent data loss with no recovery path. The restore drill is the
> gate; encryption-at-rest is gated behind it, not the other way around. This is
> the same S3 dependency that currently sits behind the interim git backups and
> the failing CO-143 backup cron — schedule it first.

Everything below assumes that gate is green.

---

## 1. Threat model

The platform's implicit claim is *"your content is encrypted at rest and the
operator cannot read it."* This section says exactly which threats that claim
covers and which it does not.

### Assets and trust boundaries

| Asset | Where it lives | Who may read it |
|---|---|---|
| Plaintext content (markdown body, image bytes) | client memory; server memory during an active session | owner (and explicitly shared recipients — deferred) |
| Ciphertext blobs | `data/universes/.../blobs/` on disk; S3 mirror | anyone with disk/bucket access — must see only opaque bytes |
| Plaintext index (filename, mime, size, sha256, tags, frontmatter) | per-universe SQLite | anyone with DB access — **deliberately clear**, see §4 |
| Per-universe DEK | `keys/universe.kek.enc` (wrapped); process memory (unwrapped, session only) | derived from owner credential only |
| Owner KEK | process memory only, for the session | never persisted; dropped on logout |
| Pairing metadata (fs-as-web) | `agent_pairings` table | the owning user; relay sees pairing IDs only |
| fs-as-web frame payloads | in flight through the relay | browser + agent only (E2E) |

### Threats and mitigations

| Threat | Mitigation | Residual |
|---|---|---|
| Disk / bucket theft (cold storage) | ChaCha20-Poly1305 AEAD; blobs are ciphertext, keyed material is wrapped | index metadata is plaintext by design (§4) |
| Operator turns evil (active server) | KEK lives only in the session that derived it; background tasks operate on ciphertext (backup needs no key) | an operator can read content of a **logged-in** session's process memory — full host compromise is out of scope |
| MITM on a flaky network | `.co` carries `content_hash` (SHA-256) + Ed25519 `signature`; fs-as-web verifies the X25519 ECDH via a 6-word phrase shown on both ends | TLS break alone does not defeat the signature |
| Tampering with ciphertext at rest | Poly1305 tag + AAD = `universe_key‖sha256` binds ciphertext to its identity; any flipped byte fails verification | — |
| Cross-universe blob copy | AAD mismatch — a blob from universe A pasted into B's `blobs/` fails to decrypt | — |
| Compromised page (XSS) tries to mount a filesystem | fs-as-web requires explicit `mount: agent` capability in the page manifest, verified server-side; CSP blocks inline scripts; `agent-client.js` served from the CO origin only | — |
| Replay of a captured frame | per-frame counter nonce + session ratchet (Signal-style) | — |
| Cryptographic break of ChaCha20-Poly1305 / X25519 / Ed25519 | out of scope — best-of-class primitives; reassess on NIST guidance | — |
| Lost owner credential (KEK underivable) | escrow/recovery (CO-106, Wave-future) — **NOT in this epic**; until then a lost password = unreadable content | this is why the restore drill gates the epic |

**The guarantee, stated precisely:** with disk/bucket access an attacker sees
opaque ciphertext for bodies and assets, and clear metadata for the index. With
an active *owner session* compromised at the host level, all bets are off — that
is full host compromise, explicitly out of scope. The platform protects
**data at rest** and **data in the relay**, not a fully-owned live host.

---

## 2. The `.co` envelope (CO-86) — the one shape everything carries

`.co` is a proto3 `CoFile` (full schema in [CO-86](../../work/co/CO-86.md#wire-format-proto3)).
Markdown stays the **authoring** surface; `.co` is the **wire** surface.

### Decisions

- **Header is always cleartext** — `version`, `content_type`, `content_hash`,
  timestamps, `author_id`, `universe_key`, `entry_path`. The header is for
  routing/dispatch and must be readable without a key.
- **Body is a `oneof`** — exactly one of `markdown` | `compressed` | `encrypted`
  | `composite`. Encrypt the **whole body or nothing** for v1 (no field-level
  encryption inside a single `CoFile`).
- **Attachments are a graph** — small (<256 KB) inline, larger by `blob_ref`
  (`sha256:…`, content-addressed CAS), external by signed `url`. Each attachment
  carries its own optional `Encryption`.
- **Disk extension `.co`, mime `application/vnd.co+protobuf`, magic bytes
  `b"co\x01\x00"`** for fast discrimination from `.md`.
- **Auto-wrap on read** — a bare UTF-8 `.md` file is wrapped into a default
  `CoFile` with empty signature/encryption. Existing universes work unchanged;
  no re-import required.
- **Round-trip is byte-faithful** — `.md → CoFile → .md` preserves frontmatter
  (1:1 YAML key mapping) and body bytes, modulo allowed YAML whitespace
  re-emit normalization.

### How `.co` wraps markdown + assets

```
.md (authored)                          attachments on disk / blob CAS
   │ parse                                   │ content-address (sha256)
   ▼                                         ▼
CoFile {                                  Attachment { id=sha256, mime, source }
  header   (cleartext: version, hashes, universe_key, entry_path)
  frontmatter (typed YAML → proto fields; unknown keys → Struct extra)
  form     (presentation hints: layout, theme, view)
  body     = markdown | zstd(markdown) | Encrypted{…} | Composite{md+ids}
  attachments[]  (inline <256KB | blob_ref sha256 | signed url)
  signature  (Ed25519 over canonical bytes, sans the signature field)
}
   │ encode
   ▼
bytes (.co)  ── same binary on every transport: file · Vault API · sync · fs-as-web frame
```

**Content negotiation** at the Vault API: `Accept: application/vnd.co+protobuf`
returns `.co` bytes; `Accept: text/markdown` returns markdown (server-side
auto-wrap decode). The same entry, two representations.

---

## 3. The protocol stack (CO-87) — `.co` at every layer

Each conceptual layer is a `Layer` trait whose message is the `CoFile`. A stack
is composed at the call site; adding a concern is a new impl, not a refactor.

| Layer | Role in this epic |
|---|---|
| Physical | `FilesystemBytes`, `HttpBytes`, `S3Bytes` — raw bytes on disk / net |
| Cache | `LruCache`, `IndexedDbCache` (browser) — sits closest to the consumer |
| Storage | `SqliteStorage` (index), `BlobStore` (sha256 CAS) — durable home |
| Network | `VaultApiTransport`, `SyncProtocolTransport`, the fs-as-web `WS-relay` |
| Privacy | `ChaCha20Encrypted` / `PassthroughPlain` — the §5 envelope |
| Security | `Ed25519Signed` / `UnverifiedTrusted` — integrity + provenance |

### Composition rules (non-negotiable ordering)

1. **Compress below encrypt** — `zstd` first, encrypt the compressed bytes
   (encrypt-then-compress leaks structure).
2. **Encrypt below sign** — sign the **ciphertext**, not the plaintext. A
   verifier can confirm integrity without ever seeing plaintext.
3. **Cache above storage/transport** — caches are nearer the consumer than the
   durable backing.
4. **A stack ends in storage *or* transport** (or both via `Tee` fan-out, e.g.
   SQLite + S3 mirror, LiteFS primary + replicas).

Worked reader stack (browser fetching a private encrypted entry):

```
HttpBytes → VaultApiTransport → Ed25519Verifier → ChaCha20Decryption → LruCache
            (.co bytes→CoFile)   (verify, strip sig)  (decrypt body)     (memoize)
```

fs-as-web (§6) is exactly the stack
`Filesystem → Cache → Privacy(ChaCha20) → Network(WS-relay)` — a worked example
of CO-87, not a new mechanism.

---

## 4. The encryption envelope (CO-145 / CO-148)

### Decision: index plaintext, encrypt body bytes

Searchable encryption is a trap. Instead we **split metadata from bytes**:

- **Plaintext (clear, in per-universe SQLite):** filename, mime, size, sha256,
  tags, and the frontmatter index. This is what stays searchable. The boundary
  is explicit — these columns are *intended* to be readable by anyone with DB
  access, so nothing private may be derived solely from a filename or a tag.
- **Ciphertext (on disk / S3):** the body bytes — markdown body and every asset
  blob. Anyone with disk access sees opaque bytes.

> **Plaintext-for-search boundary, stated as a rule:** the index may contain
> `filename, mime, size, sha256, tags, frontmatter`. The index may **never**
> contain decrypted body text. The `entries.content` markdown body column moves
> to encrypted `assets` rows when CO-86 ships (CO-148 "Phase 3.5"); until then it
> remains an accepted plaintext exception, documented here so it is not silently
> forgotten.

### Key hierarchy (CO-148)

```
Password / OAuth token
   │ Argon2id (id, t=3, m=64 MB, p=1) + per-user salt
   ▼
Owner KEK (256-bit)              ← process memory only, session-scoped, never persisted
   │ ChaCha20-Poly1305 wrap
   ▼
Per-universe DEK (256-bit)       ← generated on universe create; stored wrapped
   │ ChaCha20-Poly1305 + per-blob nonce + AAD = universe_key‖sha256
   ▼
Blob ciphertext                  ← on disk: data/universes/<aa>/<bb>/<key>/blobs/…
```

The wrapped DEK lives at `keys/universe.kek.enc` (+ `keys/nonce`). The KEK is
derived on login and dropped on logout — **the server cannot decrypt without an
active session, and that is the point.** Background tasks (backup) operate on
ciphertext and need no key.

### Storage additions (per-universe `data.db`)

```sql
ALTER TABLE assets ADD COLUMN nonce       BLOB;
ALTER TABLE assets ADD COLUMN cipher_size INTEGER;
ALTER TABLE assets ADD COLUMN encrypted   INTEGER NOT NULL DEFAULT 0;  -- 0=legacy plaintext, 1=ciphertext
```

### Migration path for existing universes

1. **Backup-and-restore-drill first** — CO-104/CO-119 green (the gate above).
2. **Auto-wrap legacy markdown** — existing `.md` reads as a default `CoFile`;
   no re-import.
3. **Background re-encryption of Phase-1 plaintext blobs** — resumable job:
   skip rows where `encrypted=1`; for each `encrypted=0` row write ciphertext to
   a temp file, fsync, rename, update `nonce`/`cipher_size`/`encrypted=1`, then
   unlink plaintext. Atomic per blob; safe to interrupt and resume.
4. **Never panic mid-migration under the storage mutex** — a re-encryption
   worker that fails must surface `ERR_NO_KEY` / a clean error, never poison the
   `Mutex<Storage>` (per the hard-won Wave-4/5 rule).

### Alignment between CO-86 and CO-148

Both use ChaCha20-Poly1305 with the same wrap shape (`encrypted_key` + `nonce` +
`key_recipient`). **If CO-86 ships first**, CO-148's `crypto.rs` becomes a thin
wrapper over `core/src/co_format/encryption.rs`. **If CO-148 ships first**, it
ships the primitives and CO-86 reuses them. Either order is valid; they must not
fork the implementation.

### Single-owner now, multi-recipient later

V1 is single-owner. Multi-collaborator universes (recipient list, DEK wrapped
per pubkey, key rotation) are deferred to CO-145 Phase-6+. The `Encrypted`
message already carries `key_recipient`, so the format does not need to change
to add recipients later.

---

## 5. Filesystem-as-web (CO-110) — the live editing flow

fs-as-web pivots from *"server holds canonical state"* to *"server is a verified
relay between a browser and the user's own machine."* It reuses the §4 envelope
primitives over a different transport.

### Components

1. **`co-agent`** — a daemon on the user's PC exposing a scoped read/write
   filesystem surface over an encrypted WebSocket. Only paths under configured
   `roots` are exposed; `../` traversal is rejected at the agent.
2. **Server relay** — `GET /api/v1/agent/relay?pairing_id=…&direction=browser|agent`.
   Forwards frames **as-is**; never inspects the body. Validates: caller
   authenticated, pairing exists/unexpired/owned, page has `mount: agent`, peer
   connected (else queue ≤ 60 s).
3. **`agent-client.js`** — browser library: WebSocket + libsodium X25519 ECDH +
   ChaCha20-Poly1305 frame encryption.

### Flow

```
Browser (Co page)  ──enc WS──►  Co server /agent/relay  ──enc WS──►  co-agent on user PC
  E2E keypair                     sees ciphertext +                    E2E keypair
  renders FS UI                   pairing IDs only                     reads/writes allowed paths
       └────────────── shared X25519 ECDH key — never on the server ──────────────┘
```

The relay multiplexes frames between paired (browser, agent) connections by
pairing ID and **cannot decrypt the payload**. It enforces capability, pairing
ownership, and rate limits — nothing more.

### Pairing trust ceremony

A pairing is established once per (page, browser, agent) with explicit human
consent on both ends. A **6-word phrase derived from the X25519 ECDH shared
secret appears on both the page and the agent terminal** — the social proof that
the pairing is correct and not a MITM (the Signal/Wire emoji-verification
pattern, adapted to terminals). `mount: agent` capability is declared in the
page frontmatter and verified server-side, composing with the per-universe
schema validator.

### Crypto details (same primitives as §4)

- **Key exchange:** X25519 ECDH; server mediates pubkey exchange but never sees
  the shared secret.
- **Frame encryption:** ChaCha20-Poly1305 with a per-frame counter nonce — the
  same primitive as the `.co` envelope.
- **Forward secrecy:** session ratchet (simplified Double Ratchet); new keys per
  connection; closing the tab kills the symmetric key.
- **Server compromise:** can drop frames, deny service, record ciphertext. Can
  **not** read or inject content (forgery fails Poly1305).

fs-as-web is **not v1** — it depends on the `.co` envelope (CO-86) and benefits
from the composable stack (CO-87) being established first. It phases read-only →
write → watch-streams → multi-pairing (CO-110 §Phasing).

---

## 6. Constraint pins back to the source specs

This doc is the decision record; the specs reference it as their constraint pin:

- **CO-86** — body is whole-or-nothing encrypted (v1); header always cleartext;
  shares the ChaCha20 impl with CO-148.
- **CO-87** — composition order is fixed: compress < encrypt < sign; cache above
  storage/transport.
- **CO-110** — fs-as-web is the `Filesystem → Cache → Privacy → Network(relay)`
  stack; reuses the `.co` envelope frame format and the §4 primitives.
- **CO-145 / CO-148** — index plaintext / encrypt bytes; per-universe DEK under
  owner KEK; the plaintext-for-search boundary in §4 is binding.
- **CO-104 / CO-119** — restore drill is the **blocking precondition** for
  encryption-at-rest. No encryption ships until restore is proven.

---

## 7. Out of scope (this epic)

- Field-level encryption inside a single `CoFile` (whole-body or nothing).
- Multi-recipient key sharing / key rotation (CO-145 Phase-6+).
- Streaming-AEAD for files > ~50 MB (full-decrypt-then-slice is correct for v1).
- Body-text full-search across encrypted blobs (client-side IndexedDB index is
  the leading candidate, unspecified here).
- Key escrow / lost-password recovery (CO-106) — until it exists, a lost owner
  credential means unreadable content, which is precisely why restore gates.
- Federation between agents; terminal/LSP bridging through the agent.
