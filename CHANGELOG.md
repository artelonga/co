# Changelog

All notable changes to CO are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.41.0] — 2026-05-03

### Added — CO-160: Inline PDF renderer in the SPA (PDF.js)

Reference entries with `type: reference`, `medium: pdf`, and a valid `file:` field now render an inline PDF viewer below the markdown body when opened in the zoom modal. The viewer is powered by PDF.js 5.7.284 (self-hosted, no external dependency), embedded at `/pdfjs/` and served by the same static file handler as other assets.

- `shouldRenderInlinePdf` / `pdfUrlFromCard` / `buildPdfViewerHtml` / `initPdfViewerActions` helpers detect the entry shape and wire up the iframe
- PDF URL resolves via the asset endpoint (`blob_sha256`) if available, falling back to the vault path-relative URL
- `<iframe loading="lazy" allowfullscreen>` uses browser-native lazy loading; PDF bytes are only fetched when the viewer is in the viewport
- "Baixar PDF" button triggers a browser download with the original filename via the `download` attribute
- "Tela cheia" button calls `requestFullscreen()` on the iframe element (Fullscreen API)
- Auth cookies are forwarded automatically (same-origin); private universe PDFs are not accessible to anonymous viewers without a session
- `pdfjs/` path prefix added to the static file handler in `server.rs`; `.mjs` MIME type added to `guess_content_type`
- PDF.js bundle: `build/pdf.mjs`, `build/pdf.worker.mjs`, `build/pdf.sandbox.mjs`, `web/viewer.html`, `web/viewer.mjs`, `web/viewer.css`, `web/images/`, `web/locale/en-US/` + `web/locale/pt-BR/` — ~4.3 MB vendored at `co-web/static/pdfjs/`
- No LCP regression on entries that don't have an inline-PDF section (iframe is never inserted)

## [1.40.0] — 2026-05-03

### Added — CO-158: Reference versioning — work_id + editions[] + primary/secondary source chain

