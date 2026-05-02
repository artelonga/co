---
title: Encrypted, indexable, lazy-loaded assets
status: design
created: 2026-04-30
parent_ticket: CO-145
related: CO-81, CO-86
---

# Encrypted, indexable, lazy-loaded assets

## Goal

Every file in a universe — markdown, image, video, audio, PDF, arbitrary blob — is:

1. **Encrypted at rest** with a per-universe key, wrapped under user-derived KEKs.
2. **Indexable** without decryption: filename, mime, size, sha256, frontmatter (for `.md`), tags, dates.
3. **Available by request** through a stable per-asset endpoint, never bulk-leaked.
4. **Lazily loaded** — large media (images, video) stream on demand with HTTP range support; the SPA uses `<img loading="lazy">` and `<video preload="none">`.

## Key insight: split metadata from body

| Layer | Plaintext | Encrypted | Purpose |
|---|---|---|---|
| **Universe row** | `key`, `name`, `slug`, `visibility`, `parent_key`, `content_version` | — | List/route/permission gates |
| **Asset index row** | `sha256`, `mime`, `size`, `filename`, `created_at`, `tags[]` | — | Search, listing, virus-scan, dedupe |
| **Frontmatter** | typed fields (title, tags, dates, status) per CO-86 | — | Search, sort, filter |
| **Body bytes** | — | ChaCha20-Poly1305 with per-universe DEK | The thing only authorized users see |

Searchable encryption is hard and slow. Don't use it. Instead: keep the *metadata that must be searchable* in the clear, and encrypt the *content bytes*. Anyone with disk access sees a directory of opaque ciphertext blobs and a SQLite table of size/mime/sha256 — enough to administer, not enough to read.

For markdown specifically, we extract the indexable plaintext at *encrypt* time (frontmatter → typed columns; body title from the first H1) and store body ciphertext separately. The SPA renders the body only after fetching + decrypting it, which gives us lazy-load for free.

## Storage layout (extends CO-77 + CO-81)

```
data/universes/<aa>/<bb>/<key>/
  data.db                           # per-universe SQLite (existing; CO-77)
  blobs/
    <aa>/<bb>/<sha256>              # ciphertext blob, content-addressed
  keys/
    universe.kek.enc                # universe DEK wrapped under owner's KEK
```

- **Content addressing**: hash is sha256 of *plaintext*. Two users uploading the same image to different universes still get separate ciphertexts (different DEKs) but the *plaintext-sha256* dedupes within a universe.
- **Sharded path**: `<aa>/<bb>` two-level fan-out keeps any directory under a few hundred entries even at millions of blobs.
- **Per-universe DEK**: ChaCha20-Poly1305 256-bit key, generated on universe create, wrapped under the owner's KEK (derived from password / OAuth token via Argon2id).

## Per-universe SQLite schema additions

```sql
CREATE TABLE assets (
    sha256          TEXT PRIMARY KEY,    -- of plaintext
    ciphertext_path TEXT NOT NULL,       -- relative to universe dir; e.g. "blobs/aa/bb/<sha>"
    nonce           BLOB NOT NULL,       -- 12 bytes for ChaCha20-Poly1305
    mime            TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,    -- plaintext size
    cipher_size     INTEGER NOT NULL,    -- ciphertext size (size + 16 tag)
    filename        TEXT,                -- original filename (informational)
    created_at_ns   INTEGER NOT NULL,
    created_by      TEXT,                -- user_id
    refcount        INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE asset_tags (
    sha256          TEXT NOT NULL,
    tag             TEXT NOT NULL,
    PRIMARY KEY (sha256, tag)
);

-- Existing entries table gets an attachments column referencing assets:
ALTER TABLE entries ADD COLUMN attachment_shas TEXT;  -- JSON array
```

Markdown bodies move from `entries.content` (plaintext column) to `assets` rows referenced by `entries.body_sha256` once Phase 3 lands. Phase 1 leaves markdown plaintext for compatibility.

## API surface

```
POST   /api/v1/universes/{u}/assets
       Body: raw bytes (or multipart for filename hint)
       Headers: Content-Type
       → { sha256, mime, size, url }

GET    /api/v1/universes/{u}/assets/{sha256}
       Range: bytes=0-1023      (for video/large image)
       → bytes (decrypted on the fly with universe DEK)
       Cache-Control: private, max-age=31536000, immutable
       ETag: "<sha256>"

GET    /api/v1/universes/{u}/assets
       ?type=image/*&tag=foo&since=2026-01-01
       → [{ sha256, mime, size, filename, created_at, tags }]

DELETE /api/v1/universes/{u}/assets/{sha256}
       → 204 if refcount == 0 after; 409 with referrers list otherwise
```

