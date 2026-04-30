# Changelog

All notable changes to CO are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

## [1.21.1] — 2026-04-26

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

## [1.21.0] — 2026-04-26

### Added — three timeline universes (`tempo`, `humanity`, `universo`)

The CO-92 timeline visualization now ships with three sibling universes seeded out of the box:

- **`tempo`** — meta-universe explaining the time-scale concept itself. 21 events bridging cosmic and human history (Big Bang → Now → heat death). Acts as the "what is this view" front door.
- **`humanity`** — focused on our species. 26 events from the emergence of Homo sapiens through agriculture, writing, the printing press, the Industrial Revolution, computing, the Web, and the present.
- **`universo`** — full cosmic timeline. 28 events from inflation through stelliferous era, Sun's red giant phase, last star, black-hole evaporation, and heat death.

Inspired by [scaleofuniverse.com/pt](https://scaleofuniverse.com/pt) but with **emphasis on time** rather than spatial scale. Each universe is `is_public=1`, `requires_login=0`, system-owned, modern theme, layout=`timeline`.

Architecture: each universe is a regular Co universe (just system-seeded). Events are markdown entries with `type: event` and a numeric `date_year` in frontmatter. Content is split from form — manifests live as JSON at `co-web/seed/timeline/{tempo,humanity,universo}.json` plus an `index.md` per universe; storage.rs only orchestrates seeding (`seed_timeline_universe`, `seed_all_timeline_universes`). Idempotent re-seed on every startup so JSON edits ship in the next deploy without manual data migration.

### Changed — timeline UI: cross-universe nav + scaleofuniverse link

`/shared/timeline.html` now shows a header tab bar with `Tempo` / `Universo` / `Humanidade` so demo viewers can flip between the three views in one click. Active universe is highlighted with an accent underline. The "scale ↗" link in the header credits scaleofuniverse.com as inspiration. Default `?u=` is now `tempo` (was `template`). Hint and error strings localized to PT-BR. Header title fetched from `/api/v1/universes/:slug` so it shows the friendly name ("Universo", not the slug).

## [1.20.11] — 2026-04-26

### Fixed — universe with no projects left the spinner up forever

`renderContent()` returned silently with `if (!state.currentProject) return;` — leaving the loading-spinner from `bootAppForUniverse` rendered indefinitely whenever the universe had no projects (or the projects fetch failed). With artelonga / qa-dev / etc. having content uploaded via vault but no canonical "project" entry, the SPA was stuck at "Carregando…" for any logged-in user opening those universes. Replaced the silent return with a call to a new `renderUniverseHome()` that always paints something visible, so the spinner can never persist past `render()`.

### Added — universe home / front page rendered from `index.md`

Each universe can now ship an `index.md` at its root. When the user enters the universe (and there's no project to render the kanban for), the SPA fetches that file and renders its body as a hero page: title from `universe.name`, description from `universe.description`, and the markdown body in the main area.

If `index.md` doesn't exist, a friendly empty state explains how to add one and reports how many entries the universe has, so the page is never spooky-blank. Mirrors the convention of `README.md` for git repos / `CLAUDE.md` for instruction files: a "what is this" front page that anyone landing here can read without scrolling.

### Added — boot watchdog + per-fetch timeouts in `bootAppForUniverse`

Each fetch step (`getUniverseInfo`, `getUniverseConfig`, `getUniverseProjects`, `selectProject`) is now wrapped in `withTimeout(promise, 8000)` so any individual hang resolves to `null` after 8s instead of blocking the whole boot. An outer 20-second watchdog renders a recovery card with "Recarregar / Voltar ao template / Reset cache" links if the boot doesn't complete — defensive against any future hang in code I haven't audited.

## [1.20.10] — 2026-04-26

### Fixed — service worker was caching every JS deploy into oblivion

`co-web/static/shared/sw.js` (the actual served file — `static/sw.js` was a stale duplicate that the server doesn't read) was cache-first for every static asset including `app.js` and `style.css`. Even `Cmd+Shift+R` couldn't bypass it: browsers route reload requests through the SW, and the SW was happily returning yesterday's bytes from `caches.match()` while only updating the cache for *next time*. So users complained that "modern theme doesn't stick" / "hard refresh doesn't load" — they were never actually receiving any of the 1.20.5 → 1.20.9 fixes.

Rewrote the SW with a **network-first** strategy for HTML/JS/CSS (deploys propagate immediately, fall back to cache only when offline) and cache-first only for icons/fonts/manifest. Also:

- Bumped `CACHE_NAME` to `co-v3-network-first`, so existing clients purge their stale cache when the new SW activates.
- Registration in `index.html` now listens for `updatefound`, calls `SKIP_WAITING` on the new worker, then reloads the page on `controllerchange` so users get the fresh bundle without manual intervention.
- Removed the `STATIC_ASSETS` precache list except for the manifest + favicon — precaching `app.js` was the original sin.

Existing users will see one auto-reload the next time they open the app; subsequent deploys arrive normally without that bounce.

## [1.20.9] — 2026-04-26

### Fixed — universe switch could leave the spinner up forever

If `selectProject`, `getUniverseProjects`, or any other async step inside `bootAppForUniverse` threw, the function fell through without clearing `state.switchingUniverse` or calling `hideLoading()`. The spinner stayed visible and the sidebar's universe-click handler refused further switches (it short-circuits on `state.switchingUniverse`). Wrapped the whole boot sequence in `try { ... } finally { state.switchingUniverse = false; hideLoading(); render(); }` so a failure can never wedge the UI. Each fetch step also has its own try-catch with `console.warn` so a bad universe degrades gracefully instead of cascading.

### Changed — modern palette is now the unconditional default

`loadThemeCss` previously fell back to the universe's own theme.css when `co_user_palette` wasn't set. With user feedback that modern should "stick" across every universe, the function now defaults to `modern` if no palette is stored and persists that choice. A per-load cache-buster (`?v=<unix>`) is appended so a recent change is picked up even when the browser was sitting on a stale theme.css.

### Changed — Conteúdo sections and folders default to collapsed

The Páginas section and every nested folder now start closed; the user expands what they want to look at. Saved-state in localStorage still wins, so once you open a folder it remembers next time. This makes universes with hundreds of entries (artelonga: 146, quilomboaraucaria: 70) approachable from a clean slate instead of dumping the whole tree on first render.

## [1.20.8] — 2026-04-26

### Fixed — modern theme actually loads modern colors

`loadThemeCss` was loading `template`'s `theme.css` when a user override was active. But `template` had `theme_preset='scholarly-light'` in the DB (left over from an earlier migration), so "modern override" was actually rendering scholarly browns. Two fixes:

- New endpoint `GET /api/v1/themes/:preset` returns the CSS for any built-in preset directly from the compiled-in `ThemePreset::by_name()`, independent of any universe's stored config. SPA's `loadThemeCss` now hits this endpoint when `co_user_palette` is set, so the user's choice always wins.
- Added `Storage::ensure_template_theme_preset()` and call it on every startup with `'modern'`. This brings the template universe's stored preset back in line with what the seed code intended, fixing the public landing page's appearance for unauthenticated visitors.

### Added — frontmatter preview when entry body is empty

Many universes encode their actual content as structured frontmatter rather than markdown body — e.g. artelonga's 146 entries are mostly member/community/service profiles with rich `nome` / `papel` / `bio` / `funcao` / `descricao` fields and no body. The Conteúdo view's `cardBodyHtml` now falls back to a compact key-value preview of the user-meaningful frontmatter fields when the body is empty (skipping scaffolding keys like `type`, `slug`, `created`, `tags`). Image URLs render as thumbnails; HTTP URLs as links. Up to 8 fields shown. New CSS classes: `.conteudo-fm-preview`, `.conteudo-fm-row`, `.conteudo-fm-key`, `.conteudo-fm-val`, `.conteudo-fm-img`.

## [1.20.7] — 2026-04-26

### Fixed — known personal universes now always belong to the current admin

`rescue_orphan_universes` only catches universes whose `owner_id` has no row in `users`. But after the prod data was bootstrapped, then partially wiped, then re-seeded, a more subtle state emerged: the prior admin user_id is **still in the users table** (left over), and `artelonga` / `rfq` / `qa-dev` still point at it. The current admin can't see them, but rescue skips them because the owner is technically a valid user.

Added `Storage::ensure_admin_owns_personal_universes(email, keys)` and called it on every startup with the well-known personal universe keys (`artelonga`, `rfq`, `qa-dev` — same list the bootstrap script seeds). For each of those keys, if it exists and its `owner_id != current admin user_id`, re-home it to the current admin and ensure an `owner` membership row. If it already belongs to the right user, only the membership row is reconciled (defensive). Idempotent — does nothing on a clean DB.

## [1.20.6] — 2026-04-26

### Changed — universe switching is now an atomic transition

`bootAppForUniverse` was a chain of partial state mutations interleaved with async fetches. The result was visible jank: cards from the previous universe lingered while the new one's config loaded, the settings gear flickered, and the theme swap landed at an unpredictable point in the sequence. Rewrote the flow:

1. Set `state.switchingUniverse = true` and reset all per-universe collections (`tasks`, `projects`, `currentProject`, `universeInfo`, `universeConfig`) up front, so nothing from the previous universe can leak through.
2. Show the loading spinner — it clears the content area immediately.
3. Apply the new theme/config FIRST (single hot-swap of `<link id="co-theme-css">`), so the spinner sits on the right palette.
4. Fetch projects, then drill into the first one.
5. Drop the flag and call `render()` exactly once.

The sidebar click handler now also marks the clicked item active immediately (before any fetch), and rapid double-clicks during a transition are ignored. Template banner show/hide is decided by the slug check (`isTemplate = slug === 'template'`) instead of being unconditionally hidden.

## [1.20.5] — 2026-04-26

### Fixed — orphan universes re-homed to the seeded admin

When the admin user was re-created after a data wipe (new uuid), prior universes still pointed to the old user_id and silently disappeared from the new admin's sidebar — even though `list_universes_for_user` already had the owner_id fallback. Added `Storage::rescue_orphan_universes(admin_email)` that runs on every startup right after `seed_admin_user_from_env`: any universe whose `owner_id` no longer exists in `users` (and isn't the `system` sentinel) gets re-homed to the seeded admin and an `INSERT OR IGNORE` membership row is added. Idempotent — does nothing on a healthy database.

### Fixed — modern theme override now actually applies cross-universe

Setting `co_user_palette = modern` in localStorage was supposed to make the modern look win over each universe's own `theme_preset`. The SPA was setting `data-palette="modern"` on `<html>`, but no CSS rules implement that selector — meanwhile `loadThemeCss(slug)` kept loading the universe's native theme.css (e.g. quilombo's earth tones), which overrode everything. Fixed by routing `loadThemeCss` through a preset-to-source map: when a user override is active, load the matching system universe's theme.css (`modern` → `template`) instead of the current board's. The same `<link id="co-theme-css">` element is reused, so the swap is hot.

## [1.20.4] — 2026-04-26

### Fixed — owner could be silently hidden from their own sidebar

`list_universes_for_user` only matched against `universe_members` and `subscriptions` rows. `create_universe` always inserts an owner row in `universe_members`, but if that row is ever lost (historic data, partial migration, manual cleanup), the owner stops seeing their own universe in the SPA sidebar. Added `WHERE u.owner_id = ?1 OR u.key IN (...members/subs...)` as a defensive fallback so ownership alone is enough to qualify.

### Added — stats strip in Conteúdo view

The Conteúdo view now shows a compact stats header above the sections: total entries, page count, task count, event count, distinct tag count, and last-edited relative time. Derived from the entries already loaded for the view (no extra API call). Renders unobtrusively as a single horizontal strip; collapses to two rows on mobile.

## [1.20.3] — 2026-04-26

### Fixed — `/entries` (no type filter) returned empty list

`EntryIndex::query` always added `entry_type = ?2` to the WHERE clause, even when called with an empty string. The `list_entries` route's "no type" branch passed `""`, so `GET /api/v1/universes/:slug/entries` (no `?type=`) returned 0 rows for every universe — even when filtered queries by type counted entries correctly. Visible symptom: SPA's Conteúdo view showed correct counts in the sidebar but rendered nothing in the main panel because the `allEntries` merge step (used to fold untyped markdown into the page tree) got an empty array.

Fix: `query` now omits the `entry_type` clause when the type is empty, so passing `""` truly means "any type". Filtered queries continue to work exactly as before.

### Fixed — timeline default universe was `co-dev`

`co-web/static/shared/timeline.html` defaulted `?u=` to `co-dev` (an internal-only universe), causing 404s on prod where co-dev is not seeded. Default is now `template`, which exists everywhere.

## [1.20.2] — 2026-04-26

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
