# CO — Use Cases (Intelligence as a Service in practice)

Companion to [`docs/roadmap.md`](roadmap.md) and the IaaS thesis at [`ArteLonga/docs/intelligence-as-a-service.html`](https://artelonga.com.br/docs/intelligence-as-a-service.html).

This doc captures concrete scenarios the platform supports — what the user sees, what flows through the bus, what's bounded service vs free brain. Each use case is testable end-to-end via the staging Playwright suite (CO-374).

> **Reminder of the stance:** the bounded intelligence (schemas, API contracts, bus events, deterministic primitives) IS the service. The brain (biological, human, creative) is liberated, not commodified. **co é livre**; **ñandé**, not oré.

---

## Use case 1 — Yuri's brain across devices, no polling

**Scenario:** Yuri writes a note on the laptop. Mobile phone, second laptop, and the live deployment all see the edit within a second.

**Flow (no polling anywhere):**
1. Yuri edits a note in CO on the laptop
2. Local CO publishes `entry.updated` to its local event bus (CO-380)
3. CO-384 bridge propagates the event over WebSocket to the cloud bus
4. Cloud bus fans out to subscribers — including Yuri's phone (subscribed to user's event scope), and any other device subscribed
5. CO-381 live timeline at `/agora` shows "📝 entry updated: …" in real time

**What's bounded service:** the `Event` schema, the WebSocket protocol, the federated bridge auth (JWKS-signed), the privacy filter (private events never federate).

**What's free brain:** the note content itself, the creative work Yuri did to write it.

**Verification:** open `/agora` in two browsers; edit on one; observe the event on the other within 300ms.

---

## Use case 2 — Yggdrasil notes flow into CO, read-only

**Scenario:** Yuri writes universe notes inside Yggdrasil's instance editor. The notes appear in CO's `yggdrasil/notes/<slug>.md` view, with full markdown render, backlinks, and a "📥 Read-only — published from Yggdrasil" banner.

**Flow:**
1. Yuri saves a note in Yggdrasil (Phase 0 NoteStore, shipped 2026-06-06)
2. Yggdrasil publishes `entry.updated{universe_key: yggdrasil, path: notes/<slug>.md}` to its bus (Yggdrasil-side Phase 2)
3. CO-384 bridge delivers the event to CO's bus
4. CO-383 subscriber upserts the note into CO's `entries` table
5. Cross-universe wikilinks `[[mbya::terms/ogunte]]` resolve via CO-363
6. Banner appears: "Edit at source →" deep-links back to Yggdrasil

**What CO never does:** poll, fetch, write-back. The flow is purely subscription-based.

**Verification:** create a note in Yggdrasil; observe it appear in CO's vault within 1s.

---

## Use case 3 — Live observability of the platform at /agora

**Scenario:** Operator opens `/agora` (pt-BR) or `/live` (en) and sees what's happening across the platform in real time.

**Events visible:**
- 📝 Entry CRUD across owned/member universes
- 🔓 Login / logout (own only)
- 👁️ Aggregated visit counts (anon-friendly; CO-378 redacts private paths)
- 💰 Billing events (own only)
- 🔄 Sync events (CO ↔ Yggdrasil bridge)
- 🚀 Deploy / migration events (admin scope)
- 🚨 Abuse detected (admin scope)

**Filter chips** narrow the view; **scope selector** switches between `mine`, `universe:<key>`, `public`, `admin`.

**What's bounded service:** the event types, the filter grammar, the WebSocket protocol, the federation visibility matrix.

**What's free brain:** the meaning the operator extracts from watching the live stream.

**Verification:** at `/agora`, observe events from at least 3 distinct event types in a 60-second window.

---

## Use case 4 — 4-day-week organizational review (CO-387)

**Scenario:** Yuri wants to test "what would running ArteLonga look like with 4-day weeks for a quarter?" — without changing how entries are stored or schemas are designed.

**Flow:**
1. Open `/u/artelonga` → click 🗓 Gregorian dropdown → pick "4-day week experiment"
2. Same entries, **different rendering**
3. Q1 2026 renders as 91 weeks × 4 days = 364 day-cells
4. Pattern visible: which "Day Quatro" is heavy with reviews, which "Day Um" is empty
5. Switch back to Gregorian; same data, default view

**The principle:** the timestamps stored in `event_at_ms` are canonical Unix ms. The "lens" is just a rendering function over them. Zero data change to experiment with calendar models.

**Configuration:** `_calendar.yaml` in the universe root defines available lenses; user picks at view time.

**Verification:** with 90 entries across Q1 2026, switching from Gregorian to 4-day-week re-arranges the grid without an API call (pure client-side re-render).

---

## Use case 5 — Cosmic + human + fictional timelines side-by-side