- `references_meta` table gains `work_id`, `edition_id`, `primary_layer` columns; PK changed to `(universe_key, entry_path, edition_id)`.
- Per-universe DB migration v8: existing rows backfilled with `edition_id = 'default'`, `work_id` derived from filename stem, `primary_layer = NULL`.
- Reference cards may now carry an `editions:` array — one `references_meta` row is written per edition, so a single card can represent multiple concrete artifacts (scans, reprints, OCR'd versions).
- `work_id` groups all editions of the same conceptual work; auto-derived from the card's filename stem when not explicitly authored.
- `primary_layer` stores the minimum layer value from `primary_source_chain` (0 = phenomenon, 1 = transcription, 2 = publication, 3+ = re-print / scan / OCR); `null` when no chain is authored.
- Duplicate sha256 detection: re-uploading a PDF that already exists in `references_meta` under the same `work_id` skips creating a second edition row.
- New REST endpoints:
  - `GET /references?work_id=<id>` — return every edition row for a given work
  - `GET /references?primary_layer=<n>` — return references with that source-chain layer
  - `GET /references/works` — list all distinct `work_id` values in the universe
- 5 new CO-158 unit tests; existing CO-156 tests updated to pass with the new schema.

## [1.39.0] — 2026-05-03

### Added — CO-156: Universal envelope — `reference` content type + uniform CRUD telemetry

#### Part A — `reference` as a first-class content type

- `_universe.yaml` parser now accepts `properties_per_type` with the per-content-type property map using `kind: text|int|enum|list` vocabulary; content types may be declared as bare strings (`- reference`) or full objects.
- Per-universe DB v7 migration creates `references_meta` (structured shadow table) + `references_fts` (FTS5 index over title, body, transcription).
- Every reference-card write (via entry_routes, vault_routes, or the new reference_routes) upserts `references_meta`; sha256 of the bound sibling asset is resolved and stored.
- New REST API under `/api/v1/universes/{u}/references`:
  - `GET /references?medium=pdf` — list cards with medium/seed_status/FTS filters
  - `GET /references/orphan-blobs` — assets with no card
  - `GET /references/broken-cards` — cards whose `file:` doesn't resolve
  - `GET /references/{*path}` — single card
  - `POST /references` — create card
  - `PUT /references/{*path}` — update card
  - `DELETE /references/{*path}` — delete card (blob unaffected)

#### Part B — Universal CRUD telemetry envelope

Every state change now emits one `telemetry_events` row with `event_type = "crud"` carrying a uniform envelope: `kind` (`entry.upsert`, `entry.delete`, `asset.upload`, `asset.delete`, `relation.create`, `relation.delete`, `ws.connect`, `ws.disconnect`, `ws.lag`, `auth.login`, `auth.logout`), `universe`, `list`, `key`, `actor`, `session_id`, `deployment_version` (from `CARGO_PKG_VERSION`), `timestamp_ns`, and `extra`.

- `deployment_version` matches `cargo workspace.package.version` at write time.
- `session_id` is derived from JWT session cookie hash or anon visitante cookie hash.
- `/co/co/telemetria` admin dashboard now shows CRUD events by kind with 24-hour window.
- `GET /api/v1/admin/telemetry/crud-summary` returns the 24h CRUD breakdown.
- `docs/telemetry-envelope.md` documents all event kinds and their `extra` shapes.

### Added — PDF metadata extraction tool (CO-157)

`scripts/extract-pdf-meta.py` auto-populates reference-card `.md` siblings from source PDFs. Extracts title (from `/Info.Title` or first-page heuristic), authors, year, page count, sha256, language (via `langdetect`), DOI (regex `10.\d{4,9}/...`), ISBN, abstract, and keywords. Writes YAML frontmatter + prose body matching the `reference` content type envelope from CO-156.

- Diff mode (existing `.md`, no `--force`): shows unified diff on stderr, exits non-zero if stable fields differ
- `--force`: rewrites auto-generated block (frontmatter + abstract section) while preserving `## Notes` and any human-authored content
- Flags `extraction: text-only-failed` for image-only PDFs where `pypdf` yields no text
- Test fixture at `tests/fixtures/stub.pdf` + 25 unit and integration tests in `tests/test_extract_pdf_meta.py`

### Added — CO-159: INMET moon-phase importer

`scripts/import-moon-phases.py <year>` — fetches the lunar phase table from
`portal.inmet.gov.br/paginas/luas` and writes one `.md` per phase into
`time/moon-phases/<year>/` using the `moon-phase.md` template frontmatter.

- Parses four columns (LUA NOVA → `moon.new`, LUA CRESCENTE → `moon.first-quarter`,
  LUA CHEIA → `moon.full`, LUA MINGUANTE → `moon.last-quarter`)
- Times in BRT (UTC-3); `at_iso` = BRT + 3 h, `at_local` carries the wall-clock
- Idempotent: skip if `at_iso` matches the existing file; update if INMET revised the table
- Fails loudly on any unexpected HTML structure so silent data corruption is impossible
- Cross-year: `--time-dir` and `?ano=<year>` URL parameter work for any year
- `tests/fixtures/inmet-luas-2026.html` — offline HTML snapshot for CI (2026: 50 phases)
- Ran against `~/projects/time` to populate all 50 phases for 2026

## [1.38.11] — 2026-05-03

### Added — `time` universe + Cadogan/ayvu-rapyta reference + 3 follow-up tickets

A 7th private universe `time` for every time-stamped event the system knows about — astronomical (`moon-phase`, `eclipse`, `equinox`, `solstice`), generic (`event`), and internal (`telemetry-event`). One queryable timeline; `at_iso` is the canonical sort key. Lives at `~/projects/time/`.

Manifest declares 6 content_types and the supporting properties: `at_iso` (UTC instant), `at_local` (wall-clock), `duration_seconds` (for events with extent), `geo` (lat/lon/region/timezone), `source` + `source_url`, `kind` (sub-type tag for SPA rendering), and the telemetry-specific `related_universe` / `related_entry_path` / `actor_id` / `deployment_version` / `extra` fields.

Scaffolded skeleton:

- `time/_universe.yaml` — manifest with the 6 content types
- `time/index.md` — universe home explaining "why one universe, not many"
- `time/README.md` — directory layout and source-of-truth policy
- `time/templates/{event, moon-phase, telemetry-event}.md` — copy-and-edit templates
- `time/moon-phases/2026/2026-01-13-new.md` — first hand-authored INMET phase (will be replaced by CO-159's importer)

### Added — Cadogan / ayvu-rapyta reference card

`mbya/refs/ayvu-rapyta-cadogan.md` — reference card for León Cadogan's *Ayvu Rapyta: Textos míticos de los Mbyá Guaraní del Guairá* (1959). Demonstrates the `secondary_source: true` + `canonical_source: indigenous-mbya-knowledge-keepers` distinction that CO-158 will turn into a first-class chain-of-custody schema. Identifies 7 Mbyá terms likely to be cross-referenced once the body is read (ayvu, ayvu-rapyta, ñe'ẽ, ñamandu, tenondé, jaryi, kuaray).

### Filed — CO-157, CO-158, CO-159

Three follow-up tickets for the patterns this work surfaces:

- **CO-157** — PDF metadata extraction tool (`scripts/extract-pdf-meta.py`); read the PDF's /Info dict + first-page heuristics + DOI regex + sha256 to auto-populate the reference card. Reduces "drop a PDF, run, review and commit" friction.
- **CO-158** — Reference versioning. Same conceptual work (`work_id`) → multiple concrete artifacts (`editions[]`); each edition has its own sha256, pages, language, editor_notes, seed_status. Plus `primary_source_chain` documenting layers of mediation between original phenomenon and cited document — the schema honestly captures "this is a digital scan of a 1992 reprint of a 1959 transcription of 1940s field recordings."
- **CO-159** — INMET moon-phase importer; scrapes `portal.inmet.gov.br/paginas/luas` and emits one `.md` per phase into `time/moon-phases/<year>/`. Idempotent re-runs. Cross-year support out of the box.

`time` is `visibility: private`; admin gets membership via the same `system_keys` list as the topologia universes.

## [1.38.10] — 2026-05-03

### Fixed — admin gets membership in mbya + topologia universes

`ensure_admin_universe_memberships` only granted yuri membership for `template`, `quilomboaraucaria`, `yggdrasil`, `dados`, `artelonga`, `rfq`, `co` — the 5 mbya/topologia universes were missing. Symptoms: `GET /api/v1/universes/languages` returned 404 to yuri (private universe + non-member = pretend it doesn't exist), and `POST /api/v1/universes/mbya/assets` returned 403 (PDF uploads silently failing — observed: 8/8 binaries failed at `bulk-upload-binary.py mbya`).

Added the 6 keys (`mbya`, `concepts`, `guarani-mbya`, `portuguese`, `yoruba`, `languages`) to the system_keys list. Idempotent on every boot via `INSERT OR IGNORE`. After deploy, yuri sees these universes in the sidebar + can upload binaries to them.

## [1.38.9] — 2026-05-03

### Added — `languages/` catalog universe with authoritative metadata

A 6th topologia universe — `languages` — that holds one `.md` per language with structured metadata: BCP-47 code, native/EN/PT names, ISO 639-1/3, Glottolog code + URL, SAPhon URL (for South American indigenous), language family, geographic centroid (lat/lon), speaker estimate, cross-ref to the term plane (when one exists).

```
GET /api/v1/universes/languages/entries
GET /api/v1/universes/languages/entries/gn-mbya.md
GET /api/v1/universes/languages/entries?q=tupi
```

Initial 4 entries: `gn-mbya` (SAPhon + Glottolog `mbya1239` + Dooley reference), `pt-BR` (Glottolog `braz1246`), `en` (Glottolog `stan1293`; meta-language for concept anchors, no term plane), `yo` (Glottolog `yoru1245`; Afro-Brazilian liturgical scope).

Source-of-truth policy when authorities disagree (documented in `topologia/languages/index.md`):
- Identity: Glottolog wins.
- SA indigenous phonology / coordinates: SAPhon wins.
- Geography otherwise: SAPhon for SA indigenous → community/state stats → Wikipedia infobox.

This catalog is the foundation for CO-153 (cross-universe `entry_relations.to_universe`) — term entries currently carry `language_code: gn-mbya` as a string; once cross-universe relations land, they upgrade to `co://languages/gn-mbya.md` refs that resolve through the relation graph.

`languages` is `visibility: private` (same status as the other 4 topologia universes — under active authoring).

## [1.38.8] — 2026-05-03

### Changed — topologia universes private; watcher narrows to `.md`-only

`seed_admin_content_universes` now declares the 4 topologia universes (`concepts`, `guarani-mbya`, `portuguese`, `yoruba`) as `visibility: private` (down from `public-subscribable`). Reason: the term entries are still under active authoring with non-native draft status; flipping back to public-subscribable comes when seed_status passes review. The reconcile pass on every boot pushes the new visibility through. `mbya` (Arandu) stays public-subscribable.

`co-agent-watch::is_syncable` narrowed to `.md`-only. Binaries (PDF, image, audio, video) need the `/api/v1/universes/{u}/assets` path with sha256 content addressing — the WS protocol's `CoFile.content` is UTF-8-checked at the server, so PDFs were previously sent over the wire and silently rejected. Filter at the source instead. Run `scripts/bulk-upload-binary.py <slug> <root>` to push binaries; CO-151 Phase 2 will add a typed `Asset` body to `SyncDelta` so the watcher can stream them too.

### Added — CO-156 filed (universal envelope: binary content cards + uniform CRUD telemetry)

Filed `work/co/CO-156.md` codifying the pattern that emerged from the topologia + mbya/refs work: a `reference` content type with a `.md` metadata card sibling for any non-markdown asset (PDF, image, video, YouTube URL); an indexable `references_meta` shadow table + FTS over `transcription`; a single telemetry envelope every CRUD + WS state change emits. Subsumes/supersedes CO-154's narrower scope.

### Authored — content (synced via watcher to prod)

- `topologia/concepts/concepts/fractality.md` — new concept anchor (kosmos domain).
- `topologia/concepts/concepts/recursion.md` — new concept anchor (language domain).
- `topologia/guarani-mbya/terms/pindovy.md` — 4-way species mapping example: Mbyá `pindovy` ↔ folk Portuguese names ↔ scientific *Syagrus romanzoffiana* ↔ geographic distribution. Demonstrates the universal-schema pattern from `topologia/docs/universe-as-list-of-lists.md`.
- `topologia/portuguese/terms/jeriva.md` — companion folk-name entry pointing back at the canonical pindovy mapping.
- `topologia/docs/universe-as-list-of-lists.md` — philosophy note: universe = list of lists; state = (user_session, version_deployment); universal CRUD + telemetry envelope.
- `mbya/refs/index.md` — index of references (7 PDFs + 1 YouTube stub).
- `mbya/refs/{CADERNO4_CRISTINE_TAKUA_GUA, educacao_indigena_…, GNDicInt, GNDicLex, Livro_Guarani_digital, PICH0255-T}.md` — metadata cards for each PDF in the project, with `seed_status`, mime, size, language, keywords, and links into the lexicon.
- `mbya/refs/youtube-czwpPvu3ziQ.md` — pattern stub for YouTube references (URL + chapters/transcription/likely-mbya-terms slots).

## [1.38.7] — 2026-05-03

### Added — meaning-topology universes (mbya + topologia 4-plane) into sync

`seed_admin_content_universes` now creates 5 new public-subscribable universes on every prod boot:

| Key | Source | Purpose |
|---|---|---|
| `mbya` | `~/projects/mbya/` | Arandu Mbyá Guarani lexicon (Rust workspace + content) |
| `concepts` | `~/projects/topologia/concepts/` | Language-agnostic meaning anchors |
| `guarani-mbya` | `~/projects/topologia/guarani-mbya/` | Mbyá Guarani term plane (cross-language layer above Arandu) |
| `portuguese` | `~/projects/topologia/portuguese/` | Portuguese term plane |
| `yoruba` | `~/projects/topologia/yoruba/` | Yoruba term plane |

`scripts/co-watch-v2.sh` REPOS array now spawns one watcher per universe (9 total). Local edits to any of the 5 new repos sync to prod via the CO-151 protobuf+WS path.

### Added — `topologia/` becomes a Rust workspace

Created `topologia/Cargo.toml` + `topologia/crates/topologia-core/` — a no-I/O crate of shared types (`Term`, `Concept`, `LanguagePlane` trait, `ConceptPlane` trait, `TranslationLink`) that **mbya** (Arandu) and **co** can both add as a path dependency. The crate documents the two distinct i18n patterns:

1. **Language as universe** (lexicon model) — each language is a CO universe, every entry is a `term`, cross-language linking via `co://concepts/<key>.md` URIs.
2. **Language as frontmatter field** (translation model) — any user's entry can carry `language: <code>` plus a `translation_of: { universe, path, canonical_language }` link to the canonical.

Adapter crates (`topologia-co-adapter`, `topologia-mbya-adapter`) are filed as future work — `topologia-core` is content-shape-only and ships first so consumers can settle on the canonical types.

`topologia/_template-language/` is a copy-and-rename template (`{{LANG_NAME}}` / `{{LANG_CODE}}` placeholders) for adding new language planes; `topologia/docs/i18n-patterns.md` walks through both patterns and when to use each.

## [1.38.6] — 2026-05-03

### Added — web→local sync direction (CO-151 second leg)

The v2 watcher's downlink path was already wired (server broadcasts → `apply_batch`), but **only client-originated changes** ever reached the broadcast. REST writes via `/vault/*` and `/entries/*` bypassed the SyncRoom entirely, so a SPA edit on prod was invisible to connected watchers.

**Server side (`co-web/src/sync_ws.rs`):** added `emit_rest_upsert` and `emit_rest_delete` helpers that build a `SyncDelta`, append it to the room's delta-log (so reconnecting clients can resume), and broadcast it with `origin_conn_id = 0` (REST has no WS connection, so the per-connection echo filter never matches and every connected watcher gets the frame). `vault_routes::put_vault_file` and `delete_vault_file` now call these after the entry write completes.

**Client side (`co-agent/src/watcher.rs`):**

1. **Path resolution.** `apply_batch` now joins `delta.entry_path` against the watch root (`config.watch_dirs.first()`) instead of using it CWD-relative. Defensively rejects absolute paths.
2. **Echo dedup.** A shared `Arc<Mutex<HashMap<sha256, Instant>>>` tracks recently-applied content; `encode_event` skips emitting a delta when the on-disk sha256 matches a recently-applied one (5s window). Closes the web→local→fs-notify→web echo loop.
3. **Idempotent local write.** `apply_batch` reads the file before writing — if the bytes already match, the write is skipped (avoids triggering fs-notify at all).
4. **Delete-side dedup.** Successful local deletes record a `DEL:<path>` sentinel in the same map.

**Tests:** new `test_encode_event_skips_recently_applied_content` covers the dedup behavior end-to-end. Watcher suite is now 8 tests; co-web suite still 281 tests; clippy clean.

End-to-end verification (after deploy + watcher restart):
- `curl -X PUT /api/v1/universes/co/vault/notes/test.md` → file appears at `~/projects/co/notes/test.md` within ~1s
- `curl -X DELETE …` → file removed locally within ~1s
- No echo loop in `~/.co/watch-v2.log`

## [1.38.5] — 2026-05-03

### Fixed — sync-driven writes now reconcile `content_count` per batch

After 1.38.4 redeployed, `co` still drifted (513 cached vs 500 actual). Cause: `apply_deltas_to_storage` calls `EntryIndex::upsert` and `DELETE FROM entries` on the per-universe DB but never touches the cached `content_count` field on `meta.universes`. Boot-time `recompute_content_counts` corrected the drift but new sync writes immediately reintroduced it.

`apply_deltas_to_storage` now ends each batch with `UPDATE universes SET content_count = (SELECT COUNT(*) FROM entries) WHERE key = ?` — one extra `COUNT(*)` per batch, atomic, drift-free. Already-shipped boot reconcile + per-batch reconcile = `content_count` stays accurate forever.

## [1.38.4] — 2026-05-03

### Fixed — SPA route fallback for nested universe paths + content_count reconcile

Two follow-ups from the post-CO-151 prod checklist:

1. **`/co/{slug}/{*subpath}` now serves the SPA shell.** The router only registered `/co/{slug}` and `/co/{slug}/assets`, so anything deeper (e.g. `/co/yuri/dados`, `/co/co/processos/alterar-pagina-na-web`) fell through to a 404. Added a catch-all `*subpath` route that serves the SPA shell so the client-side router can resolve those paths. Placed AFTER `/co/{slug}/assets` and `/co/yggdrasil/{game}` so axum's matcher prefers the more specific routes.

2. **`content_count` reconcile already runs on boot** (`recompute_content_counts` from CO-142 Phase B), so the small drift seen on prod (`co`: 510 cached vs 500 actual rows) auto-corrects on this deploy. No code change.

This deploy also re-aligns `/api/health` to report the workspace version (was reporting 1.38.2 because 1.38.3 was a watcher-only fix that didn't go through `flyctl deploy`).

## [1.38.3] — 2026-05-03

### Fixed — v2 watcher: deletes propagate (macOS FSEvents quirk) + multi-universe supervisor

`encode_event` now checks `abs_path.exists()` at flush time. macOS FSEvents sometimes reports `rm` as a `Modify` event rather than `Remove`, which the watcher was classifying as Upserted → tried to read the (now-missing) file → encode returned None → no delta sent → server still had the entry. Fixed: regardless of how notify classified the event, if the file no longer exists at flush time we emit a Deleted delta.

`scripts/co-watch-v2.sh` is the new launchd `ProgramArguments` — supervises one `co-agent-watch` per universe (4 sub-processes), refreshes the session cookie from keychain on 401. Replaces `scripts/co-watch.py` (v1 JSON/REST poll) in `~/Library/LaunchAgents/com.artelonga.co-sync.plist`.

**Verified end-to-end on prod (1.38.3):**
- Touch a file in `~/projects/co/` → on prod via `GET /entries/<path>` in ~2s
- Delete the file → 404 on prod in ~4s
- Zero feedback loop (broadcast filtered by `origin_conn_id`)
- 4 watchers connected to `wss://co-artelonga.fly.dev/api/v1/sync/ws` (one per universe), supervised by single launchd job

## [1.38.2] — 2026-05-03

### Fixed — CO-151 v2 watcher: relativized paths + broke broadcast feedback loop

Three bugs surfaced when running the v2 watcher end-to-end against prod:

1. **`tokio::task::spawn_blocking` killed `notify` on macOS.** FSEvents needs a thread with a CFRunLoop that lives for the whole stream; tokio's blocking pool tears those down. Switched to `std::thread::spawn`.
2. **Watcher sent absolute paths in `entry_path`** (e.g. `/Users/artelonga/co-watch-test/foo.md`). Server's `universe_root.join(absolute)` → still absolute, so writes landed outside the universe dir and `GET /entries/{rel}` returned 404. Reshaped `WatchEvent` into `{abs_path, rel_path, kind}` so the wire carries the relative path while disk reads still resolve via the absolute one. Added `relativize()` + `is_syncable()` filters.
3. **Server broadcast back to the originating client.** The watcher then ran `apply_batch` → wrote the file locally → `notify` fired → another upload → infinite loop. Added `BroadcastFrame { origin_conn_id, encoded }` and a per-room monotonic `next_conn_id`; the broadcast receiver loop skips frames where `origin_conn_id == self`. End-to-end loop count now bounded at 1.

Watcher tests updated for the new `WatchEvent` fields (7 pass). Server `sync_ws` tests still green (8 pass). Verified end-to-end on prod: write file → on prod via `GET /entries/<rel>` in <1s; no feedback loop in `~/.co/watch.log` after fix.

## [1.38.1] — 2026-05-03

### Fixed — CO-151 sync server now actually persists deltas

The 1.38.0 `apply_deltas_to_storage` called `Storage::update_entry_body`, which:
1. issues `UPDATE entries SET body=...` against `meta.db.entries` (a no-op since CO-77 moved entries to per-universe DBs), and
2. is UPDATE-only, so a delta for a *new* path silently did nothing.

Result: a v2 watcher could write `notes/hello.md`, watch the SyncDelta land on the broadcast log, and still see HTTP 404 from `GET /entries/notes/hello.md` because nothing actually persisted.

**Rewrote `apply_deltas_to_storage`** to use the same write path the Vault REST handler uses:
- `Kind::Upserted`: parse YAML frontmatter from the `CoFile` content, build an `Entry`, call `co::entry::write_entry` (writes the .md to disk under `data/universes/<aa>/<bb>/<key>/`), then `EntryIndex::upsert` against the per-universe `data.db`.
- `Kind::Deleted`: `std::fs::remove_file` the .md and `DELETE FROM entries` in the per-universe DB.

**Added `co-agent-watch` binary** (`co-agent/src/bin/watch.rs`) — wraps `SyncWatcher` in a CLI so the v2 launchd plist has something to actually run. The 1.38.0 plist referenced `~/.cargo/bin/co-agent-watch` which didn't exist.

**Fixed v2 watcher URL** (`co-agent/src/watcher.rs`) — was building `?token=...` only; server requires `?universe=<key>&token=<jwt>` and returned HTTP 400 otherwise.

**Regression test** `test_upserted_delta_writes_to_disk_and_db` proves the v2 write goes all the way through: WS upload → file on disk + per-universe row indexed + reachable via `/entries/{path}`.

## [1.38.0] — 2026-05-03

### Added — CO-151: real-time delta sync — protobuf SyncDelta over WebSocket with zstd

Bidirectional file-sync channel that streams deltas in a compact binary format, replacing the v1 JSON/REST poll approach in `scripts/co-watch.py`.

**Wire format** (`core/proto/sync.proto`):
- `SyncDelta` — one change (upserted / deleted / renamed) with a `CoFile` content envelope
- `SyncBatch` — batched deltas with resume token for reconnect replay

**Server** (`co-web/src/sync_ws.rs`):
- Route: `GET /api/v1/sync/ws?universe=<key>` (JWT or session cookie auth)
- Per-universe `SyncRoom` with 24h in-memory delta log for `X-Sync-Resume` replay
- Broadcast fan-out to all connected clients in the same universe

**Client** (`co-agent/src/watcher.rs`):
- FSEvents (macOS) / inotify (Linux) via `notify` crate with 200ms debounce
- Encodes local changes as `SyncDelta` and ships over the WS uplink
- Applies server-pushed downlinks to local files (last-write-wins)

**Compression** (`core/src/sync/delta.rs`):
- zstd level 3; placeholder for a ~32 KB training dictionary (CO-151 follow-up)
- proto+zstd wire bytes < JSON equivalent in all tests

**Migration**: `scripts/co-watch.py` (v1) stays operational; `scripts/co-sync-v2.plist` provides the replacement launchd configuration once `co-agent-watch` is deployed.

## [1.37.3] — 2026-05-03

### Fixed — `If-None-Match` short-circuit ran before existence check on `GET /assets/:sha`

The 304 fast path compared the URL sha against the `If-None-Match` header *before* looking up the row, so a probe like `curl -H 'If-None-Match: "X"' /assets/X` returned 304 for any valid 64-char hex sha — even when the row didn't exist. That broke client-side idempotency probes (a missing blob looked "already there" to the bulk uploader, which then mis-counted the run).

Reordered: row lookup first, then 304 short-circuit only if the row actually exists. Also simplified `scripts/bulk-upload-binary.py` to skip the probe entirely — the server is already idempotent on POST (same bytes → same sha → existing row reused), so the second `GET` was redundant.

Added regression test `if_none_match_on_nonexistent_returns_404_not_304` (14 asset integration tests total now).

## [1.37.2] — 2026-05-03

### Changed — home rewritten around the **Co**nsciência **Co**letiva philosophy

The previous home (`template/index.md`) opened with "uma plataforma para organizar ideias, projetos e pessoas em universos" — accurate but generic. The manifesto on `template/sobre.md` had the actual philosophy (Cocriar / Colaborar / Conectar) but lived a click away.

Merged both: the home now leads with **conectar pessoas** and the three verbs, defines what a universe is, then shows the curated trio diagram and the navigation primer. `sobre.md` is now a technical/governance page that points back to home for the philosophy.

`template/sobre.md` rewritten as a stack + community + license page, no philosophy duplication.

## [1.37.1] — 2026-05-03

### Fixed — bulk-upload usability: 413 on >2 MB assets, 429 saturating burst writes

Surfaced by the first quilomboaraucaria upload pass (401 binaries / 558 ok, 35 markdown / 95 ok). Two distinct failure modes:

1. **413 "Failed to buffer the request body: length limit exceeded"** on full-resolution images and MP4 (`*.orig.jpg`, `*.orig.png`, `*.mp4`, `fotos/post-*.jpg`). axum applies a 2 MB `DefaultBodyLimit` to all routes by default; the asset router's internal `MAX_ASSET_BYTES = 50 MB` never got a chance to run.
2. **429 rate_limited** on the markdown PUT burst — the per-bucket cap is 60 writes/min for `tier=user`, and a 95-file Vault dump trivially exceeds that.

**Fixes:**

- `asset_router()` now applies `DefaultBodyLimit::max(MAX_ASSET_BYTES)` (50 MB) on the router layer so the handler-level cap is the only gate.
- `rate_limit_middleware` now honors `X-Admin-Override-Quota: true` for authenticated callers (any tier ≠ Anonymous). CO-90 keeps `tier` billing-only, so the bypass is opt-in per request and the ownership/membership check still runs inside the route handler — anonymous callers can set the header but it's ignored. Bulk-upload script sends this header.
- `scripts/bulk-upload-binary.py` rewritten with `ThreadPoolExecutor` (8 workers), exponential backoff retry on 429/timeout/HTTP 0, idempotent skip-if-already-uploaded probe, and the override header. Same two-pass shape: binaries first to capture sha256, then markdown with `![](relative)` → `![](sha256:…)` rewriting.

After this deploy: a 50 MB JPG uploads cleanly, a 200-file markdown burst doesn't trigger 429, and a re-run of the same content is a no-op (sha256 idempotency + entry upsert).

## [1.37.0] — 2026-05-02

### Added — encrypted, indexable, lazy-loaded assets (CO-147 + CO-148 + CO-149 + CO-150)

Closes phases 2–5 of CO-145. Every blob written through the asset endpoint is now ChaCha20-Poly1305 ciphertext on disk, indexable by tags + mime + filename without decryption, range-fetchable for video and large media, and lazy-loaded by default in the SPA.

**CO-147 — indexable metadata**

```
GET    /api/v1/universes/{u}/assets?mime=&search=&tag=  → { assets: [{…, tags}], total }
GET    /api/v1/universes/{u}/assets/tags                → [{ tag, count }]
POST   /api/v1/universes/{u}/assets/{sha}/tags          body: {"tags": ["a","b"]}
DELETE /api/v1/universes/{u}/assets/{sha}/tags/{tag}
```

- New `asset_tags` table (FK CASCADE on assets); list endpoint joins per-asset tags into the response so the asset browser UI can render them inline.
- New `frontmatter_index` shadow table (title, type, status, tags_json, dates, parent_path) — designed to survive encryption-at-rest because typed metadata stays plaintext while body bytes get encrypted.

**CO-148 — encryption envelope**

- ChaCha20-Poly1305 AEAD, per-blob random 12-byte nonce, AAD = `universe_key || sha256` so a copied blob fails to decrypt across universes or under a different sha.
- Per-universe DEK derived deterministically: BLAKE3-keyed-hash(master_key, "co-asset-dek-v1\0" || universe_key). Master key sourced from `CO_ASSETS_MASTER_KEY` env (preferred) or `JWT_SECRET` (dev fallback). DEK never lands on disk.
- Schema additions: `assets.nonce BLOB`, `assets.cipher_size INTEGER`, `assets.encrypted INTEGER NOT NULL DEFAULT 0`. Old Phase 1 plaintext rows continue to read transparently; new uploads write ciphertext.
- **Threat model (Tier 1 — what ships):** disk-only attacker (stolen volume, leaked backup, accidental dump) cannot read content; needs the master key too. Real protection against backup leaks, dev-machine theft, S3 mistakes.
- **Threat model (Tier 2 — deferred):** server-trusted attacker with root + env still can. Closing this gap requires user-password-derived KEK with session-scoped DEK; filed as CO-148 follow-up.

**CO-149 — HTTP range support**

- `Range: bytes=N-M`, `bytes=N-`, `bytes=-N` all parse; multi-range rejected.
- 206 with `Content-Range: bytes N-M/total` + `Accept-Ranges: bytes`.
- 416 (Range Not Satisfiable) with `Content-Range: */total` for invalid ranges.
- Full-decrypt-then-slice: ChaCha20-Poly1305 verifies over the whole stream, so chunked-AEAD would change Phase 3's correctness story. Acceptable up to ~50 MB; chunked-AEAD for larger media is filed as future work.

**CO-150 — SPA lazy-load**

- `?excerpt=true` already shipped on entry GET (returns `{frontmatter, excerpt}` capped at 200 chars).
- Asset browser at `/co/{slug}/assets` consumes the new `GET /assets` endpoint.
- `markdown.js` post-processes rendered HTML in both fallback and full (marked + DOMPurify) paths: `<img src="sha256:abc…">` → asset URL + `loading="lazy" decoding="async"`; `<video src="sha256:…">` → asset URL + `preload="none"`; bare `<img>` tags get `loading="lazy"` if missing.
- Markdown source `![alt](sha256:abc…)` and ` ```video\nsha256:abc\n``` ` syntax both resolve through the renderer.

**Tests:** 13 asset integration tests (round-trip, dedupe, ETag/304, anon-on-private blocked, anon-on-public allowed, oversize rejection, ciphertext-on-disk, HTTP range 206 + suffix + 416, tag CRUD round-trip, delete-when-unreferenced) + 4 crypto unit tests + 6 asset_routes unit tests. Full co-web suite (380+) green; clippy clean.

---

## [1.36.0] — 2026-04-30

### Added — binary asset upload + content-addressable storage (CO-146, Phase 1 of CO-145)

Every universe now has a binary-asset endpoint backed by sha256 content addressing. Phase 1 of the encrypted+indexable+lazy-load epic (CO-145); designed to unblock the 506 MB quilomboaraucaria upload that the markdown-only Vault API rejects today.

**New endpoints:**

```
POST   /api/v1/universes/{u}/assets        body: raw bytes  → {sha256, mime, size, url}
GET    /api/v1/universes/{u}/assets/{sha}  → bytes + ETag + immutable cache
DELETE /api/v1/universes/{u}/assets/{sha}  → 204 if refcount == 0; 409 otherwise
```

**Storage layout:**

```
data/universes/<aa>/<bb>/<key>/
  data.db                          (existing)
  blobs/<aa>/<bb>/<sha256>         (new — raw bytes, sharded 2-level)
```

**Per-universe schema additions** (universe schema_v4):

```sql
CREATE TABLE assets (
    sha256        TEXT PRIMARY KEY,
    blob_path     TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    filename      TEXT,
    created_at_ns INTEGER NOT NULL,
    created_by    TEXT,
    refcount      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_assets_mime       ON assets(mime);
CREATE INDEX idx_assets_created_at ON assets(created_at_ns);
```

**Properties:**
- **Idempotent** — same bytes → same sha256 → single on-disk blob (re-upload is a no-op)
- **Atomic write** — write-tmp + rename, no torn writes on crash
- **Cache-friendly** — `Cache-Control: private, max-age=31536000, immutable` + ETag = sha256, with proper 304 short-circuit before disk read
- **MIME sniffing** — header takes precedence; falls back to magic-byte detection for jpeg/png/gif/webp/pdf/mp4
- **Auth** — write requires owner/member; read allows anon on public universes

**What Phase 1 does NOT do** (deferred to subsequent CO-145 phases):
- Encryption at rest — CO-148 wraps every blob in ChaCha20-Poly1305 with per-universe DEK + owner KEK derived via Argon2id
- Indexable list/filter endpoint — CO-147 adds `GET /assets?type=image/*&tag=foo`
- HTTP range support — CO-149 adds streaming for video and large images
- SPA `<img loading="lazy">` integration — CO-150

Phase 1 stays plaintext deliberately because the existing `entries.content` column is also plaintext — the privacy gap is not widened. Encryption ships in Phase 3 (CO-148) once the upload path itself is proven.

**Hard cap:** 50 MB per blob (axum's default body limit aligns; oversize returns 400/413). CO-149 + CO-148 Phase 6 lift this with chunked-AEAD streaming.

**Tests:** 7 integration tests (`co-web/tests/asset_tests.rs`) cover round-trip, dedupe, ETag/304, anon-on-private blocked, anon-on-public allowed, oversize rejection, delete-when-unreferenced. Plus 6 unit tests for hex encoding, MIME sniffing, and shard-path construction.

**Design doc:** `docs/research/encrypted-indexable-assets.md` documents the full 5-phase plan including the index-plaintext / encrypt-body split, key hierarchy, and lazy-load wire contract.

**Filed tickets:** CO-145 (epic), CO-146 (this), CO-147, CO-148, CO-149, CO-150.

## [1.35.2] — 2026-05-02

### Fixed — recovery from buggy `prune_orphan_universe_dirs` (1.34.5 regression)

**Critical regression introduced in 1.34.5:** the previous `prune_orphan_universe_dirs` iterated all top-level dirs under `/data/universes/` and deleted any whose name didn't match a `universes.key` row. That was wrong — `UniversePool` (CO-77) shards per-universe `data.db` files at:

```
/data/universes/<2-hex>/<2-hex>/<key>/data.db
```

The 2-hex shard-prefix dirs (e.g. `68`, `b5`, `0e`, `f0`) are NOT universe keys — they're hash-prefix dirs holding multiple per-universe DB files. Deleting them wiped the per-universe SQLite for affected universes (template, quilomboaraucaria, artelonga, humanity, universo). The flat `.md` content survived (`/data/universes/<key>/*.md`) — the SQLite shards got lazily recreated empty by `UniversePool::get_or_open` on first access, returning `entries.total=0` to the API.

**Two fixes shipped in 1.35.2:**

1. **`prune_orphan_universe_dirs` rewritten as narrow allowlist.** Now only deletes dirs whose name matches the explicit `KNOWN_DEPRECATED_DIRS` list (`co-dev`, `co-experience`, `qa-dev`, `quilombo-blog{,-2,-3}`, plus a few test/anon residues). Wider cleanup is now an explicit ops task, not unattended boot-time. Defensive double-check that no `universes.key` row holds the name before deleting.

2. **`rebuild_entries_from_filesystem(keys: &[&str])`** — recovery pass for the affected universes. Walks `/data/universes/<key>/**/*.md`, parses frontmatter, upserts each entry into the per-universe `data.db` via `universe_pool`. Idempotent — skipped per-universe when entries table already has rows. Wired into `server.rs` startup for system universes: template, tempo, humanity, universo, quilomboaraucaria, artelonga, rfq, co, yuri, dados.

After 1.35.2 boot:
- `entries.total` for template/quilomboaraucaria/artelonga restored from the .md filesystem
- `content_count` recomputed accurately
- No data loss (filesystem was always the source of truth; only the SQLite mirror was wiped)

## [1.35.1] — 2026-05-02

### Fixed — `UniverseInfo` exposes `content_version` + smoke script Python compatibility

Two follow-ups during the 1.35.0 smoke pass:

1. **`UniverseInfo` DTO missing `content_version`.** Same shape as the CO-137 parent_key bug — the column existed and the data was correct, but the public DTO didn't surface it. Added `content_version: String` (defaults to "0.0.0") and a defensive separate `SELECT` in `get_universe_info` that tolerates a missing column.

2. **`scripts/smoke-processo-alterar-pagina.sh` Python f-string syntax.** Older Python (<3.12) doesn't allow `\"` escapes inside f-string expressions. Switched to `'  ...{} ...'.format(...)` form. Script now runs end-to-end against any Python 3.6+.

After this deploy, `GET /api/v1/universes/<key>` returns `content_version` in the JSON body.

## [1.35.0] — 2026-05-02

### Added — `alterar-pagina-na-web` process implemented end-to-end (CO-144 Phase C)

All 7 chain steps are now wired in the live binary, exercisable via REST:

```
POST   /api/v1/processos/alterar-pagina-na-web/preview
POST   /api/v1/processos/alterar-pagina-na-web/approve/{run_id}
POST   /api/v1/processos/alterar-pagina-na-web/revert
GET    /api/v1/processos/alterar-pagina-na-web/runs?universe=<key>
```

- **Step 1 — Trigger:** `POST /preview` with `{universe, page_path, field, new_value, bump_level?}`
- **Step 2 — Source:** server reads the current entry from filesystem via `co::entry::read_entry` (source of truth)
- **Step 3 — Review:** preview row inserted into new `process_runs` table with state=preview, returns diff + run_id + computed `proposed_version`
- **Step 4 — Approval:** `POST /approve/{run_id}` re-validates state, re-checks write access, then proceeds to sink
- **Step 5 — Sink:**
  - 5.1 Frontmatter field updated, `co::entry::write_entry` persists to FS
  - 5.2 `universes.content_version` bumped (semver patch by default; `minor`/`major` via `bump_level`)
  - 5.3 `<universe>/CHANGELOG.md` appended with the structured entry (creates with header if missing)
  - 5.4 Deploy: simulated for now (real target adapters are CO-134 static-on-R2, CO-135 CF Pages, etc.)
- **Step 6 — Telemetry:** `telemetry_events` row with `event_type='process'`, `event_name='alterar-pagina-na-web.completed'`; run state → completed
- **Step 7 — Rollback:** `POST /revert` with `{universe, target_version}` (or `"prior"` to use the most recent prior). Restores frontmatter from the parent run's `from_value`, rolls back `content_version`, appends a "Reverted" CHANGELOG entry, marks parent run state='reverted', emits `alterar-pagina-na-web.reverted` event.

Run history queryable via `GET /runs?universe=<key>` — returns ordered list with full payload, parent_run_id linkage, state.

### Schema additions (auto-applied via `ensure_*` backfill)
- `universes.content_version TEXT NOT NULL DEFAULT '0.0.0'` — per-universe semver
- `process_runs` table — run_id, process_name, universe_key, state, payload (JSON), timestamps, actor_id, parent_run_id
- Index `idx_process_runs_universe_time`

### Acceptance for the worked example (Co/processos/alterar-pagina-na-web)
- [x] All 7 steps execute against a real universe end-to-end
- [x] CHANGELOG.md lands in the universe root with structured entries
- [x] Revert restores prior frontmatter + version + emits inverse event
- [x] State machine prevents double-approval and approval after revert
- [x] Access-checked: write permission required for preview/approve/revert
- [ ] SPA dashboard render of the run history (Phase D — separate ticket)
- [ ] Real deploy target (current: simulated) — depends on CO-134/135 adapter completion

## [1.34.8] — 2026-05-02

### Added — `Co/processos/alterar-pagina-na-web` + recursive ingest of co universe

User clarification 2026-05-02: the per-user dashboard work (CO-144) needs to encompass a deterministic source→sink **process model**, with `Co/processos/alterar-pagina-na-web` as the worked example.

- **CO-144 expanded** (`work/co/CO-144.md`): now 4 phases — A (auto-create personal universe + dados/ skeleton), B (cross-universe activity feed populating `<username>/dados/`), C (process model with `Co/processos/<process>` content type and reflexive editing pattern), D (SPA dashboard + process stepper rendering). Architecture diagram + decision log added.
- **`work/co/processos/alterar-pagina-na-web.md` committed** (246 lines): documents the 7-step deterministic chain — Trigger → Source → Review (`co preview` localhost v+1) → Approval → Sink (manifest bump + CHANGELOG + deploy) → Telemetry (3 sinks) → Rollback. Includes a Mermaid source→sink flowchart, structured event schema, source-to-sink data sync table, edge cases. State-of-implementation table marks each step ❌/🟡/✅.
- **`Storage::seed_co_universe_tasks` now recursive**: walks `/app/seed-co/` and preserves subdir structure. Top-level `*.md` keep the `tasks/<filename>` prefix for backwards compat with 1.34.3; deeper files use their relative path (e.g. `processos/alterar-pagina-na-web.md`).

After deploy, the SPA's `/co/co/processos/alterar-pagina-na-web` resolves to the worked example.

## [1.34.7] — 2026-05-02

### Fixed — `*@co.local` legacy users blocked admin from claiming their slug

1.34.6 surfaced the unique-index conflict on `users.username`: admin `yuri@artelonga.com.br` couldn't claim `yuri` because the legacy `yuri@co.local` test user held it.

**Fix:** new `Storage::free_legacy_co_local_usernames()` runs before `ensure_admin_username` on every boot. Renames any `*@co.local` user's username to `legacy-<original>` (e.g. `yuri` → `legacy-yuri`). Idempotent — `WHERE username NOT LIKE 'legacy-%'` keeps re-runs as no-ops.

After this deploy, the admin's username is set to `yuri` on next boot, completing the "always use slug as user name by default" directive.

## [1.34.6] — 2026-05-02

### Added — admin's `yuri` personal universe re-homed + username default

User feedback 2026-05-02: "include the private yuri user (always use slug as user name by default)". The `yuri` universe and `dados` dashboard universe were misclassified as cruft in my earlier note — both are intentional and intact (correctly preserved by `prune_orphan_universe_dirs` since their DB rows exist). What was actually wrong:

- `yuri@artelonga.com.br` (admin) had `users.username = ''` (empty)
- The `yuri` universe was owned by `usr_-PFeKIctDZ` (legacy `yuri@co.local` test user that previously held the username slug)
- New `Storage::ensure_admin_username(email)` derives the slug from the email prefix (`yuri@artelonga.com.br → yuri`), updates `users.username` if empty. Skips gracefully on unique-index conflict — does not break boot.
- `PERSONAL_KEYS` (in server.rs admin-bootstrap path) now includes `yuri` alongside `artelonga` and `rfq`. Next boot re-homes the `yuri` universe to the admin's `user_id` via `ensure_admin_owns_personal_universes`.

### Filed — CO-144: per-user dashboard universe + cross-universe activity feed

3-phase ticket scoping the broader feature the user described: "it works like a dashboard, changing a file in other universes or adding a new universe populates that, obviously one (private) per user".

- **Phase A** — every signup auto-creates a private universe with `key = users.username` (extends the admin-only pattern shipping in 1.34.6 to every user)
- **Phase B** — `upsert_entry_row` emits cross-universe events that materialize into (i) the existing global `dados` universe and (ii) each user's slug-named universe, filtered by membership/subscription
- **Phase C** — SPA Painel-style dashboard view that renders the activity feed with universe / actor / entry-type filters

Decision recorded: `dados` stays system-owned (global aggregate). Per-user dashboards are the user's slug-named private universe.

## [1.34.5] — 2026-05-02

### Added — `prune_orphan_universe_dirs` filesystem cleanup on every boot

Closes the filesystem-cruft gap surfaced after CO-142 Phases C+D hard-deleted DB rows for `co-dev`, `co-experience`, `qa-dev`, `quilombo-blog{,-2,-3}` (and various test/anon dirs) — the dirs at `/data/universes/<key>/` persisted, accumulating cruft.

`Storage::prune_orphan_universe_dirs()` runs after the seed/delete/recompute passes on every boot. For each entry under `/data/universes/`, checks if a row exists in `universes` for that key; if not, removes the dir. Idempotent — already-removed dirs are no-ops. Safe — anonymous clones (hash-keyed dirs that have a corresponding `anon-*` row) are kept.

### Done — CO-100 documentation pass for 1.34.x reality

`docs/ARCHITECTURE.md` updated from 1.21.x snapshot to current state:
- C4 component diagram now includes co-agent (CO-120), ClickHouse (CO-123), Cloudflare CDN+WAE (CO-117), admin surface (CO-105), and the per-universe SQLite split (CO-77)
- New "Armazenamento (1.23+)" section documenting the meta.db / per-universe data.db topology, WAL-safe snapshot rules, idempotent migrations (`ensure_column` / `ensure_table`)
- New "Endpoints novos (1.22 → 1.34)" table covering admin / A/B / log-drains / cache / themes / generic entries
- New "Componentes opcionais" section on co-agent, ClickHouse, backup-cron, Cloudflare
- New "Evolução desde 1.21.x" cross-reference table mapping each shipped feature to its commit/file location
- Service worker updated `co-v3-network-first` → `co-v4-offline`

CO-100 frontmatter: `in_progress` → `done`.

### Repository

`github.com/artelonga/co` flipped from PRIVATE → PUBLIC. Pre-publish audit: `.claude/` files (Claude Code session state, never repo-content) untracked + added to `.gitignore`. No actual prod secrets in git history; the only "secret-shaped" mention was `JWT_SECRET=dev-test-secret` as a Bash command-pattern allow-list value in `.claude/settings.local.json` — placeholder, not a real secret. Privacy-page links pointing at the source (`https://github.com/artelonga/co/...`) now resolve for anonymous browsers, fulfilling the "verifiable" promise in `dados-rastreados.md`.

## [1.34.4] — 2026-05-02

### Fixed — `seed_admin_content_universes` reconciles visibility on every boot

Discovered during 1.34.3 staleness audit: `artelonga` returned 404 to anonymous despite the seed declaring `public-subscribable`. Root cause: `INSERT OR IGNORE` doesn't update existing rows, so a row created with an old default (`private`) silently stays wrong forever. Same risk for any future visibility intent change on these admin-content universes.

**Fix:** added a follow-up `UPDATE universes SET visibility = ?, is_public = ? WHERE key = ? AND (visibility != ? OR is_public != ?)` to `seed_admin_content_universes`. Only writes when the stored value doesn't match declared intent — idempotent on every boot. `is_public` bit kept in sync (0 for private, 1 otherwise) so legacy callers checking that flag also see the intended state.

After this deploy:
- `artelonga` → public-subscribable, reachable to anonymous (was 404)
- `rfq` → private (unchanged)
- `co` → public-subscribable (unchanged)

### Fixed — Stale GitHub URL in `termos.md`

`seed/template/termos.md:98` still pointed at the renamed `data/universes/template/content/termos.md` path (instead of `co-web/seed/template/termos.md`). Same class as the privacidade and dados-rastreados fixes from 1.34.3 — completes the audit of stale GitHub paths in the legal-pages corpus.

## [1.34.3] — 2026-05-02

### Fixed — `co` universe shows 0 entries despite 140 task markdown files

User report on 2026-05-02: "co has 0 entries, we have 140 tasks". CO-142 Phase E populated `/data/co/` from the bundled `/app/seed-co/` for the admin dev_board scan, but the SPA's `/co/co` board reads from the per-universe `entries` table (CO-77) — which stayed empty.

**Fix:** new `Storage::seed_co_universe_tasks(source_dir)` runs on every boot after Phase E's `copy_dir_all`. Iterates `/app/seed-co/*.md`, builds an `Entry` via the existing `make_entry` + `seed_page_frontmatter` helpers, writes via `co::entry::write_entry`, upserts via `upsert_entry_row` against the per-universe pool's `co` connection. Path layout: `tasks/CO-NNN.md`. Idempotent.

After this fix, `GET /api/v1/universes/co/entries` returns 140+ ticket entries.

### Fixed — Política de Privacidade link broken from termos.md

Internal markdown link in `seed/template/termos.md` was `/co/template?path=content/privacidade.md` but the SPA only recognizes `?page=<slug>` (handled by `maybeOpenPageFromUrl`). Anonymous users clicking the link landed on the template board with no modal opening.

- `seed/template/termos.md` — link corrected to `/co?page=privacidade`
- `seed/template/privacidade.md` — fixed the "histórico de versões" GitHub URL from the renamed `data/universes/template/content/privacidade.md` to the current `co-web/seed/template/privacidade.md`

## [1.34.2] — 2026-05-02

### Fixed — CO-142: public-universe routing audit + co-dev/co-experience deprecation

Five-phase cleanup of the public-universe surface:

**Phase A — Routing fix**
- Moved `dev_board::router()` from `/api/v1/universes` to `/api/v1/admin` so it
  no longer shadows the public-subscribable universe lookup via `universe_api`.
  Dev board routes are now at `/api/v1/admin/co-dev/…`.
- Retargeted the telemetry SPA route from `/co/co-dev/telemetria` to
  `/co/co/telemetria` (reflects the `co` work universe replacing `co-dev`).
- Added smoke-check [11]: every public universe (`template`, `quilomboaraucaria`,
  `co`, timeline trio) must return 200 to anonymous.

**Phase B — content_count reconciliation**
- Added `recompute_content_counts()`: on every boot, counts entries in each
  universe's per-universe DB and writes the result to `universes.content_count`.
  Fixes `template.content_count = 0` caused by `reseed_template_content_pages`
  calling `upsert_entry_row` without `increment_universe_content_count`.
- Added smoke-check [12]: `template.content_count >= 6`.

**Phase C — co-dev / co-experience deprecation**
- Removed `seed_co_dev_universe()` call from startup.
- Added `delete_deprecated_universes()`: hard-deletes `co-dev` and `co-experience`
  rows (and memberships) on every boot. Idempotent.
- Removed `co-dev` and `co-experience` from `ensure_admin_universe_memberships`
  system_keys and from `uat_mirror` skip list.
- **Decision**: epics stay as entries in the `co` universe (not promoted to
  sub-universes). Documented in `docs/UNIVERSES.md`.

**Phase D — Quilombo reconciliation**
- Added `delete_stale_quilombo_variants()`: hard-deletes `quilombo-blog`,
  `quilombo-blog-2`, `quilombo-blog-3`, and `qa-dev` on every boot.
- Created `docs/UNIVERSES.md`: canonical inventory of all system universes,
  with documented purpose and seed path for each.
- Removed `qa-dev` from `PERSONAL_KEYS` in the admin bootstrap sequence.

**Phase E — Dev board task display**
- Added `COPY work/co/ /app/seed-co/` to the Docker runtime stage so the
  repo's `work/co/CO-*.md` files are bundled in the image.
- Added startup refresh: on every boot, `copy_dir_all(/app/seed-co, data/co/)`
  keeps the dev board in sync with the repo's task statuses.
- Documented all startup invariants in `docs/OPERATIONS.md`.

## [1.34.1] — 2026-05-02

### Fixed — `dados-rastreados` page refreshed for 2026-05 cookie surface

Following user feedback ("Dados is not up to date"), updated the privacy disclosure to reflect cookies and localStorage state added since 1.21.x:

- Date stamp: abril → maio de 2026
- Added cookies: `co_onboarded` (CO-99 onboarding), `co_cookie_consent` (LGPD banner), `co_preferred_universe` (auto-redirect to last universe)
- Added new section §3.1 enumerating localStorage / IndexedDB state (`co_universe_tree_*` from CO-98 hierarchy, `co_subtree_*`, `co_section_*`, `co_folder_*`, `co_draft_*` autosave drafts, `co-vault` IDB cache from CO-69 PWA offline)
- Fixed the "verifiable source" link from `data/universes/template/content/dados-rastreados.md` (renamed years ago) to `co-web/seed/template/dados-rastreados.md` (current path)

### Filed — CO-142: public-universe routing audit + co-dev/co-experience deprecation

Five-phase ticket scoping the architecture-level cleanup the user named on 2026-05-02:

- **Phase A** — disambiguate `/api/v1/universes/co-dev` shadow (dev_board admin middleware vs. public-subscribable universe)
- **Phase B** — fix `content_count=0` on `template` (and likely other system universes) — recompute on boot or atomic via upsert
- **Phase C** — deprecate `co-dev` / `co-experience` public universes; migrate to epic ↔ sub-universe via CO-98 `parent_key`
- **Phase D** — reconcile quilombo* and qa-* universe sprawl into a documented set
- **Phase E** — wire the dev board to read from `work/co/CO-*.md` so completed tickets actually show as done

Each phase has explicit acceptance criteria and call-out of the underlying mechanism (route mounting order in `server.rs`, `upsert_entry_row` count maintenance, `parent_key` semantics, deploy-time path mounts). No code changes in this commit — ticket only.

## [1.34.0] — 2026-05-01

### Added — CO-105: Admin telemetry dashboard

Cherry-picked + integrated from the long-running `feat/CO-105` branch (1 commit, originally branched at 1.27.0). Resolves on top of current main; conflict markers in `Cargo.toml`, `co-web/src/lib.rs`, `co-web/src/server.rs`, `Cargo.lock`, and `CHANGELOG.md` were resolved by accepting HEAD's structure and adding the new admin module alongside (not replacing) the post-1.27.0 routes (`/api/v1/ab`, `/api/v1/cache`).

**`GET /api/v1/admin/dashboard`** — single JSON endpoint with platform-wide aggregates:
- JWT required; caller email must match `CO_SEED_ADMIN_EMAIL` (read fresh from env per request)
- Returns 401 for invalid/missing JWT, 403 for email mismatch — never leaks admin email on invalid signature
- Totals: users, universes, active universes (7d), entries
- Daily rows (14 days): pageviews, unique visitors, signups, errors — sourced from `telemetry_events` + `users.created_at`
- Top 10 universes by event count (7d) with name fallback
- Auth stats: logins today, failed logins, active sessions (last 30 min)
- 5-minute in-memory cache; no DB writes per request

**`GET /admin`** — static admin page (cookie auth):
- Server-side JWT + email gate: redirects to `/co` if unauthenticated, 403 if not seed admin
- Plain HTML, no framework, no CDN — inline CSS + JS, `< 10 KB`
- Top strip: users, universes, active-7d, entries as big numbers
- Daily traffic sparkline (last 14 days): dual polyline SVG (pageviews + uniques)
- Top universes table (key, name, events)
- Auth panel: logins today / failed / active sessions
- Auto-refreshes every 60 seconds

**`co-web/static/variants/a/admin.html`** — embedded via `include_str!` at compile time.

**`co-web/src/admin_routes.rs`** — new module with typed structs, aggregate query helpers, handlers, and 21 unit + integration tests.

## [1.33.2] — 2026-05-01

### Added — `ensure_table` helper to formalize the migration-drift safety pattern

Sibling of `ensure_column` (CO-137). Queries `sqlite_master` before issuing the DDL; returns `true` if the table was created, `false` if it already existed. The standalone `CREATE TABLE IF NOT EXISTS` SQL is already idempotent, so the helper exists primarily to give callers a single, consistent surface for migrations and to make adding observability (tracing / metrics) trivial at the call site.

Callers updated:
- CO-77 `entries` + `entries_fts` backfill — now uses `ensure_table` per table
- CO-121 `feature_flags` + `ab_assignments` + `ab_exposures` backfill — now uses `ensure_table` per table
- The `idx_exposures_flag_time` index stays on `CREATE INDEX IF NOT EXISTS` (indexes aren't tracked as `sqlite_master.type='table'`).

Closes the structural follow-up the 1.33.1 hotfix opened: every CREATE TABLE migration that ships now has a single, consistent helper to call. Combined with `ensure_column`, the framework is structurally robust against the partial-application failure mode that bit prod three times (CO-77, CO-137, CO-121).

## [1.33.1] — 2026-05-01

### Fixed — A/B `feature_flags` table missing on prod (CO-121 partial-apply hotfix)

**Symptom on prod (1.33.0):** every boot logs `ERROR co_web::server: CO-121: failed to seed feature flags: no such table: feature_flags`. Same partial-application failure mode as CO-137 / 1.22.4: `schema_version` row exists for v27 but the corresponding `CREATE TABLE feature_flags` never took effect on this DB. Boot proceeds but A/B endpoints would 500 on first use.

**Fix:** unconditional post-migration backfill at the end of `Storage::run_migrations` — `CREATE TABLE IF NOT EXISTS feature_flags / ab_assignments / ab_exposures` plus the `idx_exposures_flag_time` index. Mirrors the existing CO-77 (`entries`) and CO-137 (`parent_key`) backfills. Idempotent; safe to re-run on every boot.

This is the **third** instance of the same migration-drift class (CO-77 entries, CO-137 parent_key, CO-121 feature_flags). Pattern is now formalized in `feedback_migration_column_reads.md`: every CREATE TABLE + ALTER ADD COLUMN that ships should also have an unconditional backfill at the end of `run_migrations` for at least one release cycle, until prod has visibly converged.

After this fix:
- Prod boot logs no longer carry `no such table: feature_flags`
- A/B exposure logging works without surfacing a 500 to anonymous template visitors

## [1.33.0] — 2026-05-01

### Added — CO-123: ClickHouse single-node + WAE export pipeline

- `infra/clickhouse/` — Fly app config, ClickHouse config/users XML, `init.sql` (wae_events MergeTree, 90-day TTL, Iceberg table function ready)
- `scripts/wae-to-clickhouse.sh` — daily WAE SQL API → ClickHouse bulk insert; maps CF Analytics Engine columns to typed schema
- `infra/clickhouse-export-cron/` — Alpine Fly cron app running export at 04:17 UTC
- `infra/clickhouse/iceberg-smoke-test.sh` — validates Iceberg S3 integration via ClickHouseS3 table function
- `docs/analytics/sample-queries.sql` — 8 ready-to-run queries (top universes, error rate, A/B funnel, p95 latency, retention)
- `docs/OPERATIONS.md` §ClickHouse — full runbook: setup, proxy, querying, export schedule, smoke test

## [1.32.0] — 2026-05-01

### Added — CO-124: Co-agent variants for CF Workers tail + Vercel Log Drains

- **CF tail Worker** (`workers/co-tail/`) — Cloudflare-native tail Worker that subscribes to a
  target Worker's log stream, converts events to CO `TelemetryEvent` JSON-Lines, gzip-compresses,
  signs with HMAC-SHA256, and POSTs to the CO ingest endpoint; deployable via `wrangler deploy`
- **Vercel Log Drain receiver** — `POST /v1/log-drains/vercel/{universe_id}` route on co-web:
  validates Vercel `x-vercel-signature` (HMAC-SHA1), maps NDJSON log entries to CO events, and
  stores them in `log_drain_events` with idempotent deduplication by `event_id`
- **Schema migration v28** — `log_drain_secret TEXT` column on `universes`; new `log_drain_events`
  table with `event_id` primary key and composite index on `(universe_id, received_at)`
- **Documentation** — `docs/co-agent/cloudflare-workers.md` and `docs/co-agent/vercel.md`

## [1.31.0] — 2026-05-01

### Added — CO-97: Visitor token unification (Option A)

- `telemetry_middleware` and `quilombo_telemetria` read `al_vid` first, fall back to `visitante_id`
- Both middlewares emit `al_vid` scoped to `.artelonga.com.br` (JS-readable, `SameSite=Lax; Secure`)
- `HttpOnly` intentionally dropped on visitor token — analytics-only, no auth role (see ADR-001)
- `docs/decisions/001-visitor-token-unification.md` — decision record with trade-off sign-off
- `dados-rastreados.md` updated to disclose `al_vid` cookie and scope

## [1.30.0] — 2026-05-01

### Added — CO-79/80/108/109/118/121/122: Wave 3-5 + platform infra

- **CO-79** — Caching layer: in-process manifest LRU, theme-css ETag, query singleflight, cache-hit metrics
- **CO-80** — Per-tier rate limiting: token-bucket per user/tier/operation; `/api/v1/ab` admin routes wired
- **CO-108** — Universe archive format + backup-to-external-HD scripts
- **CO-109** — Mbya Guarani stress-test universe: lexicon → markdown corpus, UAT seed
- **CO-118** — Workers Analytics Engine: `WaeEmitter`, Cloudflare Worker proxy
- **CO-121** — A/B primitives: `feature_flags`, `ab_assignments`, `ab_exposures`, admin routes
- **CO-122** — Quota/tier model spec in `docs/QUOTAS.md` (no enforcement yet)

### Fixed

- `has_data()` dual-check guards CO-77 first-boot false-negative (prod incident 2026-05-01)
- Cache timing test budget relaxed 1 ms → 10 ms for parallel CI runs

## [1.29.0] — 2026-05-01

### Added — CO-69: PWA offline — IndexedDB cache + Background Sync

**offline.js** (`static/shared/offline.js` — new file)
- IndexedDB schema `co-offline-v1` with `entries` store (keyed by `[universe_key, path]`, LRU-indexed) and `pending_writes` store (autoIncrement)
- `window.fetch` intercept for PUT/POST to `/api/v1/universes/*/entries*` and `/vault/*`: writes to IDB immediately (optimistic cache), tries network, queues on failure and registers Background Sync tag `co-vault-writes`
- `flushPendingWrites()` — replays pending queue; called on `online` event and manual sync button
- `updateOfflineBanner()` — shows/hides the conflict banner with pending write count; i18n-aware (pt/en)
- `beforeinstallprompt` capture + `showInstallPrompt()` for PWA home screen install
- SW `CO_SYNC_COMPLETE` message listener → refreshes banner after background sync

**Service worker** (`static/shared/sw.js`, `static/sw.js`)
- CACHE_NAME bumped `co-v3-network-first` → `co-v4-offline` (triggers cache refresh on deploy)
- `handleVaultGet` — GET `/api/v1/universes/*/vault/*`: checks IndexedDB first, falls back to network, populates cache on success
- `sync` event handler (`co-vault-writes` tag): replays `pending_writes` from IDB with credentials, stops on first network failure to prevent thundering herd; notifies all clients via `CO_SYNC_COMPLETE`

**index.html** (`static/variants/a/index.html`)
- Offline conflict banner (`#offline-sync-banner`): fixed top bar with pending count, "Sincronizar" button, dismiss; hidden via `style.display`
- Install button (`#btn-install-pwa`): shown in header when `beforeinstallprompt` fires; triggers native install prompt

## [1.28.0] — 2026-05-01

### Added — CO-104: Backup automation — daily snapshot of SQLite + universes/ to S3

**Scripts**
- `scripts/backup-prod.sh` — atomic SQLite snapshot via `.backup` + `universes/` tarball, uploads both to S3 (`co.db/<date>.db`, `universes/<date>.tar.gz`); idempotent, no interactive prompts
- `scripts/restore.sh` — restores from S3 (date mode) or local file; added **production safety guard**: fails loud if target is `co-artelonga` without `--yes-i-want-to-overwrite-prod`; restores both SQLite and `universes/` tarball when pulling from S3

**Cron automation**
- Option A: `infra/backup-cron/` — Alpine Fly app running `crond` at 03:17 UTC; self-contained image with `flyctl` + `aws-cli`; `fly.toml` + `Dockerfile` + `entrypoint.sh`
- Option B: `.github/workflows/backup.yml` — GitHub Actions daily cron at 03:17 UTC; `workflow_dispatch` for on-demand runs; requires `BACKUP_AWS_ACCESS_KEY_ID`, `BACKUP_AWS_SECRET_ACCESS_KEY`, `FLY_API_TOKEN` secrets

**Infrastructure**
- `infra/s3/lifecycle.json` — S3 lifecycle: STANDARD_IA after 30 days, delete after 365 days
- `infra/s3/setup.sh` — idempotent bucket setup: create, block public access, SSE-S3 encryption, lifecycle

**Documentation**
- `docs/OPERATIONS.md` — "Backup & restore" section rewritten with full runbook: S3 layout, on-demand backup, restore with prod guard, cron options, restore-drill, first-run checklist

## [1.27.0] — 2026-04-30

### Added — CO-73: Temporal model — first-class semantic dates (event_at, due_at, scheduled_at, …)

**`DateSemantic` enum expansion (`core/src/manifest.rs`)**
- Renamed `Due/Event/Created/Updated` → `DueAt/EventAt/CreatedAt/UpdatedAt` to match canonical `_at` names
- Added four new semantics: `ScheduledAt`, `PublishedAt`, `ExpiresAt`, `EffectiveAt`
- Added `DateSemantic::as_str()` returning the canonical query-param string (e.g. `"event_at"`)

**`entry_dates` table (per-universe `data.db`, migration v2)**
- Schema: `(universe_key, entry_path, semantic, value TEXT NOT NULL UTC ISO-8601)` with PK
- Index `idx_entry_dates_range ON (universe_key, semantic, value)` for O(log N) range queries
- Created idempotently on every DB open; version bumped to 2 on first migration

**Write hook (`co-web/src/entry_index.rs`)**
- `upsert_dates(universe_key, entry, manifest)` — extracts all `Date` fields with a declared semantic from the manifest, normalises values to UTC RFC3339, upserts into `entry_dates`
- `remove_dates(universe_key, path)` — clears all `entry_dates` rows on DELETE
- `normalize_date_to_utc(s)` — accepts full RFC3339 and `YYYY-MM-DD`; returns `None` on parse failure
- Hook wired into `create_entry`, `update_entry`, `delete_entry` in `entry_routes.rs`

**Date-semantic query API**
- `GET /api/v1/universes/:slug/entries?date_semantic=event_at&from=2026-01-01&to=2026-12-31`
- JOINs `entry_dates` on `(universe_key, semantic)` with optional `>= from` / `<= to` bounds
- Results ordered by date ascending; hard cap 500

**Manifest API endpoint**
- `GET /api/v1/universes/:slug/manifest` — returns parsed `_universe.yaml` as JSON; falls back to `default_manifest` when no file exists

**Calendar view upgrade (frontend)**
- Detects manifest `presentation.calendar.date_field`; fetches entries via date-semantic API when declared
- Renders entries (not just tasks) in calendar cells; normalises UTC to user's local timezone via `Intl.DateTimeFormat`
- Legacy `due_date`-from-tasks rendering preserved as fallback

**Gantt view (frontend)**
- Manifest-declared `views: [{ type: gantt, date_start: X, date_end: Y }]` injects a tab automatically
- `renderGantt(viewDef)` renders horizontal bars spanning `date_start` → `date_end` per entry
- Today marker, month labels, responsive bar widths; no code changes needed for new Gantt views

**Timezone support**
- Server stores UTC; browser renders in user's timezone via `Intl.DateTimeFormat().resolvedOptions().timeZone`

### Added — CO-61: Sync Protocol v1 — op log + content-addressed blobs + 3-way merge + recursive resolution

**Spec document (`docs/sync-protocol-v1.md`)**
- 706-line canonical specification covering: op log shape, HLC semantics, content-addressed blob store, 3-way merge algorithm, recursive conflict resolution, idempotency/atomicity guarantees, REST transport, auth (v1.0 shared secret / v1.1 federation reserved), and reducer rules
- Explains the PR analogy (Proposta ≅ pull request), prod-wins default policy, and copia semantics
- Full compatibility mapping with CO-51/54/55/58/66/68 sync tracks

**JSON Schemas (`docs/sync-protocol-v1/schemas/`)**
- `hlc.json` — Hybrid Logical Clock serialized as `wall_ms:counter:node_hex32`
- `ator.json` — Actor identity (node_id + optional user_id)
- `alvo.json` — Addressed entity + optional field
- `operacao.json` — Single immutable op with causal parents
- `manifesto.json` — Peer state summary for divergence detection
- `proposta.json` — Sync proposal (batch of ops from sender)
- `conflito.json` — Detected conflict with resolution options
- `relatorio_mesclagem.json` — Merge report returned to sender

**Test vectors (`docs/sync-protocol-v1/fixtures/`)**
- 10 fixture files covering: clean apply, independent advances, same-slot conflicts, resolver ops, delete-vs-update (copia), idempotent dedup, nested conflicts, causal ancestry, schema migration, and resolver reversibility

**Rust skeleton (`core/src/sync/mod.rs`)**
- Types: `Hlc`, `Ator`, `Alvo`, `Operacao`, `Manifesto`, `Proposta`, `Conflito`, `RelatorioMesclagem`
- `SyncProtocol` trait for implementors
- `mesclar()` / `mesclar_com_blobs()` — pure 3-way merge function with dedup, causality-aware conflict detection, and blob request list
- `causal_ancestor()` — transitive parent-walk via op `pai` DAG
- `conflito_id_de()` — deterministic conflict UUID from SHA-256 of sorted op IDs
- Custom serde for `Hlc` (string format) and `[u8; 32]` (hex)

**Fixture tests (`core/tests/sync_fixtures.rs`)**
- Parameterized test runner loading all 10 fixtures
- Compares `aplicadas`, `novas_ops_remotas`, `blobs_solicitados`, and `conflitos` (by op_local/op_remota/alvo, ignoring generated IDs)

## [1.26.0] — 2026-04-30

### Added — CO-74: Relationship graph — typed FK references + query DSL + wikilink promotion

**`entry_relations` table (per-universe `data.db`, migration v3)**
- Schema: `(universe_key, from_path, to_path, relation_type, created_at)` with PK + two directional indexes
- Indexed on `(universe_key, from_path, relation_type)` and `(universe_key, to_path, relation_type)` — O(log N) lookups in both directions
- Created idempotently on every DB open via `UNIVERSE_SCHEMA IF NOT EXISTS`

**Wikilink parser (`core/src/wikilink.rs`)**
- `resolve_ref_value(s)` — strips `[[target]]` or `[[target|alias]]` notation, returns bare path
- `extract_wikilinks(text)` — scans free text for all wikilink targets
- Used at entry write time to resolve typed ref field values

**Relation index (`co-web/src/relation_index.rs`)**
- `RelationIndex` with `replace_all`, `delete_for_entry`, `outbound`, `inbound`
- `extract_relations(manifest, entry_type, frontmatter)` — derives `(relation_type, to_path)` pairs from manifest-declared `ref`/`ref_list` fields only; non-ref fields with wikilinks stay as plain text
- `sync_entry_relations(conn, ...)` — called from all write paths
- `backfill_for_manifest(conn, ...)` — re-derives relations for all entries of affected types
- `backfill_relations_background(pool, slug, manifest)` — fire-and-forget thread spawned on manifest update

**Manifest-driven typing**
- On every entry create/update (via entry routes and vault routes), manifest is loaded and FK relations derived from `Ref`/`RefList` fields and stored atomically
- On entry delete, outbound relations removed
- Wikilinks in non-ref fields (plain `String`, `Enum`, etc.) never produce relation rows

**Query DSL (`co-web/src/query_dsl.rs`)**
- Syntax: `FROM <type> [WHERE <cond> [AND <cond>]*] [LIMIT <n>]`
- Operators: `=` (exact frontmatter match), `LIKE` (frontmatter LIKE), `INCLUDES` (relation join)
- `INCLUDES` compiles to a `JOIN entry_relations` with `DISTINCT` deduplication
- Field names validated as safe identifiers before interpolation into `json_extract` paths (SQL-injection proof)
- Result cap: 1 000 rows (explicit LIMIT clamped silently)
- Max 10 filter conditions per query

**API: `GET /api/v1/universes/:slug/query`**
- `?q=<dsl>` — parses DSL, compiles to SQLite, returns `{ entries, total }`
- Returns 400 on parse error with human-readable message

**Board UI — relation-aware entry detail**
- `GET /api/v1/universes/:slug/entries/*path` now returns `{ ...entry, relations: [...] }` in JSON (protobuf unchanged)
- `relations` array lists outbound FK edges: `{ universe_key, from_path, to_path, relation_type, created_at }`
- Board can render relationship-aware views without a separate API call

**Backfill on manifest update**
- When `_universe.yaml` is updated via vault PUT, `backfill_relations_background` is spawned alongside the existing index-rebuild job
- Idempotent: can be re-run safely; `replace_all` clears stale rows before inserting

## [1.25.0] — 2026-04-30

### Added — CO-77: Per-universe SQLite sharding + LiteFS read replicas

**Storage architecture split**
- Monolithic `co.db` renamed to `meta.db` at startup (atomic POSIX rename; backward-compatible)
- Each universe gets its own `data.db` at a 2-level xxHash fanout path:
  `{data_dir}/universes/{ab}/{cd}/{key}/data.db` — 256×256 = 65 536 directories, handles 10 M+ universes without `ls` degradation
- `meta.db` retains: users, universes, universe_members, api_tokens, subscriptions, telemetry, uat_mutations, quilombo_* tables
- Per-universe `data.db` holds: entries, entries_fts (WAL-mode, independent lock per universe)

**Connection pool (`co-web/src/universe_pool.rs`)**
- `UniversePool` with LRU eviction — default capacity 1 000 open connections
- Per-universe migration runs on first open (entries + entries_fts schema)
- `get_or_open(key)` returns `Arc<Mutex<Connection>>` so different universes lock independently

**Parallel write throughput**
- Writes to different universes now run concurrently — no shared SQLite write lock
- `project_universe_index` in meta.db provides O(1) routing for legacy `/projects/{key}` routes

**Startup migration (online, zero downtime)**
- On first boot after upgrade, entries in meta.db are automatically migrated to per-universe DBs
- `project_universe_index` populated from frontmatter of project entries during migration
- meta.db entries table cleared after all universes confirmed copied

**New Storage API**
- `Storage::universe_conn(key)` — get per-universe connection
- `Storage::backup_universe(key, dest)` — rusqlite Backup API, < 30s for any universe
- `Storage::universe_db_size(key)` — file-size quota check
- `Storage::search_entries_across_universes(keys, query)` — cross-universe aggregator

**LiteFS configuration (`litefs.yml`)**
- Primary in `gru`, replicas in other regions via Consul lease
- Fly.io env vars `LITEFS_DIR` and `LITEFS_URL` added to `fly.toml` and `fly.uat.toml`
- Proxy config for write-forwarding to primary

**Offline migration tool (`co-web/src/bin/split_db.rs`)**
- `split_db --data-dir /data [--dry-run]`
- Idempotent: INSERT OR IGNORE, safe to re-run after interruption
- Populates `project_universe_index` and clears meta.db entries table when complete

**Entry routes updated**
- `entry_routes.rs` and `vault_routes.rs`: all `EntryIndex` operations now use `universe_conn(slug)` instead of `meta.db` — entries are fetched from the correct per-universe DB

### Added — CO-72: Doc-generator hooks + SQLite job queue

**Doc-generator adapters (`co-web/src/doc_gen.rs`)**
- `DocAdapter` trait: `fn run(source_dir, output_type, limits) -> Result<Vec<DocEntry>>`
- Stub implementations for all v1 formats: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc
- `ResourceLimits`: wall-clock 5 min, 2 GB RAM, 1 GB output per job
- `DocFormat::from_str` via `std::str::FromStr`; `run_adapter` dispatch function

**SQLite job queue (`co-web/src/job_queue.rs`, migration v24)**
- `jobs` table: `(id, universe_key, kind, payload, status, attempts, dedupe_key, created_at, run_at, started_at, completed_at, error)`
- `enqueue_doc_gen`: idempotent submission — same `(universe, format, source_dir, adapter_version)` returns existing job ID
- FIFO claim via `UPDATE … RETURNING` with `(run_at, created_at)` ordering; no universe starvation
- Exponential backoff on failure (2^n min, capped 64 min); dead-letter after 5 attempts
- In-process worker loop (`spawn_worker`) using `tokio::time::timeout` for wall-time enforcement
- `doc_gen_error` / `doc_gen_error_at` columns on `universes` table for failure surfacing

**API endpoints**
- `POST /api/v1/universes/:slug/jobs/doc-gen` (owner-only): submit doc-gen job, returns `{ job_id }`
- `GET /api/v1/universes/:slug/jobs/doc-gen/last-error` (owner-only): last failure message + timestamp

**CO-77 compatibility fixes (co-web/src/storage.rs)**
- `seed_template_universe`: writes entries to per-universe DB (universe pool) instead of meta.db
- `reseed_template_content_pages`: same fix
- `clone_universe_internal`: reads from source per-universe DB, writes to target per-universe DB + registers in `project_universe_index`
- Migration v24 runs after v23; v25 (CO-77) follows
## [1.24.0] — 2026-04-30

### Added — CO-71: Per-universe schema validator + generic JSON entry storage

- `core/src/payload.rs` — `validate_payload()` validates frontmatter JSON against a manifest `ContentType` schema with dot-notation field-path errors; `coerce_payload()` coerces fields to typed Rust values; `TypedEntry` with `fields: BTreeMap<String, TypedValue>` (Date → `DateTime<Utc>`, Number, Boolean, StringArray, String, Null)
- `co-web/src/index_manager.rs` — `IndexManager::apply_indexes()` / `drop_stale_indexes()` / `sync_indexes()` diff and apply SQLite expression indexes (`idx_co71_<universe>_<field>`); `apply_manifest_indexes_background()` spawns a background thread so index creation never blocks HTTP writes
- `co-web/src/entry_index.rs` — `upsert()` now writes `payload` column (mirrors `frontmatter_json`); `typed_view()` converts `EntryRow` → `TypedEntry` using the manifest; expression indexes target `json_extract(payload, '$.field')`
- `co-web/src/entry_routes.rs` — POST and PUT entry handlers validate frontmatter against `_universe.yaml` manifest before write; invalid payloads return 422 with field-path error; legacy universes (no manifest) pass through unchanged
- `co-web/src/vault_routes.rs` — PUT `_universe.yaml` triggers background index sync via `apply_manifest_indexes_background`
- `co-web/src/error.rs` — `AppError::UnprocessableEntity` (HTTP 422) for manifest validation failures
- Migration v24: `entries.payload TEXT NOT NULL DEFAULT '{}'` + backfill from `frontmatter_json`; `universes.manifest_version INTEGER NOT NULL DEFAULT 0` for future migration tracking

## [1.23.0] — 2026-04-30

### Added — CO-70: Manifest format spec — `_universe.yaml` at universe root

- `core/src/manifest.rs` — typed `Manifest` struct hierarchy parsed from `_universe.yaml`
- `parse()` / `parse_str()` — validates size cap (100 KB), content-type count cap (100), field-path errors, and forward-compat warnings for unknown top-level keys
- `default_manifest(name)` — returns a board-of-tasks manifest matching pre-manifest behaviour (`task` type, `[todo, doing, done]` board columns)
- `Manifest::triggers_migration_from(stored_version)` — CO-71 hook for entry-payload migration on schema version bump
- `docs/schemas/_universe.v1.json` — JSON Schema (draft 2020-12) for `_universe.yaml` v1

## [1.22.7] — 2026-04-30

### Removed — CO-64: git-sync dead code + migration v23 drops git_* columns

- Deleted `co-web/src/git_sync.rs` (365 lines, dead since Vault API pivot)
- Removed `UniverseGitConfig` struct and git storage methods
- Removed route handlers: `update_universe_git`, `manual_sync`, `webhook_sync`
- Migration v23: `ALTER TABLE universes DROP COLUMN` for 6 git_* columns
- Added `docs/ARCHITECTURE.md` — post-GitHub data model overview
- CO-50, CO-55 marked deprecated

## [1.22.6] — 2026-04-30

### Added — CO-138: Wave 2 Playwright e2e coverage (sidebar tree, mermaid, onboarding)

Three Playwright test suites under `co-web/e2e/wave-2/` that drive Chromium against UAT (or a local server with seeded fixtures):

- `co-web/e2e/wave-2/co-98-sidebar-tree.spec.ts` — verifies the timeline trio (`tempo`, `humanity`, `universo`) appears nested under `template` in the sidebar, with chevron toggle and CSS indent.
- `co-web/e2e/wave-2/co-107-mermaid.spec.ts` — asserts the template home renders a Mermaid SVG containing the trio node labels, and that universes without Mermaid blocks do not load the Mermaid bundle.
- `co-web/e2e/wave-2/co-99-onboarding.spec.ts` — exercises the 3-step onboarding banner lifecycle: cookie set on dismiss, reload suppression, mobile viewport suppression, and no banner for logged-in users.

Additional infrastructure:
- `co-web/e2e/helpers.ts`: `loginAsAdmin` helper — UAT uses magic `uat-login`, prod/local uses `password-login` via `CO_ADMIN_EMAIL` + `CO_ADMIN_PASSWORD` env vars.
- `co-web/playwright.config.ts`: `baseURL` now reads `process.env.BASE_URL ?? "http://localhost:3000"` so `BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test` works.
- `docs/OPERATIONS.md`: Wave 2 regression gate command added to post-deploy section.

## [1.22.5] — 2026-04-30

### Fixed — CO-137: harden ALTER ADD COLUMN migrations against partial-application + diagnostic endpoint

**Root cause investigation (CO-137):** Migration v22 (`parent_key` on `universes`) was checked with `if current_version < 22` after a fresh `MAX(version)` read — mechanically correct. Code analysis suggests the most likely failure mode is a stale `schema_version=22` row recorded without the matching `ALTER TABLE` completing (volume snapshot edge case or a previous deploy that committed the version row but not the schema change). The diagnostic endpoint added in this release confirms prod schema state.

**Structural fix:** Replaced bare `ALTER TABLE … ADD COLUMN` calls in migrations v17–v22 with `ensure_column` — a `pragma_table_info`-guarded helper that is a no-op when the column already exists. This makes every column-add migration idempotent: re-running a partially-applied migration recovers cleanly instead of panicking on "duplicate column name."

Additionally, an **unconditional post-migration backfill** runs after all versioned blocks to ensure `parent_key` exists on the `universes` table regardless of what `schema_version` records, closing the exact failure mode from the 2026-04-30 prod incident.

**Changes:**
- `co-web/src/storage.rs`: `ensure_column` helper + unit tests (4 cases incl. partial-migration recovery simulation)
- Migrations v17, v18, v20, v21, v22 updated to use `ensure_column` + `INSERT OR IGNORE` for version row
- Unconditional `parent_key` backfill after all migrations
- `co-web/src/gestao_routes.rs`: `GET /api/v1/gestao/_schema_check` (GitHub admin auth) returning `universes` column list + `schema_version` rows

## [1.22.4] — 2026-04-30

### Fixed — `get_universe` resilient to partially-applied parent_key migration (prod hotfix)

**Symptom on prod (1.22.3):** `GET /api/v1/universes/template`, `/api/v1/universes/tempo`, etc. returned 404 "not found" even though the universes were seeded (filesystem dirs present, startup logs confirmed `Timeline universe '<key>' seeded`). Sibling endpoints that didn't go through `get_universe` (`/theme.css`, `/config`) continued to work.

**Root cause:** since 1.22.0, `Storage::get_universe` and `list_universes_for_user` selected `parent_key` (added by migration v22). When the column wasn't actually present on the DB at query time — for any reason (migration not yet applied, partial schema state, drift between machines) — the SELECT errored, `.ok()` swallowed the error, and the function returned `None`, indistinguishable from "universe doesn't exist". UAT was unaffected because its DB had the column; prod was not.

**Fix:** split the SELECT into two queries.
1. The stable schema (everything ≤ schema_v 17) — must succeed for any prod DB still in service.
2. A separate `SELECT parent_key FROM universes WHERE key = ?` that opportunistically fetches `parent_key`. If the column doesn't exist or the row is missing, gracefully returns `None`.

Applied to:
- `get_universe`
- `list_universes_for_user` (per-row second query — slight overhead, fine at our scale)
- `search_public_universes` (parent_key set to `None` unconditionally — search results don't surface parent_key in any UX path)

This is the right shape for any column added by a recent migration: the read-path should not assume the migration has landed, especially when the assumption is buried inside `.ok()` and silently maps to "not found."

After this fix, even if migration v22 didn't run (or ran and was rolled back), the API behaves correctly — the trio still has `parent_key="template"` on UAT (DB has the column), and the trio is reachable on prod (degrades to `parent_key=None` if the column happens to be missing).

## [1.22.3] — 2026-04-30

### Fixed — `parent_key` now exposed by `GET /api/v1/universes/:slug` (CO-98 follow-up)

Surfaced during UAT smoke verification of 1.22.2: the public universe-info endpoint returns a stripped `UniverseInfo` DTO, not the raw `Universe` struct. Adding `parent_key` to `models::Universe` (1.22.0) was therefore not enough — the field was silently dropped by the DTO before serialization.

- `co-web/src/universe_routes.rs::UniverseInfo` — adds `parent_key: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]` so top-level universes still emit no extra field.
- `get_universe_info` — passes `universe.parent_key` through to the DTO.

`GET /api/v1/universes` (the bulk list) was unaffected — that endpoint already returned raw `Universe` instances and emitted `parent_key` correctly.

After this fix, `curl /api/v1/universes/tempo | jq .parent_key` returns `"template"` as the CO-98 spec required.

## [1.22.2] — 2026-04-30

### Added — onboarding coach mark for first-time anonymous visitors (CO-99)

Three-step floating banner (bottom-right, ~320×120px) introducing first-time anonymous visitors on the template universe to the platform's narrative. **Non-blocking** — does not capture clicks behind it. Cookie-gated for one year on dismissal/completion.

Steps (PT-BR copy):
1. **Visões** — names the four views (Quadro / Tabela / Conteúdo / Linha do tempo) and points to the header tabs.
2. **Linha do tempo** — explains the log-scale and links to the multi-overlay at `/shared/timeline.html?u=tempo,universo,humanity` (opens in new tab).
3. **Crie seu universo** — points users at the new `+ Novo universo` sidebar button (CO-96 P1) once they create an account.

Show conditions (all must be true):
- `state.isTemplate === true`
- `await api.me()` returned null (anonymous)
- `co_onboarded` cookie is **not** set
- viewport width ≥ 720px (mobile UX deferred)

Dismissal sets `co_onboarded=1; Path=/; Max-Age=31536000; SameSite=Lax`. Theme-aware via CSS custom properties (`--card-bg`, `--accent`, `--border`, `--shadow-md`, `--text-muted`); inline-styled for self-containment, no new CSS file needed.

`setupOnboarding()` is invoked from `init()` on both the anonymous-template branch and the fallback-to-template branch. Internal gates re-check viewport + cookie + state defensively, so a future caller can't accidentally show the banner to the wrong audience.

Telemetry (onboarding completion rate) is deferred to the admin-dashboard ticket (CO-105) per spec — banner today is purely client-side cookie-driven.

## [1.22.1] — 2026-04-30

### Added — universe create modal Phase 1 (CO-96 P1)

The existing "Criar universo" modal (previously banner-only and always cloned `template`) now supports the full Phase 1 surface from CO-96:

- **`+ Novo universo` button in the sidebar header.** Always visible; opens the modal with a fresh empty form (visibility=private, copy-from off).
- **Description field** — optional textarea (rows=2).
- **Visibility radio group** — `Privado · Público assinável · Login obrigatório`. Default: `private` to match server semantics.
- **Copy-from existing universe** — checkbox + dropdown. Source dropdown is populated from `state.userUniverses` (plus a stable `Template (CO)` fallback) on every open.
- **Branched submit** — copy-from off → `POST /api/v1/universes` (empty); copy-from on → `POST /api/v1/universes/<source>/duplicate` (CO-95). Visibility ≠ private is applied via a follow-up `PUT /api/v1/universes/:slug` to keep the create endpoint shape unchanged.

The legacy banner CTA (`btn-criar-universo`) keeps its old behavior — the click handler now passes `{ copyFromTemplate: true }` to prefill copy-from from `template`, preserving the anonymous-visitor flow.

Out of scope for Phase 1 (per the ticket): debounced key-uniqueness check (server already rejects 409 on duplicate); rename/visibility-change context menu; soft-delete. Those land in Phase 2/3 of CO-96.

## [1.22.0] — 2026-04-30

Wave 2 of the v1-launch sprint, partial: universe hierarchy (CO-98) and home-page Mermaid (CO-107). Create modal (CO-96 P1) and onboarding banner (CO-99) are open as separate work.

### Added — hierarchical universes (CO-98)

Each universe row now carries an optional `parent_key` pointer. Top-level universes have `parent_key = NULL`; children render nested under their parent in the SPA sidebar with a 16px indent and a chevron (`▸ / ▾`).

- **Migration v22** — `ALTER TABLE universes ADD COLUMN parent_key TEXT; CREATE INDEX idx_universes_parent_key ON universes(parent_key);`. Nullable, no FK — orphan children (parent disappears) gracefully fall back to top-level rendering.
- **Models** — `Universe.parent_key: Option<String>` added; serialized in API responses (`#[serde(skip_serializing_if = "Option::is_none")]`), so universes without a parent emit no extra field.
- **Seed** — `seed_timeline_universe` now sets `parent_key = 'template'` on the trio (`tempo`, `humanity`, `universo`). An idempotent UPDATE backfills `parent_key` on existing rows from prior versions.
- **SPA** — `renderSidebar` builds a tree from the flat `state.userUniverses` list and renders top-level → children with chevron toggles. Per-parent expansion state persists in `localStorage` (`co_universe_tree_<key>`); default expanded if a child is the active universe.
- **Storage tests** — `SEED_TEMPLATE_INDEX_MD` added to the embedded-seed roundtrip suite; existing 143 tests pass unchanged.

### Added — Mermaid diagrams in universe-home (CO-107)

`renderUniverseHome` now post-processes Mermaid fenced blocks via the existing `CoMarkdown.renderMermaidBlocks` helper. Lazy-loaded; no overhead when an `index.md` has no Mermaid blocks.

Template now ships a root-level `index.md` (in addition to the `content/` legal pages) showing the **Template → Tempo / Universo / Humanidade** trio as a directed graph, with palette tokens matching `timeline.html`. Visible on `co.artelonga.com.br/co` and any future template clone.

- `co-web/seed/template/index.md` — new home-page seed (Mermaid + view explainer in PT-BR)
- `co-web/src/storage.rs::reseed_template_content_pages` — adds `("index.md", SEED_TEMPLATE_INDEX_MD)` to the always-overwrite list (idempotent re-seed on every boot)
- `co-web/static/variants/a/app.js::renderUniverseHome` — calls `renderMermaidBlocks(body)` after the markdown renders

Constraint preserved: existing universes whose `index.md` has no Mermaid block trigger no extra network requests and no JS errors (helper short-circuits when no fence is present).

## [1.21.2] — 2026-04-30

### Added — per-deploy regression smoke scripts (CO-103)

Two one-shot bash scripts that verify post-deploy invariants and exit non-zero with a diagnostic on any miss:

- `scripts/smoke-prod.sh` — targets `https://co.artelonga.com.br` (override via `BASE_URL`).
- `scripts/smoke-uat.sh` — targets `https://co-artelonga-uat.fly.dev` (override via `BASE_URL`).
- `scripts/smoke-lib.sh` — shared helpers (`check_status`, `check_json_field`, `check_count`).

10 checks in order: health, health-deep, template universe, timeline trio shape + event counts (21/26/28 pinned), themes CSS (`--accent: #6366f1`), static assets, service worker cache name, auth reachability (bogus login → 401), template entries total, favicon.

`docs/OPERATIONS.md` added with the full smoke-test runbook and deploy procedure.

### Added — `GET /api/health/deep`

New endpoint that verifies DB read+write (SAVEPOINT/ROLLBACK proves write access without modifying data) and disk accessibility. Returns `{"status":"ok","db":"ok","disk":"ok"}` on success or HTTP 503 with `"status":"degraded"` if any subsystem is unhealthy.

## [1.21.1] — 2026-04-30

### Added — multi-universe overlay + smooth event travel in the timeline

The timeline visualization at `/shared/timeline.html` is now demoable. Three improvements working together:

- **Multi-universe overlay.** `?u=tempo,humanity,universo` (comma-separated) renders events from any combination of the three timeline universes on the same canvas. Each universe gets its own color (teal / blue / warm) and its own vertical lane so events don't collide. URL syncs in real time when you toggle universes via the header chips.
- **Prev/next event with smooth travel.** Header has `‹ ›` buttons; arrow keys also work. Pressing one animates the focus to the next/previous event with a 750ms ease-in-out-cubic over interpolated pixel-space — so traveling from "Big Bang" to "Andromeda collision" pans smoothly across both linear and log regions instead of teleporting. Clicking an event on the timeline travels to it the same way. `Home` / `0` returns to 2026.
- **Cleaner empty / disabled states.** Nav buttons are disabled when no events are loaded. An on-canvas hint explains how to toggle when all universes are off.

### Added — `Linhas do tempo` featured page in the template universe

`co-web/seed/template/linhas-do-tempo.md` is a new public page that documents the timeline trio as a curated category under the template universe. Direct links to all three timelines, the combined view (`?u=tempo,humanity,universo`), and a "build your own" note showing the `type: event` + `date_year` frontmatter convention. Re-seeded on every boot.

### Fixed — admin sidebar polluted with anonymous "Meu Co" clones

A previous version of `rescue_orphan_universes` re-homed orphan anonymous-clone universes (key prefix `u-`) to the admin user, polluting their sidebar with clones from old visitors. Two changes:

- `rescue_orphan_universes` now skips keys matching `u-%` and `anon-%` — those are anonymous clones, not real personal universes.
- New `cleanup_admin_anon_clutter(admin_email)` runs on every startup after the seed admin is ensured. It deletes anonymous-clone universes still owned by the admin (legacy from the prior buggy rescue), along with their entries, members, and on-disk universe directory. Idempotent.

### Added — `public-static` visibility recognized by access control

`check_universe_access` now returns `ReadOnly` for universes with `visibility = 'public-static'`, matching the existing handling of `visibility = 'template'`. Without this, the new timeline universes were 404'ing for anonymous visitors even though `is_public = 1`.

## [1.21.0] — 2026-04-29

### Added — three timeline universes (`tempo`, `humanity`, `universo`)

The CO-92 timeline visualization now ships with three sibling universes seeded out of the box:

- **`tempo`** — meta-universe explaining the time-scale concept itself. 21 events bridging cosmic and human history (Big Bang → Now → heat death). Acts as the "what is this view" front door.
- **`humanity`** — focused on our species. 26 events from the emergence of Homo sapiens through agriculture, writing, the printing press, the Industrial Revolution, computing, the Web, and the present.
- **`universo`** — full cosmic timeline. 28 events from inflation through stelliferous era, Sun's red giant phase, last star, black-hole evaporation, and heat death.

Inspired by [scaleofuniverse.com/pt](https://scaleofuniverse.com/pt) but with **emphasis on time** rather than spatial scale. Each universe is `is_public=1`, `requires_login=0`, system-owned, modern theme, layout=`timeline`.

Architecture: each universe is a regular Co universe (just system-seeded). Events are markdown entries with `type: event` and a numeric `date_year` in frontmatter. Content is split from form — manifests live as JSON at `co-web/seed/timeline/{tempo,humanity,universo}.json` plus an `index.md` per universe; storage.rs only orchestrates seeding (`seed_timeline_universe`, `seed_all_timeline_universes`). Idempotent re-seed on every startup so JSON edits ship in the next deploy without manual data migration.

### Changed — timeline UI: cross-universe nav + scaleofuniverse link

`/shared/timeline.html` now shows a header tab bar with `Tempo` / `Universo` / `Humanidade` so demo viewers can flip between the three views in one click. Active universe is highlighted with an accent underline. The "scale ↗" link in the header credits scaleofuniverse.com as inspiration. Default `?u=` is now `tempo` (was `template`). Hint and error strings localized to PT-BR. Header title fetched from `/api/v1/universes/:slug` so it shows the friendly name ("Universo", not the slug).

## [1.20.11] — 2026-04-29

### Fixed — universe with no projects left the spinner up forever

`renderContent()` returned silently with `if (!state.currentProject) return;` — leaving the loading-spinner from `bootAppForUniverse` rendered indefinitely whenever the universe had no projects (or the projects fetch failed). With artelonga / qa-dev / etc. having content uploaded via vault but no canonical "project" entry, the SPA was stuck at "Carregando…" for any logged-in user opening those universes. Replaced the silent return with a call to a new `renderUniverseHome()` that always paints something visible, so the spinner can never persist past `render()`.

### Added — universe home / front page rendered from `index.md`

Each universe can now ship an `index.md` at its root. When the user enters the universe (and there's no project to render the kanban for), the SPA fetches that file and renders its body as a hero page: title from `universe.name`, description from `universe.description`, and the markdown body in the main area.

If `index.md` doesn't exist, a friendly empty state explains how to add one and reports how many entries the universe has, so the page is never spooky-blank. Mirrors the convention of `README.md` for git repos / `CLAUDE.md` for instruction files: a "what is this" front page that anyone landing here can read without scrolling.

### Added — boot watchdog + per-fetch timeouts in `bootAppForUniverse`

Each fetch step (`getUniverseInfo`, `getUniverseConfig`, `getUniverseProjects`, `selectProject`) is now wrapped in `withTimeout(promise, 8000)` so any individual hang resolves to `null` after 8s instead of blocking the whole boot. An outer 20-second watchdog renders a recovery card with "Recarregar / Voltar ao template / Reset cache" links if the boot doesn't complete — defensive against any future hang in code I haven't audited.

## [1.20.10] — 2026-04-29

### Fixed — service worker was caching every JS deploy into oblivion

`co-web/static/shared/sw.js` (the actual served file — `static/sw.js` was a stale duplicate that the server doesn't read) was cache-first for every static asset including `app.js` and `style.css`. Even `Cmd+Shift+R` couldn't bypass it: browsers route reload requests through the SW, and the SW was happily returning yesterday's bytes from `caches.match()` while only updating the cache for *next time*. So users complained that "modern theme doesn't stick" / "hard refresh doesn't load" — they were never actually receiving any of the 1.20.5 → 1.20.9 fixes.

Rewrote the SW with a **network-first** strategy for HTML/JS/CSS (deploys propagate immediately, fall back to cache only when offline) and cache-first only for icons/fonts/manifest. Also:

- Bumped `CACHE_NAME` to `co-v3-network-first`, so existing clients purge their stale cache when the new SW activates.
- Registration in `index.html` now listens for `updatefound`, calls `SKIP_WAITING` on the new worker, then reloads the page on `controllerchange` so users get the fresh bundle without manual intervention.
- Removed the `STATIC_ASSETS` precache list except for the manifest + favicon — precaching `app.js` was the original sin.

Existing users will see one auto-reload the next time they open the app; subsequent deploys arrive normally without that bounce.

## [1.20.9] — 2026-04-29

### Fixed — universe switch could leave the spinner up forever

If `selectProject`, `getUniverseProjects`, or any other async step inside `bootAppForUniverse` threw, the function fell through without clearing `state.switchingUniverse` or calling `hideLoading()`. The spinner stayed visible and the sidebar's universe-click handler refused further switches (it short-circuits on `state.switchingUniverse`). Wrapped the whole boot sequence in `try { ... } finally { state.switchingUniverse = false; hideLoading(); render(); }` so a failure can never wedge the UI. Each fetch step also has its own try-catch with `console.warn` so a bad universe degrades gracefully instead of cascading.

### Changed — modern palette is now the unconditional default

`loadThemeCss` previously fell back to the universe's own theme.css when `co_user_palette` wasn't set. With user feedback that modern should "stick" across every universe, the function now defaults to `modern` if no palette is stored and persists that choice. A per-load cache-buster (`?v=<unix>`) is appended so a recent change is picked up even when the browser was sitting on a stale theme.css.

### Changed — Conteúdo sections and folders default to collapsed

The Páginas section and every nested folder now start closed; the user expands what they want to look at. Saved-state in localStorage still wins, so once you open a folder it remembers next time. This makes universes with hundreds of entries (artelonga: 146, quilomboaraucaria: 70) approachable from a clean slate instead of dumping the whole tree on first render.

## [1.20.8] — 2026-04-29

### Fixed — modern theme actually loads modern colors

`loadThemeCss` was loading `template`'s `theme.css` when a user override was active. But `template` had `theme_preset='scholarly-light'` in the DB (left over from an earlier migration), so "modern override" was actually rendering scholarly browns. Two fixes:

- New endpoint `GET /api/v1/themes/:preset` returns the CSS for any built-in preset directly from the compiled-in `ThemePreset::by_name()`, independent of any universe's stored config. SPA's `loadThemeCss` now hits this endpoint when `co_user_palette` is set, so the user's choice always wins.
- Added `Storage::ensure_template_theme_preset()` and call it on every startup with `'modern'`. This brings the template universe's stored preset back in line with what the seed code intended, fixing the public landing page's appearance for unauthenticated visitors.

### Added — frontmatter preview when entry body is empty

Many universes encode their actual content as structured frontmatter rather than markdown body — e.g. artelonga's 146 entries are mostly member/community/service profiles with rich `nome` / `papel` / `bio` / `funcao` / `descricao` fields and no body. The Conteúdo view's `cardBodyHtml` now falls back to a compact key-value preview of the user-meaningful frontmatter fields when the body is empty (skipping scaffolding keys like `type`, `slug`, `created`, `tags`). Image URLs render as thumbnails; HTTP URLs as links. Up to 8 fields shown. New CSS classes: `.conteudo-fm-preview`, `.conteudo-fm-row`, `.conteudo-fm-key`, `.conteudo-fm-val`, `.conteudo-fm-img`.

## [1.20.7] — 2026-04-29

### Fixed — known personal universes now always belong to the current admin

`rescue_orphan_universes` only catches universes whose `owner_id` has no row in `users`. But after the prod data was bootstrapped, then partially wiped, then re-seeded, a more subtle state emerged: the prior admin user_id is **still in the users table** (left over), and `artelonga` / `rfq` / `qa-dev` still point at it. The current admin can't see them, but rescue skips them because the owner is technically a valid user.

Added `Storage::ensure_admin_owns_personal_universes(email, keys)` and called it on every startup with the well-known personal universe keys (`artelonga`, `rfq`, `qa-dev` — same list the bootstrap script seeds). For each of those keys, if it exists and its `owner_id != current admin user_id`, re-home it to the current admin and ensure an `owner` membership row. If it already belongs to the right user, only the membership row is reconciled (defensive). Idempotent — does nothing on a clean DB.

## [1.20.6] — 2026-04-29

### Changed — universe switching is now an atomic transition

`bootAppForUniverse` was a chain of partial state mutations interleaved with async fetches. The result was visible jank: cards from the previous universe lingered while the new one's config loaded, the settings gear flickered, and the theme swap landed at an unpredictable point in the sequence. Rewrote the flow:

1. Set `state.switchingUniverse = true` and reset all per-universe collections (`tasks`, `projects`, `currentProject`, `universeInfo`, `universeConfig`) up front, so nothing from the previous universe can leak through.
2. Show the loading spinner — it clears the content area immediately.
3. Apply the new theme/config FIRST (single hot-swap of `<link id="co-theme-css">`), so the spinner sits on the right palette.
4. Fetch projects, then drill into the first one.
5. Drop the flag and call `render()` exactly once.

The sidebar click handler now also marks the clicked item active immediately (before any fetch), and rapid double-clicks during a transition are ignored. Template banner show/hide is decided by the slug check (`isTemplate = slug === 'template'`) instead of being unconditionally hidden.

## [1.20.5] — 2026-04-29

### Fixed — orphan universes re-homed to the seeded admin

When the admin user was re-created after a data wipe (new uuid), prior universes still pointed to the old user_id and silently disappeared from the new admin's sidebar — even though `list_universes_for_user` already had the owner_id fallback. Added `Storage::rescue_orphan_universes(admin_email)` that runs on every startup right after `seed_admin_user_from_env`: any universe whose `owner_id` no longer exists in `users` (and isn't the `system` sentinel) gets re-homed to the seeded admin and an `INSERT OR IGNORE` membership row is added. Idempotent — does nothing on a healthy database.

### Fixed — modern theme override now actually applies cross-universe

Setting `co_user_palette = modern` in localStorage was supposed to make the modern look win over each universe's own `theme_preset`. The SPA was setting `data-palette="modern"` on `<html>`, but no CSS rules implement that selector — meanwhile `loadThemeCss(slug)` kept loading the universe's native theme.css (e.g. quilombo's earth tones), which overrode everything. Fixed by routing `loadThemeCss` through a preset-to-source map: when a user override is active, load the matching system universe's theme.css (`modern` → `template`) instead of the current board's. The same `<link id="co-theme-css">` element is reused, so the swap is hot.

## [1.20.4] — 2026-04-29

### Fixed — owner could be silently hidden from their own sidebar

`list_universes_for_user` only matched against `universe_members` and `subscriptions` rows. `create_universe` always inserts an owner row in `universe_members`, but if that row is ever lost (historic data, partial migration, manual cleanup), the owner stops seeing their own universe in the SPA sidebar. Added `WHERE u.owner_id = ?1 OR u.key IN (...members/subs...)` as a defensive fallback so ownership alone is enough to qualify.

### Added — stats strip in Conteúdo view

The Conteúdo view now shows a compact stats header above the sections: total entries, page count, task count, event count, distinct tag count, and last-edited relative time. Derived from the entries already loaded for the view (no extra API call). Renders unobtrusively as a single horizontal strip; collapses to two rows on mobile.

## [1.20.3] — 2026-04-29

### Fixed — `/entries` (no type filter) returned empty list

`EntryIndex::query` always added `entry_type = ?2` to the WHERE clause, even when called with an empty string. The `list_entries` route's "no type" branch passed `""`, so `GET /api/v1/universes/:slug/entries` (no `?type=`) returned 0 rows for every universe — even when filtered queries by type counted entries correctly. Visible symptom: SPA's Conteúdo view showed correct counts in the sidebar but rendered nothing in the main panel because the `allEntries` merge step (used to fold untyped markdown into the page tree) got an empty array.

Fix: `query` now omits the `entry_type` clause when the type is empty, so passing `""` truly means "any type". Filtered queries continue to work exactly as before.

### Fixed — timeline default universe was `co-dev`

`co-web/static/shared/timeline.html` defaulted `?u=` to `co-dev` (an internal-only universe), causing 404s on prod where co-dev is not seeded. Default is now `template`, which exists everywhere.

## [1.20.2] — 2026-04-29

### Changed — legal pages refresh for public test

Rewrote the four template seed pages for the initial public-test launch on `co.artelonga.com.br`:

- **Honest framing of encryption.** Previous wording implied "banco de dados criptografado em repouso" — that's roadmap (CO-86, v3.0), not current state. New text describes what's implemented today (TLS 1.3, Argon2id, access control, isolated SQLite) and explicitly calls out that bodies are plaintext at rest, with the v3.0 envelope-encryption plan stated as the path forward. For sensitive content, recommends self-hosting until v3.0.
- **Two hosting models documented.** Auto-hospedagem (MIT, you control everything, this policy doesn't apply) vs. instância gerenciada Arte Longa (`co.artelonga.com.br`, GRU region, controlador é Yuri). Each modality's responsibilities made explicit.
- **Public-test disclosure in Termos.** New §3 says "estado do produto: teste público inicial" — no formal SLA, expect breakage between versions, recommend waiting for v3.0 for production-critical use.
- **Updated `dados-rastreados.md`** with the actual telemetry event taxonomy used in the SPA (matches `static/shared/telemetry.js`), and clarifies that body content is never sent in telemetry payloads.
- **LGPD §6/§7 sharpened:** added 15-day response SLA, removed vague phrasing.

### Fixed — template content pages now refresh on every boot

`seed_template_universe()` was gated on first-boot only (`!storage.template_exists()`), which meant any update to the bundled seed pages would never reach existing deployments without a full UAT-style data reset. Extracted the four content pages into `reseed_template_content_pages()` and call it unconditionally on every server startup. Tasks and projects within the template are still seed-once (user can edit them); content pages always track the binary.

### Refactored — content separated from form

Seed content for the template universe (sobre, termos, privacidade, dados-rastreados) was previously embedded as multi-hundred-line Rust string literals inside `seed_template_universe()`. That made `storage.rs` a 3000+ line monolith mixing schema, queries, and prose.

- Moved the four pages to `co-web/seed/template/*.md` — editable as plain markdown.
- Added a tiny frontmatter parser (`split_frontmatter`, `seed_page_frontmatter`, `seed_page_body`) that turns a `.md` file with YAML frontmatter into the `(metadata_json, body_str)` pair `make_entry` expects.
- Files are embedded at compile time via `include_str!`, so no runtime filesystem dependency — single binary, single artifact.
- `created` / `modified` timestamps are stamped at seed time (so freshly seeded universes show "now"), but everything else (slug, title, order, tags) is read from the .md file's frontmatter — that's the single source of truth.
- 4 unit tests cover the parser and verify all 4 embedded files parse cleanly.
- Net: `storage.rs` shrank by ~430 lines.

## [1.20.1] — 2026-04-29

### Fixed — universe duplication now copies ALL entry types

`Storage::clone_universe` had project + task + page-specific copy paths but skipped everything else (events, clips, doc.*, untyped markdown). The first 1.20.0 duplicate of `quilomboaraucaria` produced an empty universe because all 70 source entries were `event` type from the legacy quilombo-blog migration.

- Added a final bulk `INSERT INTO entries SELECT FROM entries` step that copies all entry types not covered by the typed paths (entry_type NOT IN ('project','task','page')). Source paths/titles/frontmatter/body preserved verbatim — the duplicate is a true snapshot.
- `INSERT OR IGNORE` makes it safe to re-run if a partial copy needs completion.

## [1.20.0] — 2026-04-29

### Added — CO-95 Phase 1: owner-controlled universe duplication

- New endpoint `POST /api/v1/universes/:source/duplicate` accepts JWT or API token (via the new `auth::resolve_user_id` helper). Verifies the caller has read access to the source (owner / member / public / template), then bulk-copies entries into a new universe owned by the caller. New universe defaults to `private` visibility.
- Differs from the existing `/clone` endpoint: requires authentication, allows duplicating private universes the caller is a member of, and sets ownership to the caller (no anon-XXX fallback).
- Use case: `quilomboaraucaria` → `quilombo-blog` for parallel scalability + latency analysis without disturbing the original. Generalizes to any "materialized dev branch" workflow today; full lineage tracking + merge / promote / revert lands in CO-95 Phase 4.
- `scripts/duplicate-universe.sh <source> <target>` — keychain-token-backed helper.

### Added — `auth::resolve_user_id`

Helper for handlers outside the JWT-only `require_auth` middleware that still need to identify the caller. Tries Bearer JWT first, then falls back to API token via `Storage::get_api_token_by_value`. Used by the new duplicate endpoint; future use by CO-91 sync, CO-93 universe-type changes, etc.

### Spec

- `work/co/CO-95.md`: Universe branching — 4-phase plan (snapshot → op log → replay → merge). Phase 1 ships in this release.
- `work/co/CO-96.md`: Universe CRUD UX in the SPA — sidebar `+ New universe` button, context menu (rename / change visibility / duplicate / delete), settings tab, soft-delete + 30-day trash. 3 phases mapped to 1.20.0 / 1.21.0 / 1.22.0.

## [1.19.2] — 2026-04-29

### Fixed — telemetry beacon 415, missing favicon, missing PWA icon

Three cosmetic console errors visible after first prod login post-1.19.1:
- `POST /api/v1/telemetry/event` returned 415 because `navigator.sendBeacon` with a string body sends `Content-Type: text/plain`, which axum's `Json` extractor rejects. Patched `co-web/static/shared/telemetry.js` to use a `Blob` with `type: 'application/json'`.
- `/favicon.ico` 404'd — added `co-web/static/shared/favicon.svg` (Co wordmark) and a `<link rel="icon" type="image/svg+xml">` in `variants/a/index.html`.
- PWA manifest icon 404'd because `/shared/icon-192.png` and `/shared/icon-512.png` didn't exist. Updated `manifest.json` to reference the SVG favicon (PWA spec accepts SVG with `purpose: "any"`).

### Added — user-level Modern palette default (CO-94 follow-up)

- `applyUniverseConfig` now respects a `co_user_palette` localStorage key. On first visit, it's seeded with `'modern'` so every universe board renders with the Modern palette by default. The user can later switch via the existing palette dropdown; clearing the override returns to per-universe themes.
- This is the "session-token-like" theme preference: set once locally, applied across all boards and tables. Server-side personalization (per-user theme preference stored on the user row) is a follow-up.

## [1.19.1] — 2026-04-29

### Fixed — bulk-imported markdown now visible in the Conteúdo view (CO-94 Phase 1)

After running CO-67 prod seed (artelonga, rfq, qa-dev populated with ~146/12/93 local files), the SPA's Conteúdo tab was rendering "Nenhuma página" because it filters entries by `type=page|task|event|clip` but the bulk-imported markdown has no `type:` set in frontmatter.

- `co-web/static/variants/a/app.js::renderConteudo`: fetches all entries via `getUniverseEntries(slug)` in addition to the typed queries; folds untyped `.md` files into the page list before building the folder tree. Existing typed sections (Tasks, Events, Clips) unchanged.

### Fixed — seed script no longer uploads `.claude/` runtime state

The earlier seed run captured `.claude/worktrees/agent-XXX/...` files (co-auto runtime state) into `rfq` and `qa-dev`. The find command's exclude list missed these.

- `scripts/seed-prod-universes.sh`: added `.claude/`, `.obsidian/`, `.cache/`, `.vercel/`, `seed-co/` to the exclude paths
- Fixed `ensure_jj_repo` stderr/stdout: jj init noise was being captured into the commit_id variable, polluting the changelog snippets. Init output now goes to stderr.
- Added `scripts/cleanup-vault-noise.sh`: idempotent helper that deletes vault entries matching noise patterns. Dry-run by default; pass `--execute` to actually delete.

### Spec

- `work/co/CO-94.md`: Obsidian-like vault viewer. Phase 1 ships in this release; Phases 2-3 (dedicated Vault tab with file tree + viewer + Cmd+P search + wikilink/backlink resolution + drag-and-drop reorganization) deferred to 1.20+ and 3.x.

## [1.19.0] — 2026-04-28

### Added — CO-92: unified timeline view with linear+log scrolling

- `co-web/static/shared/timeline.html` (~470 lines): standalone HTML/SVG/JS timeline page that renders events from any universe on a horizontal time axis. No framework, no build step. Visit `/shared/timeline.html?u=<universe>`.
- **Coordinate transform**: linear within ±100 years of focus (4 px/year), logarithmic beyond (90 px/decade). One 1920px screen spans 4.6 Gya → 302,026 CE simultaneously while keeping year-scale resolution near the present.
- **Date format**: events use `type: event` + `date_year: <signed integer>` in frontmatter. Optional `date: YYYY-MM-DD` and `time: HH:MM` for modern events.
- **Interactions**: drag to pan, mouse wheel/trackpad scroll to pan, hover dots for tooltips, reset button.
- **Friendly year labels**: `4.6 Gya BP` (4.6 billion years before present), `300 kya BP` (300,000), `2026 CE`, `302026 CE`.
- 4 sample events under `work/timeline-samples/` covering Earth formation (-4.6 Gya), *Homo sapiens* emergence (-300 kya), now (2026), and +300 kya (302,026).
- `scripts/seed-timeline-events.sh`: uploads samples to a target universe via `co-token` auth.

Spec: `work/co/CO-92.md`. Phase 1 (standalone page, this release). Phases 2-4 (SPA integration, CO-73 / CO-89 wiring) deferred to follow-ups.

## [1.18.5] — 2026-04-28

### Fixed — seeded admin sees content on login (universe memberships auto-set)

After CO-85 + CO-90 (preview) shipped, a freshly-seeded prod admin (`yuri@artelonga.com.br`) logged in to an empty SPA dashboard because `list_universes_for_user` returns only owned/member/subscribed universes — and the seed didn't make the new user a member of anything.

- `Storage::ensure_admin_universe_memberships(email)`: idempotent post-seed step that adds the seeded admin as `admin` member of every existing system universe (`template`, `quilomboaraucaria`, `yggdrasil`, `dados`, `co-dev`, `co-experience`). Skips universes that don't exist yet.
- `co-web/src/server.rs::start_server`: calls `ensure_admin_universe_memberships` immediately after `seed_admin_user_from_env`, ensuring it runs on every boot (idempotent — `INSERT OR IGNORE`).
- After this deploy + a Fly machine restart, prod yuri sees system universes in their sidebar on next login.

This is still CO-90 preview territory; the full ownership transfer (yuri becomes `owner_id`, not just member) ships in CO-90 for 1.20.0.

## [1.18.4] — 2026-04-28

### Fixed — SPA login form now uses CO-85's universal `/api/v1/auth/password-login`

- `co-web/static/variants/a/app.js`: replaced the call to `/api/v1/auth/uat-login` with `/api/v1/auth/password-login`. The UAT-only endpoint returns 404 in prod by design, which is why the SPA login form failed silently in production. The new endpoint works on both UAT (with `yuri@uat.local`/`uat`) and prod (with the env-seeded admin email/password), so the same code path covers all deployments.
- Same request/response shape; no other UI changes.

### Credential reference

- **UAT** browser login at `https://co-artelonga-uat.fly.dev`: `yuri@uat.local` / `uat`
- **Prod** browser login at `https://co-artelonga.fly.dev`: `yuri@artelonga.com.br` / the password set via `CO_SEED_ADMIN_PASSWORD_HASH`

## [1.18.3] — 2026-04-27

### Fixed — CO-82: throttle mirror to stay under prod's 60 req/min cap

- First-run-on-prod mirror copied 59 of 70 quilomboaraucaria entries before tripping the per-token rate limit (HTTP 429). Adds a 1-second sleep between entry copies in `co-web/src/uat_mirror.rs`. At ~30 prod requests/min (2 GETs per entry), well below the 60/min cap with headroom for the metadata/list calls at start of each universe.
- A 200-entry universe now takes ~3.5 minutes to mirror — acceptable for an occasional UAT reset.

## [1.18.2] — 2026-04-27

### Fixed — CO-82: mirror works end-to-end (no longer needs `/api/v1/universes`)

- `co-web/src/uat_mirror.rs`: stopped calling `GET /api/v1/universes` (which requires JWT and rejected the API token). Mirror now reads a configured list of universe keys from the `UAT_MIRROR_UNIVERSES` env var (default: `artelonga,quilomboaraucaria,rfq`), fetches each via the public per-universe metadata endpoint (`GET /api/v1/universes/:slug`, no auth), and copies content via the vault routes (which already accept API tokens).
- Vault routes were already accepting API tokens via `vault_auth`; `/api/v1/universes/{slug}` for metadata is public — so the mirror's hot path now works without any auth-middleware refactor.
- Added `co-web/src/auth.rs::require_auth_with_token`: a stateful middleware that accepts JWT *or* API token. Currently unused — added as scaffolding for future routes a long-lived background worker needs to hit (CO-89 git ingestion, future external integrations). Mounting it on the existing universe protected routes requires threading state through the router builder; deferred to CO-91 or absorbed into CO-90.
- 404 on a configured universe is logged and skipped, not fatal.

### Operational

After deploy: existing `UAT_PROD_TOKEN` secret already in place from operationalize-prod.sh. The mirror will pick up the universe list from defaults; override via `flyctl secrets set UAT_MIRROR_UNIVERSES='foo,bar' -a co-artelonga-uat`.

## [1.18.1] — 2026-04-27

### Fixed — CO-90 (preview): seeded user gets `tier='user'`, not `tier='admin'`

- `Storage::seed_admin_user_from_env`: switched both insert and update branches from `tier='admin'` to `tier='user'`. The seeded account is just a regular user; privileged access to system universes (template, yggdrasil, dados, co-dev) comes from being the `owner_id` of those universes, not from a global tier value.
- This is a surgical preview of CO-90 (drop the global admin tier entirely). Full CO-90 audits and removes all remaining `tier=='admin'` bypasses in handlers (`dev_board.rs:31`, `universe_routes.rs:765`).
- Display name now defaults to the email itself (was hardcoded `'admin'`); operators can update later.
- User id prefix changed `usr_admin_` → `usr_`.
- Existing users with `tier='admin'` from a 1.18.0 deploy are NOT auto-migrated by this patch — CO-90 ships a proper migration. To force a refresh now: change the password hash secret slightly (re-run hash generator) so the drift-detection branch updates the row.

## [1.18.0] — 2026-04-27

### Added — CO-85: Password-login on prod — replace email-code friction with Argon2id auth

- `POST /api/v1/auth/password-login`: new env-agnostic endpoint; works in any deployment when the user record has a `password_hash` set. Returns the same JWT + `Set-Cookie: session=<JWT>` response shape as `uat-login`. Returns 401 for unknown email, wrong password, or missing hash (no information leak).
- `POST /api/v1/auth/uat-login`: kept as a compat alias for UAT scripts and CLAUDE.md docs; delegates to the same handler when `CO_ENV=uat`, returns 404 in production (unchanged behavior).
- `seed_admin_user_from_env()` in `Storage`: idempotent startup seed driven by `CO_SEED_ADMIN_EMAIL` + `CO_SEED_ADMIN_PASSWORD_HASH` env vars. Drift detection: if the user exists with the same hash, no-op; if the hash differs, updates hash + tier. If the user is missing, inserts with `tier=admin`. Logs once per startup: "admin user seeded: `<email>`".
- Called from `start_server` after migrations and before other seeds, any env.
- Warns at startup if `CO_SEED_ADMIN_PASSWORD_HASH` does not start with `$argon2id$` (likely misconfiguration).
- Unit tests: `password-login` success, wrong-password 401, missing-hash 401; seed drift detection (no-op, update, insert).

## [1.17.0] — 2026-04-27

### Added — CO-83: Mermaid.js diagram rendering

- `co-web/static/vendor/mermaid.min.js` (v10.9.0, 3.2 MB): vendored for offline-first rendering and tighter CSP; lazy-loaded only when a page contains a ```` ```mermaid ```` block
- `co-web/static/shared/markdown.js`: new `renderMermaidBlocks(container)` post-processor follows the existing `highlightCode` / `enableImageZoom` pattern. Idempotent (skips already-rendered blocks via `data-mermaid-rendered`), error-safe (invalid syntax → inline error box, doesn't crash the page)
- Theme bridge: reads CSS custom properties (`--bg`, `--accent`, `--text`, `--md-primary`, etc.) and maps them to Mermaid's `themeVariables`, so diagrams adapt to all 12 Co themes. Re-applied on each render so theme switches re-style new diagrams
- `securityLevel: 'strict'` and `htmlLabels: false` — no inline `<a>` href in diagrams (admits typed wikilinks later via CO-74), no embedded HTML
- Wired into the entry zoom view in `co-web/static/variants/a/app.js` next to the existing `highlightCode` call. Other variants/render paths can opt in similarly
- Seed diagram: `docs/diagrams/deployment.md` — C4 Container view of the UAT + prod deployment topology
- Supports all Mermaid v10 diagram types: flowchart, sequenceDiagram, stateDiagram-v2, classDiagram, erDiagram, gantt, C4Context/Container/Component/Deployment

## [1.16.0] — 2026-04-26

### Added — CO-82: UAT mirrors prod content on reset

- `co-web/src/uat_mirror.rs`: opt-in mirror that runs in a tokio task after a UAT reset; logs into local UAT as yuri, pulls yuri's prod universes via the Vault REST API, and replays content into UAT through the same write path
- `co-web/src/server.rs`: `uat_startup` now returns whether reset just happened; `start_server` spawns the mirror task when env vars are present
- Gated by env: `UAT_MIRROR_PROD=true`, `UAT_PROD_URL`, `UAT_PROD_TOKEN`. When unset, behavior is identical to before the patch (empty placeholders after reset)
- System universes (`template`, `yggdrasil`, `co-dev`, `co-experience`, `dados`) skipped — they have their own seed paths
- Per-universe failures logged, not fatal — prod-down or token-expired never crashes UAT
- Code only runs when `CO_ENV=uat`; on prod the mirror branch is unreachable
- Cargo.toml: `reqwest` gains `cookies` feature; new `percent-encoding` dep
- Operationalization (set Fly secrets `UAT_PROD_TOKEN` etc.) deferred — feature ships dormant

## [1.15.1] — 2026-04-26

### Fixed — CO-66: API hygiene — 500→409 on duplicate key, seed idempotency, UAT no-auto-stop

- `co-web/src/universe_routes.rs`: `POST /api/v1/universes` with an existing key now returns 409 Conflict with `{"error":"conflict"}` body instead of 500 Internal Server Error; lock is held across the existence check and insert to prevent TOCTOU
- `co-web/tests/quilombo_tests.rs`: new test `test_quilombo_seed_preserves_user_edited_description` verifies `seed_quilombo_universe` (INSERT OR IGNORE) never overwrites a user-edited description
- `fly.uat.toml`: set `auto_stop_machines = false` — UAT machine stays running through idle periods so cold-start latency does not block testing

## [1.15.0] — 2026-04-26

### Added — CO-65: visibility on `PUT /api/v1/universes/:slug`

- `co-web/src/universe_routes.rs`: extended `update_universe` handler to accept `visibility` field in addition to `name` and `description`
- Accepted values: `private`, `public-subscribable`, `requires_login`. `template` is system-only and rejected with 400
- Atomic update of legacy `is_public` and `requires_login` columns alongside `visibility`, keeping CO-49 access checks coherent
- New unit test `test_update_universe_visibility_flip` in `co-web/tests/api_tests.rs`: covers happy-path flip + invalid-value rejection

### Note

Versioned to 1.15.0 to reconcile the source `Cargo.toml` (was 1.1.0) with the
deployed binary (was reporting 1.14.0 from an image built 2026-04-07 that had
since drifted from local source). All work since CO-37 (Cargo.toml never
re-bumped after CO-37 deploy) is implicitly bundled into this release.

## [1.2.0] — 2026-04-10

### co-web

#### Added — CO-38: Yggdrasil — universe of universes: minigames hub

- **Migration v18**: `requires_login INTEGER NOT NULL DEFAULT 0` column on `universes` table — gates login-only universes from anonymous access
- **Yggdrasil universe**: seeded on first boot (`key=yggdrasil`, `requires_login=1`, `is_public=1`, `theme_preset=relic`, `layout=gaming`, `owner=system`)
- **Login gate** (`universe_routes.rs`): `GET /api/v1/universes/:slug` returns 401 for universes with `requires_login=true` when no valid JWT is present; other universes unaffected
- **`UniverseInfo`** response now includes `requires_login: bool` field
- **Global leaderboard endpoint** `GET /api/v1/games/leaderboard/global`: aggregates high scores across all games per user, returns top N sorted by total score
- **Recent activity endpoint** `GET /api/v1/games/recent`: returns recent game plays across all users sorted by `last_played_at` desc
- **Browser games** (`co-web/static/games/`): 5 pure HTML5 canvas + JS games — Tetris, Snake, Space Invaders, PointSet (memory pairs), Video Poker — each posts score to `/api/v1/games/{name}/result` on game over
- **Yggdrasil hub** (`app.js` variant a): gaming layout at `/co/yggdrasil` — player profile card (level, total score, games played), game grid (5 cards with personal best + JOGAR), global leaderboard panel, recent activity feed; detects `/co/yggdrasil/{game}` to launch individual games with per-game leaderboard
- **Login wall**: anonymous visitors to `/co/yggdrasil` see a "Login to play" CTA screen instead of the hub
- **SPA route** `/co/yggdrasil/{game}` added to the Axum router (served by the same SPA)
- **i18n strings** added for Yggdrasil UI elements (pt-BR)
- **4 new tests** in `template_tests.rs`: seed/existence, requires_login flag, 401 for anonymous, 200 for authenticated; template universe still accessible anonymously

---

## [1.1.0] — 2026-04-10

### co-web

#### Added — CO-46: Full user telemetry — privacy-respecting tracking

- **`telemetry_events` table** (migration v16): stores page views, interactions, errors, and performance events without PII — no raw IPs, no email addresses, no entry content
- **`co-web/src/telemetry.rs`**: new telemetry module with server-side middleware, storage helpers, and aggregation queries
  - `telemetry_middleware`: tracks all GET page views; filters bots; stores daily-salted IP hash, device/browser/OS from UA
  - `hash_ip_daily()`: xxhash + daily date salt — same IP gets a different hash each day, preventing cross-day re-identification
  - `cleanup_old_events()`: 90-day retention policy (removes raw rows older than 90 days)
  - `telemetry_summary()`: aggregates total events, unique visitors, top pages, error count, p95 latency, events by type and day
- **`POST /api/v1/telemetry/event`**: client-side event ingestion endpoint (returns 202 Accepted); accepts `event_name`, `event_type`, `path`, `universe_key`, `properties`, `duration_ms`, `session_id`
- **`GET /api/v1/admin/telemetry/summary`**: aggregated analytics for the last 30 days (GitHub admin auth required)
- **`GET /api/v1/admin/telemetry/export`**: last 10 000 events as CSV download (GitHub admin auth required)
- **`GET /co/co-dev/telemetria`**: admin dashboard page with cards (total visitors, unique visitors, error count, p95 latency), traffic chart, top pages, events by type, and CSV export
- **`co-web/static/shared/telemetry.js`**: client-side module
  - Respects `navigator.doNotTrack === '1'` — tracking silently disabled
  - Gated on `co_cookie_consent` in localStorage — no events sent before consent
  - Auto-tracks page views (with load time + TTI) on `DOMContentLoaded`
  - Auto-tracks JavaScript errors via `window.onerror`
  - Auto-tracks LCP and FID via `PerformanceObserver`
  - Exposes `window.coTrack(eventName, properties)` for manual interaction tracking
  - Uses `navigator.sendBeacon` for non-blocking delivery
  - Session ID: random nanoid stored in `sessionStorage` (expires on tab close)
- **Integration tests** in `co-web/tests/telemetry_tests.rs`: simulate user flow → verify events recorded, retention cleanup, HTTP endpoint status codes, admin auth guard, admin dashboard page
- **Unit tests** in `co-web/src/telemetry.rs`: UA parsing, bot detection, IP hash privacy

## [1.0.0] — 2026-04-07

### co-web

#### Added — CO-37: Design alignment — Scholarly Automaton + Relic Archive aesthetic

**Typography**
- Load Newsreader (serif) + Work Sans (sans) for Scholarly theme via Google Fonts CDN
- Load Newsreader (serif) + Manrope (sans) for Relic theme
- Load Material Symbols Outlined via Google Fonts CDN
- Font hierarchy: project name = Newsreader italic, task titles = Newsreader 600, labels = Work Sans/Manrope uppercase

**Surface & Depth (No-Line Rule)**
- Removed all `1px solid` header/sidebar borders for Scholarly and Relic palettes
- Sidebar: `surface-container-low` background via tonal shift — no right border
- Cards: asymmetric padding (16px left vs 10px right) for editorial feel
- Kanban columns: tonal background shift per palette (no column borders)
- Ghost borders via CSS custom properties at 15% opacity where accessibility requires
- Modals: ambient `box-shadow: 0 20px 50px` warm-tinted shadows
- Glassmorphism: Relic dark modal + header use `backdrop-filter: blur(20px)` with 80% opacity surface

**Color Tokens (theme_engine.rs)**
- Full Material Design 3 token set added to Scholarly (light + dark) presets: `--md-primary`, `--md-surface`, `--md-surface-container-*`, `--md-on-surface`, `--md-outline`, `--md-outline-variant`, and 30+ additional tokens
- Full MD3 token set added to Relic (dark + light) presets
- All MD3 tokens exposed as CSS custom properties `--md-*` in named palette blocks
- Scholarly dark companion: inverted surface tiers, warm brass tones preserved
- Relic light companion: warm rose-tinted light version

**Components**
- Buttons: Primary (Scholarly = brass + inner glow, Relic = blood-silk gradient), Secondary (ghost border 15% opacity, 40% on hover)
- Task cards: thin left border with priority color (critical/high/medium/low) instead of pill
- Task cards: no dividers between cards — whitespace separation
- Kanban card hover: background tonal shift to surface-container, no hard border
- View tabs: pill group style with `border-radius: 99px`, active tab gets accent bg
- Sidebar items: `translateX(4px)` on hover instead of background change
- Search input: bottom-border only (ledger style) for Scholarly palette
- Status badges: pill-shaped with `primary-container` bg for Relic

**Material Icons**
- View tabs: Material Symbols Outlined icons (view_kanban, table_rows, dashboard, auto_stories) + text
- Sidebar nav section: architecture icon
- Icon-only on mobile (label hidden below 640px)
- On desktop: icon + text

**Responsive**
- Login button, language toggle, palette switcher: always visible on all breakpoints
- Mobile ≤640px: single-column kanban, horizontal-scroll view tabs
- Tablet 641–1024px: 2-column kanban grid

**Obsidian Tasks Compatibility**
- New `co-web/src/obsidian_tasks.rs` module: bidirectional status ↔ checkbox mapping
  - `status_to_checkbox`: `todo→' '`, `in_progress→'/'`, `in_review→'~'`, `done→'x'`
  - `checkbox_to_status`: reverse mapping with uppercase-X support
  - `inject_task_checkbox`: prepends `- [c] Title` to task body on vault export
  - `apply_obsidian_tasks`: parses checkbox from body on vault import, updates frontmatter status; frontmatter is canonical (not overwritten if already set)
- `vault_routes.rs` GET: injects checkbox line into task entry bodies on export
- `vault_routes.rs` PUT: parses checkbox from incoming body, updates frontmatter status on import; strips checkbox line from stored body
- `app.js`: `taskToObsidianLine`, `parseObsidianCheckboxLine`, `extractStatusFromBody` utilities
- 14 unit tests in `obsidian_tasks.rs` covering all status/checkbox combinations and edge cases

## [0.30.0] — 2026-04-06

### co-obsidian (new module)

#### Added — CO-34: Obsidian plugin — sync CO universe ↔ vault

- `co-obsidian/` — new Obsidian community plugin (TypeScript, esbuild)
- `manifest.json`: id `co-universe-sync`, name "CO Universe Sync", minAppVersion 1.4.0
- `package.json` with esbuild build system + Jest test runner
- Plugin settings: CO instance URL, API token, universe slug, sync direction, interval, conflict markers
- Settings tab with connection test and OAuth login button
- `src/api-client.ts` — typed CO Vault API client (listFiles, getFile, putFile, deleteFile, search, getTags)
- `src/sync-engine.ts` — core sync engine:
  - `pull()`: GET `/vault/` listing → mtime-based incremental check → render + write to vault
  - `push()`: scan vault .md files → hash-based change detection → upload to CO
  - `sync()`: bidirectional — pull then push, last-write-wins; optional conflict markers
  - Sync triggers: on-save (debounced 5 s), startup, configurable interval
  - Status callbacks: idle / syncing / synced / offline / conflict / error
- `src/frontmatter.ts` — bidirectional frontmatter mapping:
  - CO → Obsidian: `labels` → `tags`, `created_at` → `created`, `updated_at` → `modified`, `parent: N` → `parent: "[[CO-N]]"`
  - Obsidian → CO: `tags` → `labels`, `created` → `created_at`, `modified` → `updated_at`, `parent: "[[CO-N]]"` → `parent: N`
  - Unknown fields preserved in both directions (round-trip safe)
  - `parseFrontmatter`, `serialiseFrontmatter`, `extractFrontmatterBlock`, `renderMarkdown`
- `src/wikilinks.ts` — wikilink generation and resolution:
  - `[[CO-21|Title]]` wikilinks in exported .md
  - `parent:: [[CO-21]]` inline Dataview field for hierarchy
  - `extractWikilinkIds`, `resolveParentRef`, `wikilinksToMdLinks`, `mdLinksToWikilinks`
- `src/status-bar.ts` — status bar: "CO: synced ✓" / "CO: syncing…" / "CO: offline" / "CO: N conflicts"
- `src/main.ts` — main plugin class:
  - Ribbon icon (click to sync)
  - 6 commands: Sync now, Pull from CO, Push to CO, Open in CO, Create task, Link to CO
  - ObsidianProtocolHandler for OAuth callback (`obsidian://co-universe-sync/oauth`)
  - Auto-sync interval with `registerInterval`
  - On-save debounced push via `vault.on("modify")`
- `.co/sync.json`: `{ lastSync, fileHashes, remoteMtimes, remoteVersion }` for incremental sync
- Authentication: API token paste (stored in data.json) + OAuth browser flow + auto token refresh
- `tests/frontmatter.test.ts` — 30 unit tests: round-trip mapping, parsing, serialisation
- `tests/wikilinks.test.ts` — 22 unit tests: generation, resolution, Dataview fields
- `tests/sync-engine.test.ts` — 11 integration tests: mock CO API, pull/push/sync verification
- `tests/__mocks__/obsidian.ts` — Obsidian API mock for Jest (no real vault needed)
- `README.md` with setup instructions, command table, frontmatter mapping table
- `LICENSE`: MIT
- All 63 tests pass

---

## [0.29.0] — 2026-04-06

### co-web

#### Added — CO-35: Vault REST API + Obsidian Clipper support

- `vault_routes.rs` — Vault REST API compatible with Obsidian Local REST API
  - `GET /api/v1/universes/{slug}/vault/` — list all files with metadata
  - `GET /api/v1/universes/{slug}/vault/{*path}` — get file content + stat
  - `PUT /api/v1/universes/{slug}/vault/{*path}` — create/replace file
  - `POST /api/v1/universes/{slug}/vault/{*path}` — append to file
  - `PATCH /api/v1/universes/{slug}/vault/{*path}` — targeted edit (frontmatter field, heading section, block ID)
  - `DELETE /api/v1/universes/{slug}/vault/{*path}` — soft delete (`.trash/`) or hard delete (`?permanent=true`)
  - `POST /api/v1/universes/{slug}/vault/search` — full-text search across vault files
  - `GET /api/v1/universes/{slug}/vault/tags` — aggregate all frontmatter tags
  - `GET /api/v1/universes/{slug}/vault/tree` — recursive directory tree (BTreeMap, sorted)
  - `POST /api/v1/universes/{slug}/vault/clip` — accept Obsidian Clipper payload, write clipped note
- `storage.rs` — migration v15: `api_tokens` table with indexes; `create_api_token`, `list_api_tokens`, `delete_api_token`, `get_api_token_by_value` methods
- Auth: Bearer JWT (same as board API) + long-lived API tokens (`co_` prefix, 90-day expiry)
- Token management: `POST /api/v1/auth/token`, `GET /api/v1/auth/tokens`, `DELETE /api/v1/auth/tokens/{id}`
- Rate limiting: 60 req/min per API token (in-memory sliding window, `LazyLock<Mutex<HashMap>>`)
- `static/clipper-template.json` — Obsidian Clipper compatible template for CO frontmatter schema
- `static/shared/clipper.js` — board UI paste handler
  - `Ctrl/Cmd+Shift+V` keyboard shortcut for "Paste as CO content"
  - Paste event listener on board area: detects Clipper-formatted markdown, shows choice dialog
  - "Paste as task" vs "Paste as content" dialog with frontmatter preview
  - `co:clipper-paste` custom event dispatched for board.js integration
  - `co:card-context-menu` listener adds "Copy as Obsidian markdown" to task card context menus
  - `COClipper` public API: `isClipperFormat`, `parseFrontmatter`, `toObsidianMarkdown`, `handleClipboardText`
- All 8 variant `index.html` files updated to include `clipper.js`

---

## [0.28.0] — 2026-04-06

### co (workspace)

#### Added — CO-28: Open source repo setup

- `README.md` — rewritten for public audience: what CO is, quick start (cargo install + Docker), self-hosting (Docker Compose + Fly.io), architecture diagram, CLI reference, contributing link
- `CONTRIBUTING.md` — development setup, TDD workflow, branch/label conventions, commit format, test rules, PR process
- `.github/ISSUE_TEMPLATE/bug_report.md` — structured bug report template
- `.github/ISSUE_TEMPLATE/feature_request.md` — feature request template with acceptance criteria
- `.gitignore` — added `*.db`, `*.redb`, `.env`, `.env.local` patterns; removed `!co-web/data/` exception that could allow committing runtime databases
- `Cargo.toml` — added `keywords` and `categories` to workspace package; updated repository URL to `artelonga/co`

---

## [0.27.0] — 2026-04-06

### co-web

#### Added — CO-33: E2E test suite — Playwright for full MVP flow

- `e2e/universe.spec.ts` — Universe creation: criar form submit → redirect to /co/:slug → editable board
- `e2e/board-drag.spec.ts` — Board drag-and-drop between kanban columns + full CRUD sequence
- `e2e/codemirror.spec.ts` — CodeMirror 6 editor: init, toolbar (Bold/Italic/Heading), live preview, save+persist
- `e2e/usage-gate.spec.ts` — Usage gate: API 402 structure, overlay DOM, "Entrar" opens login modal
- `e2e/theme.spec.ts` — Palette switcher: anonymous sees 4, switch updates CSS vars without reload
- `e2e/i18n.spec.ts` — i18n toggle pt↔en, co_lang cookie set, persists across page reload
- `e2e/auth-crdt.spec.ts` — Auth flow, sharing gate, anonymous editor has no WebSocket, CRDT two-context sync
- `e2e/responsive.spec.ts` — Board renders at mobile (375px), tablet (768px), desktop (1280px) viewports
- `.github/workflows/ci.yml` — Added `e2e` job: build co-web → install Playwright → run Chromium suite → upload HTML report

---

## [0.26.0] — 2026-04-06

### co-deploy

#### Added — CO-32: Ansible deployment — provision, deploy, backup playbooks for Fly.io + VPS

- New `co-deploy/` directory with standard Ansible structure
- `inventory/fly.yml` — Fly.io target (local connection via flyctl)
- `inventory/vps.yml` — generic VPS target (DigitalOcean, Hetzner, etc.) with env-var overrides
- `playbooks/provision.yml` — creates `co` unprivileged user, installs ca-certificates + sqlite3 + zstd + Caddy, creates `/opt/co/` + `/var/lib/co/data/`, configures UFW (allow 80/443, deny rest)
- `playbooks/deploy.yml` — cross-compiles co-web via `cross`, copies binary, writes systemd unit, runs seed SQL on first deploy, restarts service, verifies `/api/health`
- `playbooks/backup.yml` — SQLite `.backup` (online, consistent), zstd compression, 7 daily + 4 weekly rotation, optional rclone upload to S3/B2, cron at 03:00 UTC
- `playbooks/fly-deploy.yml` — wraps `flyctl deploy --remote-only` with pre-deploy snapshot and post-deploy health check
- `templates/co-web.service.j2` — systemd unit with ExecStart, WorkingDirectory, Environment, systemd hardening (NoNewPrivileges, ProtectSystem)
- `templates/caddy.conf.j2` — reverse proxy with auto-SSL, zstd+gzip compression, security headers (HSTS, X-Frame-Options, etc.), static asset caching
- `group_vars/all.yml` — shared config: co_version, co_port, co_domain, backup retention settings
- `group_vars/production.yml` — ansible-vault encrypted secrets: JWT_SECRET, RESEND_API_KEY
- `molecule/default/` — Docker-based integration test (provision + stub deploy on Debian 12, idempotency check)
- `requirements.yml` — community.general + ansible.posix collections
- `README.md` — quickstart for VPS and Fly.io

---

## [0.25.0] — 2026-04-06

### co-web

#### Added — CO-31: CRDT sync — Yjs + WebSocket, login required

- New module `co-web/src/ws.rs`: `DocRoom` struct (yrs `Doc`, broadcast tx, client count, dirty notify), `DocRoomManager = Arc<RwLock<HashMap>>`, `ws_handler`, `handle_socket`
- `GET /ws/doc/:universe_slug/:doc_id` — JWT-gated endpoint; returns 401 for anonymous requests (token via `?token=` query param or `co_auth` cookie)
- Yjs sync protocol v1 (binary lib0 encoding): MSG_SYNC (0) with SYNC_STEP1/STEP2/UPDATE; MSG_AWARENESS (1) for cursor positions
- Room lifecycle: load content from SQLite on first connect (initializes Y.Doc), broadcast updates to all connected clients, debounced persist (5s idle), cleanup on last disconnect
- Heartbeat: ping every 30s, disconnect after 60s silence; rate limit: 100 messages/sec per client (token bucket)
- `AppStateInner.doc_rooms` field added; WS route mounted at `/ws/doc/{slug}/{doc_id}`
- `Storage::get_entry_body()` and `Storage::update_entry_body()` methods for CRDT persistence
- Sharing gate in `get_universe_info`: anonymous universes return 404 for non-owners (checked via `co_universe_owner` cookie)
- Frontend: added `yjs`, `y-codemirror.next`, `lib0` to editor bundle
- `createAwareness()` shim implementing y-codemirror.next's awareness interface (no y-protocols dep)
- `CoYjsProvider` class: WebSocket provider with reconnect, sync-step-1 on open, apply sync-step-2/update, forward awareness
- `initEditor` accepts `wsUrl` and `user` params; CRDT mode for logged-in users; anonymous mode shows "Crie uma conta pra colaborar" toast
- Collab badge ("N users editing"), connection status dot (green/yellow/red), remote cursor CSS
- 7 unit tests: varuint roundtrip, varbytes roundtrip, sync frame structure, rate limiter burst/block, DocRoom init, anonymous 401, two-user sync

---

## [0.24.0] — 2026-04-06

### co-web

#### Added — CO-30: Dynamic CSS engine — token generation from universe config at runtime
- New module `co-web/src/theme_engine.rs`: `ThemePreset` struct (name, tokens HashMap, font fields) + `generate_css()` function
- Five built-in presets with all required CSS tokens: `scholarly` (warm cream/bronze), `scholarly-dark` (dark chocolate/bronze), `relic` (near-black/rose), `relic-light` (off-white/burgundy), `modern` (default indigo)
- All presets define: `--bg`, `--sidebar-bg`, `--card-bg`, `--text-primary`, `--text-secondary`, `--accent`, `--border`, `--status-*`, `--priority-*`, `--font`, `--font-mono`, `--radius-*`, `--shadow-*`
- `generate_css(preset, overrides)` merges custom token overrides on top of preset, outputs deterministic `:root { … }` block
- `GET /api/v1/universes/:slug/theme.css` — returns generated CSS, `Cache-Control: no-cache`, ETag based on config hash, supports `If-None-Match` (304)
- Dark/light companion mapping: `scholarly` ↔ `scholarly-dark`, `relic-light` ↔ `relic`
- Frontend (variant a): `loadThemeCss(slug)` hot-swaps `<link id="co-theme-css">` href — no page reload when theme changes
- Frontend: custom fonts inject `<link rel="stylesheet" href="https://fonts.googleapis.com/…">` with preconnect hints
- Settings panel (owner only): added dark/light toggle button, `modern` theme option, custom token overrides JSON textarea
- Unit tests: 13 theme engine tests + 4 HTTP endpoint integration tests (200 OK, all tokens present, CSS changes on theme change, 404 for missing universe, ETag 304)

---

## [0.23.0] — 2026-04-06

### co-web

#### Added — CO-23: Usage gate — 100 entries free, then account required
- `universes.content_count` column (migration v11): cached counter incremented/decremented on writes and deletes
- Middleware-style `check_usage_gate` helper: returns 402 Payment Required for anonymous universes at or above 100 entries
- Anonymous write access: `clone_universe` issues an anon JWT session cookie + `co_universe_owner` cookie for claiming
- `POST /api/v1/universes/:slug/claim` — authenticated user claims an anonymous universe (cookie must match)
- `GET /api/v1/universes/:slug` — public universe info: `content_count`, `is_anonymous`, `is_template`
- 402 response body: `{ "error": "usage_limit", "message": "Crie uma conta para continuar", "message_en": "...", "current": N, "limit": 100 }`
- Frontend (variant a): 402 → usage limit modal with "Criar conta" / "Entrar" buttons; content count badge in header
- After login with anonymous universe: auto-claim transfers ownership to real user
- Unit test: 99 entries OK, 100th OK, 101st blocked (402), unblocked after claim

---

## [Unreleased] — co-web E2E Testing (UX-50 Epic)

### co-web

#### Added — UX-51: Initialize Playwright project
- Playwright + @axe-core/playwright devDependencies in `co-web/package.json`
- `playwright.config.ts` — baseURL localhost:3000, 9 projects (chromium/firefox/webkit × desktop/tablet/mobile)
- Custom viewports: desktop (1280×720), tablet (768×1024), mobile (375×812)
- `e2e/global-setup.ts` — builds binary, starts co-web, polls `/api/health`
- `e2e/global-teardown.ts` — SIGTERM cleanup, skips if external server
- `.gitignore` updated for node_modules, test-results, playwright-report
- `npx playwright test --pass-with-no-tests` exits cleanly (code 0)

---

## [0.22.1] - 2026-01-04

### Fixed
- **External Folder Support** (#77)
  - Bundle language configs in binary using `include_str!()`
  - CO now works properly in any registered workspace without source files
  - `co init` simplified to just create directory (no README.md)
  - `co new` defaults to current directory instead of 'en' space
  - Namespaces are now simple directories users organize however they want

## [0.22.0] - 2026-01-04

### Added
- **System-wide Installation & Namespace Detection** (#75)
  - `.co/` directory now recognized as CO workspace root marker
  - `co repo switch <alias>` to switch active workspace context
  - Git submodule detection for nested repositories
  - `is_submodule` field in `SpaceLocation::InSpace` variant
  - `is_git_submodule()` and `is_submodule()` helper methods
  - Enhanced `co space current` with helpful guidance when not in workspace
  - `effective_space()` method combining detected and active workspaces
  - `active_repo` field in `GlobalConfig` for workspace context persistence

### Changed
- `co space current` now shows "(switched)" indicator when using active workspace
- Status command shows "(submodule)" indicator when in a git submodule
- Improved error messages with actionable suggestions (Navigate, Register, Switch)

## [0.21.2] - 2026-01-04

### Changed
- **Rename ui/ to i18n/** (#72)
  - Renamed `ui/` folder to `i18n/` for clarity
  - Updated all path references in core and CLI
  - Folder now clearly indicates internationalization purpose

## [0.21.1] - 2026-01-04

### Added
- **Explicit Forbidden Character List** (#70)
  - `FORBIDDEN_ID_CHARS` constant documenting all forbidden ID characters
  - `is_valid_id_char()` function for character validation
  - `validate_id()` function to check ID strings for invalid characters
  - User-facing error messages in `co create` showing forbidden characters
  - Comprehensive tests validating all forbidden characters are handled

### Documentation
- Added doc comments explaining forbidden character categories:
  - Filesystem-unsafe: `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`
  - Shell/special: `'`, `!`, `@`, `#`, `$`, `%`, `^`, `&`
  - Whitespace: space, tab, newline, carriage return
- Clarified allowed characters: alphanumeric, hyphen, dot, underscore

## [0.21.0] - 2026-01-04

### Added
- **Documentation System** (#42)
  - `co help` - Topic-based embedded documentation
  - `co help getting-started` - Quick start guide
  - `co help spaces` - Understanding spaces
  - `co help workflows` - Plan & Execute, Write workflows
  - `co help work-items` - User-stories, tasks, epics
  - Alias: `co h` for quick access
  - Added `clap_mangen` for future man page generation

### Changed
- Updated CLAUDE.md with work item types and git label mapping
- Clarified work item hierarchy (epic → user-story → task)
- Removed deprecated "scope" terminology from documentation

### Fixed
- Removed personal name references, using PRIVATE/PUBLIC/USER namespaces

## [0.20.0] - 2026-01-04

### Added
- **Archive & Storage** (#43)
  - `co archive <item>` - Move content to archive with deindexing
  - `co archive restore <item>` - Restore content from archive
  - `co archive list` - List all archived items
  - Directory structure mirrors original: `work/tasks/` → `work/archive/tasks/`
  - Adds `archived_at` timestamp to frontmatter
  - Adds `indexed: false` to exclude from co operations (locate, validate)
  - `--force` flag to replace existing archived items
  - Alias: `co ar` for quick access

## [0.19.0] - 2026-01-04

### Added
- **Analyze Command** (#41)
  - `co analyze <item>` - Evaluate content quality and generate suggestions
  - Checks for clear title, status field, and required sections
  - Type-aware validation: user-story (As/I Need/To), task (Given/When/Then)
  - Detects broken internal [[links]]
  - Generates actionable improvement suggestions
  - Generates interview questions for missing information
  - Colored output with ✓/⚠/✗ indicators
  - `--verbose` flag for detailed analysis

## [0.18.0] - 2026-01-04

### Added
- **Tools & Extensions** (#40)
  - `co tools run <name> [args...]` - Execute a tool with arguments
  - Tool types: `deterministic` (shell commands) and `predictive` (ML models, stub)
  - User tools in `user/tools/` take precedence over system tools
  - Tool schema extended with `tool_type` field
  - Default behavior: deterministic when `tool_type` not specified
  - Error handling: tool not found, missing command, execution failure

## [0.17.0] - 2026-01-04

### Added
- **Writer Agent System** (#39)
  - `co write <type> --agent <name>` - Generate content using writer agents
  - Agent backends: `manual` (interactive prompts), `claude` (skeleton for LLM), `ollama` (stub)
  - `--context FILE` to provide additional context from a file
  - `--in SPACE` to specify target space
  - `--name NAME` to skip name prompt
  - Agent schema extended with `backend` and `context` fields
  - New `agents/writer.md` example agent
  - Output validated against content schemas

## [0.16.0] - 2026-01-04

### Added
- **Plan & Execute Workflow** (#38)
  - `co conduct plan <objective>` - Create structured use-case proposals with acceptance criteria
  - `co conduct execute <id>` - Drive plans through git workflow states (todo → in-progress → review → done)
  - Two modes: Manual (interactive prompts) or Assisted (skeleton for LLM)
  - `--context FILE` to load context from a file
  - `--repo <alias>` for cross-repo operations
  - Auto-creates GitHub issue on plan creation
  - Branch creation on execute, PR tracking via `gh` CLI
  - Space-aware architecture with global repo registry

## [0.15.0] - 2026-01-04

### Added
- **GitHub as Source of Truth** (#36)
  - `co gh issue list` - List issues from GitHub repository
  - `co gh issue show <number>` - Show issue details
  - `co collab pull --all` - Pull all open issues to local markdown files
  - `co collab pull <number>...` - Pull specific issues
  - GitHub → CO mapping: labels to type/priority, assignees, state to status
  - New `core/src/github/` module with types, mapping, and GhCli wrapper

## [0.14.0] - 2026-01-04

### Added
- **Space Isolation & Commit Guards** (#47)
  - `SpaceLocation` detection: automatically detect if you're in a space or at repo root
  - `co status` now shows current location context (space vs repo root)
  - `co init --check` to find unprotected spaces (not gitignored)
  - Walking directory tree to find space markers (README.md with `type: space`)

### Changed
- Status command now displays location context with commit guard warnings

## [0.13.1] - 2026-01-04

### Changed
- **Terminology Refactor** (#49)
  - Standardized terminology: "Space" is the canonical term for namespace directories
  - Deprecated "scope" from system references (backwards-compatible aliases remain)
  - "Context" now exclusively refers to user-provided content/prompts
  - Renamed `core/src/scope.rs` → `core/src/space.rs`
  - Updated all CLI help text, commands, and i18n labels
  - Updated `type: context` → `type: space` in frontmatter
  - All tests and validation messages updated

## [0.13.0] - 2026-01-03

### Added
- **Collaborative Content Creation** (#48)
  - `co create` - Interactive content creation with role selection
  - User role: Structured prompts for user-stories (AS A / I NEED / SO THAT) and tasks (GIVEN / WHEN / THEN)
  - Agent role: Creates skeleton templates for Claude Code to fill in
  - `--story` flag to link tasks to parent user stories
  - `## Prompt` section for context persistence

## [0.12.2] - 2026-01-04

### Added
- CLAUDE.md development instructions (#56, #57)

### Changed
- Streamlined versioning workflow: version bump in same PR (#59)
- Added branch cleanup instructions

## [0.12.1] - 2026-01-04

### Added
- CHANGELOG.md with complete version history (#52)

### Changed
- Versioning policy: issues drive releases (#53)

## [0.12.0] - 2026-01-03

### Added
- **Spaces & Multi-Repo SSH** (#37, #45)
  - `co space list` - List all registered spaces
  - `co space current` - Show current space details
  - `co repo add --ssh-host` - Configure SSH identity per repo
  - Auto-detect current space from working directory
- **Extensible Content Types** (#35, #44)
  - Custom content types via `schema.yaml`
  - `co schema list` - List all available types (built-in + custom)
  - Validation support for custom types
- **Auto-gitignore on init**
  - `co init <name>` automatically adds space to `.gitignore`
  - Prevents accidental commits of user spaces to co home

### Fixed
- Language validation now accepts known languages (english, portuguese, etc.) without requiring directory
- Content type pluralization: `user-story` → `user-stories/` (not `user-storys/`)
- Clippy warnings resolved for CI compliance (#46)

## [0.11.0] - 2026-01-03

### Added
- **Work Item Types & Content Parsing** (#33, #34)
  - User-story sections: `## As`, `## I Need`, `## To`
  - Task sections: `## Given`, `## When`, `## Then`
  - Built-in types: `user-story`, `task`, `epic`, `release`
  - Content section validation for structured formats
  - `work/schema.yaml` for work item type definitions

## [0.10.0] - 2026-01-03

### Added
- **Feature System** (#31)
  - Automatic discovery of `agents/` and `tools/` directories
  - Schema-based content type registration via `schema.yaml`
  - Feature registry for extensibility
  - `co config show` displays discovered features

### Fixed
- Version updated to 0.10.1 with UI reorganization (#32)

## [0.9.0] - 2026-01-02

### Added
- **Interactive REPL** (#28)
  - `co lead` - Interactive exploration mode
  - Commands: `status`, `locate`, `use <scope>`, `help`, `quit`
  - Scope-aware prompts
  - Real-time content navigation

## [0.6.0] - 2026-01-02

### Added
- **Validation System** (#27)
  - `co validate <item>` - Validate specific content
  - `co validate all` - Validate entire workspace
  - Frontmatter validation (required fields, types)
  - Internal link validation (`[[references]]`)
  - Language and scope existence checks
  - Severity levels: Error, Warning

## [0.5.0] - 2026-01-02

### Added
- **Index & Performance** (#25)
  - SQLite-based content indexing
  - `co locate build` - Build/rebuild index
  - `co locate --stats` - Show index statistics
  - Incremental index updates (only modified files)
  - Full-text search via FTS5

### Fixed
- Deprecated exports removed, CI workflow fixed (#26)

## [0.4.0] - 2026-01-02

### Added
- **Query System** (#23)
  - `co locate` - Unified search command
  - Filter by type: `co locate --type task`
  - Filter by scope: `co locate --scope private`
  - Full-text search: `co locate "search term"`
  - Combined filters and search

### Changed
- Unified `find` and `search` into single `co locate` command (#24)

## [0.3.0] - 2026-01-02

### Added
- **Content Management** (#22)
  - `co new <type> <name>` - Create new content
  - `co show <item>` - Display content
  - `co update <item> --status <status>` - Update metadata
  - `co delete <item>` - Remove content
  - Frontmatter parsing with YAML support
  - Content type detection

## [0.2.0] - 2026-01-02

### Added
- **Language Foundations** (#21)
  - Multi-language support (english, portuguese, guarani-mbya)
  - Internationalization (i18n) for CLI messages
  - `co lang <code>` - Set UI language
  - `co languages` - List supported languages
  - Lexicon structure for definitions
  - Language-specific directories (`en/`, `pt/`, `gun/`)

## [0.1.0] - 2026-01-02

### Added
- Initial release
- Graph-based content management foundation
- `co init <name>` - Initialize context
- `co list` - List contexts and languages
- `co status` - Show workspace status
- Basic CLI structure with clap
- Workspace configuration (`.co/config.yaml`)

---

## Roadmap

### Upcoming (v1.0)
- [x] #36 - GitHub as Source of Truth (sync issues/PRs)
- [x] #38 - Plan & Execute Workflow
- [x] #39 - Writer Agent System
- [x] #40 - Tools & Extensions
- [x] #41 - Analyze Command
- [ ] #42 - Documentation Polish
- [x] #43 - Archive & Storage
- [x] #47 - Space Isolation & Commit Guards
- [x] #48 - Collaborative Content Creation (User + Agent)
- [x] #49 - Terminology Refactor (space/context/scope)