Indexable list endpoint returns *only metadata*, never bytes. SPA does the second hop per asset, which gives `<img loading="lazy">` natural breathing room.

## Encryption details

| Concern | Decision |
|---|---|
| Algorithm | ChaCha20-Poly1305 (existing CO-86 dep, AEAD, fast on ARM) |
| Key size | 256 bits |
| Nonce | 12 random bytes per blob, stored alongside |
| AAD | `universe_key \|\| sha256` — binds ciphertext to its universe + identity |
| Key wrapping | Owner KEK derived from password via Argon2id (id, t=3, m=64MB, p=1) |
| Key rotation | New DEK generated on rotate; rewrap of existing blobs is async via background job |
| Multi-recipient | Universe DEK wrapped once per collaborator's pubkey (out of scope for v1; single-owner only) |

**What the server can see**: universe metadata, asset metadata, ciphertext bytes. **What it cannot see** without the owner's KEK: plaintext bytes. Server-side full-text search of bodies is not supported in this design — it would defeat the encryption. Search runs over (a) frontmatter, (b) filenames, (c) tags, (d) sha256 lookups. If body-text search is later required, it goes through a privileged compute zone (per the homomorphic-encryption-functional doc) running on the client.

## Lazy-load contract

The SPA must:
- Use `<img src="/api/v1/universes/{u}/assets/{sha256}" loading="lazy" decoding="async">` for images.
- Use `<video src="…" preload="none">` for video; browser will issue range requests on play.
- For markdown previews in the board view, fetch only frontmatter + first-paragraph (a separate `?excerpt=true` query that decrypts and returns the first 200 plaintext chars).
- Cache decrypted blobs in IndexedDB keyed by sha256 (immutable; safe to cache forever).

Server must:
- Serve `Cache-Control: private, max-age=31536000, immutable` (sha256 = identity).
- Honor `Range:` requests by decrypting the *whole* blob into memory then slicing. ChaCha20 supports random-access decrypt but Poly1305 verifies over the whole stream; for v1 we pay the full-decrypt cost (acceptable up to ~50MB; revisit chunked-AEAD for video later).
- Reject anonymous requests for private universes; allow them for `is_public=1` universes (per existing visibility model).

## Out of scope for v1

- Multi-user shared universes (Phase 4: collaborator key sharing).
- Streaming encryption for files >50MB (Phase 5: STREAM construction or age's chunked format).
- Server-side thumbnails (Phase 5: client-side derives + uploads its own thumbnail blob).
- Body-text search across encrypted blobs (open question; client-side index in IndexedDB is the leading candidate).
- Replacing markdown plaintext storage with encrypted blobs (Phase 3 — Phase 1 keeps the existing `entries.content` column to avoid a destabilizing rewrite).

## Phasing (matches CO-146..150)

| Phase | Ticket | Deliverable | Unblocks |
|---|---|---|---|
| 1 | CO-146 | Binary upload + sha256 CAS, plaintext, per-universe `assets` table + GET endpoint | 506 MB quilomboaraucaria image upload; markdown ![] references |
| 2 | CO-147 | Indexable metadata: list/filter endpoint, tags, mime filter, frontmatter extraction | Search UI, asset browser |
| 3 | CO-148 | Per-universe DEK + ChaCha20 envelope at write/read; key wrapping under owner KEK | Encryption-at-rest claim |
| 4 | CO-149 | HTTP range support, ETag, immutable cache, IndexedDB cache contract | Video/large-image streaming |
| 5 | CO-150 | SPA `<img loading="lazy">` + `<video preload="none">`, asset browser UI, frontmatter excerpt endpoint | Real lazy load in the UI |

Phase 1 ships first because the user has a concrete blocker: 506 MB of un-uploaded content sitting in the quilomboaraucaria repo. Encryption (Phase 3) is correct to defer until upload works at all — encrypting a path that doesn't exist is wasted scaffolding.

## Risks

- **Decryption-at-egress is CPU-bound**: at 50MB blobs and 100 req/s we'll saturate one core on Fly. Mitigation: stream + tokio::spawn_blocking, or ship the user's own KEK to the client and decrypt browser-side (preferred long-term; CO-148 follow-up).
- **Backups must include `keys/`**: lose the wrapped DEK and you've lost the universe. The CO-143 backup-cron script already tars the whole universe dir, so this is automatic.
- **Refcount drift**: if a markdown reference is deleted by direct DB edit (not via API) the refcount decay is missed. Mitigation: nightly GC pass scans markdown bodies for `![]()` and recomputes refcounts. Same pattern CO-81 already specifies.
- **Migration of existing universes**: 1.35.x universes have plaintext markdown in `entries.content` and no blob store. Phase 3 adds encryption *for new content*; existing content gets a one-shot rewrite job. No data loss possible because the rewrite reads from plaintext to ciphertext atomically.
