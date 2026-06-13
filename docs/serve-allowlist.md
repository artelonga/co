# Serve allowlist — serve only published/indexed content (CO-439)

> **Principle:** the surfaces server serves **only** content that is present in
> a universe's published index, never a raw file that merely happens to exist on
> disk. *Serve the index, not the disk.*

## Why this exists — the draft-leak post-mortem (2026-06)

A personal draft (`thrive market.md`) in `ArteLonga/yuri/` went public. It was
the only file to cross **four** conditions at once:

1. it sat in a **served directory**,
2. it did **not** match any `.dockerignore` pattern,
3. it **existed on disk** at deploy time, and
4. it was in fact **private**.

The structural gap: the surfaces server had **no allowlist** — it would serve
any file in a served directory, indexed or not. The only barrier between "draft
on disk" and "public on the internet" was the `.dockerignore`, a **denylist**
that *fails open*: whatever it does not know about leaks.

**`.dockerignore` is not a security boundary.**

## The boundary: allowlist-on-serve

`co-web/src/server/allowlist.rs` is the single chokepoint. A deep-link request
`GET /{universe}/{*subpath}` resolves to **404** unless:

- one of the candidate index paths for `subpath` exists in the universe's
  published index (the per-universe `entries` table), **and**
- the entry is visible to the caller — on a `anon_published_only` universe an
  anonymous caller only sees entries explicitly marked `published: true`
  (CO-324/CO-330). Authenticated callers and non-gated universes always pass.

A `draft.md` sitting in a served directory but **absent from the index** is
never served — it is a 404, not a 200. This mirrors the API-layer published
filter (`entries/routes.rs`) so the surfaces server and the content API agree.

Because content is read from the index (SQLite `entries`), not streamed from
disk, an unindexed file is structurally unreachable — the allowlist is the
posture, not a patch.

## Defense in depth — `.dockerignore`

The root [`.dockerignore`](../.dockerignore) is a cheap, complementary second
layer (NOT the boundary). It keeps known draft/scratch categories out of the
build context entirely:

```
WhatsApp*
_*
shot-*
*.mov
**/IMG_*
```

These match the categories that appeared in the incident. A denylist fails
open, so it never substitutes for the allowlist above.

## Flow rule — where a draft is born

**A draft never originates inside a served directory.** It is born in the vault
(`~/projects/yuri`) and only crosses into a served universe (e.g.
`ArteLonga/yuri/`) via the explicit drafts→published flow, which creates an
**index entry**. If it is not in the index, it is not served — so an in-progress
draft that never gets published can never leak, regardless of where it sits on
disk.

```
~/projects/yuri  (vault — private, never served)
      │  drafts → published flow (creates an index entry)
      ▼
ArteLonga/yuri/  (served — only the indexed/published entries reach the surface)
```

This is the same family as the universe visibility gate (CO-161) and feeds the
content-addressed encrypted-asset storage plan (CO-145): once assets are
indexed + content-addressed, "serve only the indexed" is the default posture.

## Audit — the leak surface today

`audit_serve` walks every universe's served on-disk root and reports files that
exist on disk but are **absent from the index** — the "servable but not
published" surface that the allowlist now refuses:

```bash
# against a CO data dir (defaults to $CO_DATA_DIR, then ./data)
cargo run -p co-web --bin audit_serve -- /data
```

Exit code `0` when no leak surface remains, `1` when at least one unindexed file
is found (so it can gate a pre-deploy check). What it lists is exactly the
surface to remove or push through the drafts→published flow.