**Scenario:** at `/timeline` (existing route per CO-280), Yuri wants to see Big Bang → CE → mankind → fictional Shandara epoch, all rendered with the same component, scaled appropriately.

**Flow:**
1. `/timeline?lens=cosmic,human,fictional` → 3 stacked rows
2. Cosmic row: log scale, Big Bang at left edge, Common Era near right
3. Human row: linear, CE 0 → 2026, with cultural overlay events
4. Fictional row: arbitrary epoch, drives off entry's `mythos_year` frontmatter field
5. Same `<co-time-grid>` component; different lens config from `_calendar.yaml`

**What's bounded service:** the lens YAML schema, the conversion math (linear/log/custom), the render component. Three lens specs, one component.

**What's free brain:** the choice of epoch, the fictional universe's invented time, the cultural overlays Yuri layers on the human row.

**Verification:** `?lens=cosmic,human,fictional` renders 3 rows; lens dropdown can hide/show each independently.

---

## Use case 6 — Scrum sprint review with DoD verification (CO-382)

**Scenario:** It's Thursday 14:30 BRT — 30 minutes before the bi-weekly release. The sprint review automatically generates from CI events.

**Flow:**
1. Each PR in the wave published `ci.dod.verified` events as acceptance criteria checked off
2. Thursday 14:30 BRT: `sprint-review.ts` runs; pulls DoD percentages from event_log
3. Commits `docs/scrum/sprints/sprint-<N>.md` with per-PBI green/red checklist + velocity + carried-over count
4. Posts to atividades feed (visible in /agora)
5. Thursday 15:00 BRT: release-gate.yml checks all PRs have DoD = 100%; if yes, prod deploys
6. If any PR's DoD < 100%, release blocked; PRs roll to next sprint

**What's bounded service:** the DoD criteria (each `- [ ]` in spec's `## Acceptance`), the event types, the release gate.

**What's free brain:** the work that satisfies the DoD — the creative judgment of HOW to satisfy each acceptance item.

**Verification:** PR with unchecked acceptance item triggers red DoD; merge blocked.

---

## Use case 7 — Private universe with privacy redaction (CO-378)

**Scenario:** Yuri runs a private universe for `/2026-05-29/` — a `noindex,nofollow` event slide-deck. Analytics shouldn't leak its existence to anyone but Yuri.

**Flow:**
1. Visitor lands on `/2026-05-29/`; analytics fires page_view event
2. CO-378 detection rules mark the path as private (matches noindex meta OR private frontmatter)
3. Event published with `Visibility::UniverseOwner` — only Yuri's subscribers receive
4. Aggregated total visits ARE published as `Visibility::Public` (count only, no path)
5. `/gestao/resumo` top-pages table shows `🔒 (private — N entries)` cluster for non-Yuri admins
6. Yuri sees full detail; everyone else sees the aggregated count

**What's bounded service:** the visibility enum, the per-event federation rules, the path-redaction detector.

**What's free brain:** Yuri's choice to keep the page private; the creative content of the slide deck.

**Verification:** anonymous viewer of `/gestao/resumo` sees zero entries with `/2026-05-29/` in the URL.

---

## Use case 8 — Multi-device local-first editing (Wave 5+)

**Scenario (v3.1+):** Yuri starts a note on the laptop (cabin, no wifi). Two hours later, opens the phone (which had been idle); types a related note. Hours later, both reconnect. Sync resolves the divergence via Mac-style UPSERT options (CO-385).

**Flow:**
1. Laptop edit → local bus publishes; bridge unable to deliver (no wifi)
2. Event queued locally in `event_log`
3. Phone edit (independently) → local bus → queued
4. Laptop reconnects → bridge drains queue → CO Fly bus receives
5. Phone reconnects → same drain → CO Fly bus receives
6. CO-385 CRUD action tree detects "same slug, divergent body" → presents Mac-style options:
   - **Keep both** → newest renamed `<slug>_1.md`
   - **Ignore** (if hashes match) → no-op
   - **Replace** → overwrite local with cloud
   - **Update** → 3-way merge (CO-162 primitive)
   - **Upsert** → merge + insert new items

**What's bounded service:** the event durability (event_log), the conflict detection (hash compare), the action tree (5 options).

**What's free brain:** Yuri's choice of which option to apply per conflict; the creative reconciliation of the two versions.

**Verification:** simulate offline edits on both devices; reconnect; assert all 5 action tree options work.

---

## Use case 9 — Add anything → delivered downstream (CO-367)

**Scenario:** Yuri adds a poem, an audio clip, an article reference, a screenshot, a Python notebook — to any universe. Within seconds, it's:
- Cached locally for instant render
- Indexed in the KB
- Searchable via `/api/v1/kb/search`
- Visible in `/agora` as `kb.ingested` event

**Flow:**
1. New entry written to CO's vault
2. Vault write publishes `entry.created` to local bus
3. KbIndexer subscriber (CO-367) picks it up; idempotent upsert into `entry_kb_index`
4. Asset refs (CO-146 CAS) recorded
5. CO-380 publishes `kb.ingested` event
6. /agora live timeline shows it within 300ms

**What's bounded service:** the KB index schema, the search query DSL, the ingest pipeline.

**What's free brain:** the content itself (the poem's words, the article's insight, the screenshot's image).

**Resilience:** if the KB indexer is down, the entry is STILL rendered (cache-first). KB lags behind; never blocks.

**Verification:** add an entry; observe it in `/api/v1/kb/search` within 30s.

---

## Use case 10 — Public launch readiness check

**Scenario:** v3.0 Thursday 15:00 BRT — is everything ready?

**Pre-flight (automated):**
- ✅ Staging Playwright suite green (CO-374) — universe recursion, promotion, funnel, user routes all passing
- ✅ OpenAPI contract probe green (CO-375) — code matches catalog matches openapi.yaml
- ✅ Migration validation green for all PRs in wave (CO-376)
- ✅ DoD verification 100% across all wave PRs (CO-382)
- ✅ Backup snapshot exists < 24h old (CO-365)
- ✅ Rate limits active (CO-278-B) — 429 returns with Retry-After

**Pre-flight (operator, in 30-min review window 12:00-14:30 BRT):**
- 👁️ Visit prod incognito on iPhone 14 → board loads in pt-BR, install-as-app works
- 👁️ Login with yuri creds → atividades feed shows recent CI events
- 👁️ Open `/agora` → events flowing
- 👁️ Test conversion: fake email signup → magic code → verify lead+user link

**15:00 BRT:** `release-gate.yml` checks all gates green → `release-commit.sh` tags v3.0.0 → public.

**What's bounded service:** the entire 10-step CI route (CO-382), every event in the pre-flight matrix.

**What's free brain:** the operator's final judgment in the 30-min review window; the marketing copy in the launch blog post.

**Verification:** dry-run the entire flow against staging the Thursday BEFORE launch.

---

## Where every architectural piece shows up

| Use case | Specs involved |
|---|---|
| 1. Across devices | CO-380, CO-381, CO-384 |
| 2. Yggdrasil ingest | CO-380, CO-381, CO-383, CO-384 |
| 3. /agora live observability | CO-380, CO-381, CO-378 |
| 4. 4-day-week review | CO-387, CO-355 (lens pattern from workspace registry) |
| 5. Cosmic + fictional timelines | CO-387, CO-280, CO-345 |
| 6. Scrum DoD CI | CO-368, CO-369, CO-372, CO-382, CO-380 |
| 7. Private universe redaction | CO-378, CO-360, CO-340 |
| 8. Multi-device local-first | CO-385 (v3.1), CO-386 (v3.1+), CO-384 |
| 9. Add anything → KB | CO-367, CO-340, CO-146, CO-380 |
| 10. Launch readiness | All of Wave 4 + DoD verification (CO-382) |
| 11. A draft never leaks | CO-439 (allowlist-on-serve), CO-324, CO-161 |

## Use case 11 — A draft never leaks (serve only the published)

**Scenario:** Yuri keeps a private draft, `thrive market.md`, while working. It
must be impossible for that draft to appear on the public site — even by
accident, even if it lands in a served directory at deploy time.

**The flow rule:** a draft is **born in the vault** (`~/projects/yuri`), never
inside a served directory (e.g. `ArteLonga/yuri/`). It only crosses into a
served universe through the drafts→published flow, which creates an **index
entry**.

**What's bounded service:** the surfaces server serves **only** content present
in the published index — it serves the index, not the disk (CO-439). An
unindexed file in a served directory resolves to **404**, never 200. The
`.dockerignore` is a cheap second layer (defense in depth), but it is a denylist
that fails open and is **not** the boundary — the allowlist is. The `audit_serve`
tool reports any "servable but not published" file still on disk.

**What's free brain:** the decision of *what* to publish and *when* — privacy is
the user's choice; the service just makes "unpublished ⇒ unserved" structural.
See [`serve-allowlist.md`](serve-allowlist.md).

## What stays out of the bounded service

- The user's creative work (notes, poems, fictional epochs, choice of calendars to experiment with)
- The cultural meaning of an epoch (what "Day Quatro" means to ArteLonga)
- The judgment in conflict resolution (which option to pick in the action tree)
- The decision to make a universe private (privacy is the user's choice; the service just enforces it)

**The service is bounded. The brain is free.**
