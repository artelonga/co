## Current Task: CO-49 — User access model spec — deterministic API for anonymous, logged-in, and subscribed users

---
id: 49
title: "User access model spec — deterministic API for anonymous, logged-in, and subscribed users"
status: todo
priority: critical
parent: 20
labels:
  - architecture
  - spec
  - api
module: co-web
created_at: 2026-04-13T00:00:00Z
updated_at: 2026-04-13T00:00:00Z
---

GIVEN the platform has three user tiers (anonymous, logged-in, subscribed) and three universe visibility levels (public-static, public-subscribable, private), and the current behavior is ad-hoc across multiple code paths,
WHEN I formalize this as a deterministic API specification,
THEN:

## User Tiers

### 1. Anonymous (non-logged-in)
- **Sees by default:** Co template board (static, read-only, cached per browser)
- **Can do:** Browse template tasks + content, switch themes, switch language
- **Cannot do:** Create/edit tasks, create content, drag cards, access private universes
- **Any edit action → login modal**
- **Session:** No server-side state. Cookie `co_lang2` (language) + `co_named_palette` (theme) only.

### 2. Logged-in (authenticated, no subscriptions)
- **Sees by default:** Personal private community (empty on first login, project "Bem-vindo ao Co")
- **Can do:** Full CRUD on own community, create tasks/content, switch themes, sign out
- **Limits:** 1 private community, up to 10 public communities, 100 entries per anonymous pre-login interaction
- **Session:** JWT in `session` cookie (7 days), `SameSite=Lax`
- **Sidebar:** Shows owned communities + any subscribed universes

### 3. Subscribed (logged-in + subscribed to a universe)
- **Sees:** All of tier 2 + subscribed universes in sidebar
- **Subscribing:** Search for public universes → click "Subscribe" → universe appears in sidebar
- **Permissions on subscribed universe:** Read-only by default. Write access requires invite from universe owner.
- **co-auto:** Available on subscribed universes that have `co_auto: true` in config

## Universe Visibility

| Visibility | Who sees it | Who can edit | How to access |
|------------|------------|-------------|---------------|
| `template` | Everyone (anonymous + logged-in) | Nobody (static, read-only) | Default at `/` |
| `private` | Owner only | Owner only | Created on first login |
| `public-subscribable` | Anyone who subscribes | Owner + invited collaborators | Search → Subscribe |
| `requires_login` | Any logged-in user with membership | Members with write role | Membership added by owner/system |

## API Spec (deterministic)

### Universe discovery
```
GET /api/v1/universes/search?q=co-dev    → list of public-subscribable universes matching query
GET /api/v1/universes                     → list of user's owned + subscribed universes (auth required)
GET /api/v1/universes/:slug               → universe info (public: anyone, private: owner/member only)
```

### Subscription
```
POST /api/v1/universes/:slug/subscribe    → subscribe to a public universe (auth required)
DELETE /api/v1/universes/:slug/subscribe  → unsubscribe
GET /api/v1/universes/:slug/subscribers   → list subscribers (owner only)
```

### Access check (deterministic)
```
Given: user_id, universe_slug
1. Is universe public-static (template)? → READ for everyone
2. Is user the owner? → READ + WRITE
3. Is user a member with write role? → READ + WRITE
4. Is user a member with read role? → READ
5. Is user subscribed? → READ
6. Is universe public-subscribable? → metadata only (title, description, subscriber count)
7. Otherwise → 404 (don't reveal existence)
```

## Database Changes

- [ ] Add `visibility` column to universes: `template`, `private`, `public`, `public-subscribable`
- [ ] Add `subscriptions` table: `user_id, universe_key, subscribed_at`
- [ ] Add `universe_members.role` values: `owner`, `admin`, `editor`, `viewer`
- [ ] Rename `is_public` + `is_template` + `requires_login` → single `visibility` enum

## Acceptance Criteria

- [ ] API returns correct access for all 7 scenarios in the access check table
- [ ] Anonymous user sees ONLY template board
- [ ] Logged-in user with no subscriptions sees ONLY personal community
- [ ] Subscribing to `co-dev` adds it to sidebar
- [ ] Unsubscribing removes it
- [ ] Private universe returns 404 to non-owners
- [ ] `cargo test` covers all access combinations


---

## Parent Epic: CO-20 — MVP: plataforma pública multi-tenant em artelonga.com.br/co

---
id: 20
title: "MVP: plataforma pública multi-tenant em artelonga.com.br/co"
status: todo
priority: critical
labels:
  - epic
  - mvp
  - platform
created_at: 2026-04-06T00:00:00Z
updated_at: 2026-04-06T00:00:00Z
---

AS A visitor at artelonga.com.br/co,
I NEED to see a template board, optionally create my own universe without login (up to 100 entries), and log in for full features,
SO THAT CO is accessible as a free, open-source, multi-tenant platform.

## MVP Definition

### Anonymous User Flow
1. Visit artelonga.com.br/co → see template universe (read-only board)
2. Click "Criar universo" → clone template into own universe (no login)
3. Create up to 100 content entries (tasks, notes, pages) freely
4. At 100 entries → prompted to create account to continue

### Logged-in User Flow
1. Full CRUD on own universe(s)
2. Access to all 5 named palettes + 8 variants (anonymous only gets Scholarly + Relic)
3. No limit on content entries
4. Can create additional universes

### Content/Form Separation
- Content: markdown files + SQLite metadata per universe (what exists today)
- Form: theme config (.universo.yaml) + CSS tokens + layout choice
- Every universe inherits two default themes: Scholarly (light/dark) + Relic (light/dark)

### i18n
- UI strings in pt-BR (default) and en
- Toggle in header, stored in cookie
- Domain terms in Portuguese, technical terms in English

## Architecture
```
artelonga.com.br/co          → template universe (read-only)
artelonga.com.br/co/:slug    → user universe (CRUD if owner)
```

SQLite `universes` table extended with: theme_preset, content_count, is_template
Board API already scoped by universe_key — extend to support slug-based routing

## Out of Scope (post-MVP)
- ContentDB with zstd compression (optimize later)
- CRDT / real-time collaboration
- CodeMirror editor (use plain textarea for MVP)
- Electron / Capacitor
- Ansible deployment (keep Fly.io)
- FTS5 search
- Version history beyond Git


---

## Project Configuration

```yaml
name: CO Platform
key: CO
description: CO open source platform — board UI, API, CLI, desktop apps
created_at: 2026-04-01T00:00:00Z
next_id: 60

```

---

## Roadmap

# CO Platform — Execution Roadmap

## Phase 1–2: Board (done)

1–7. CO-2..CO-8: Board API + UI overhaul ✅

## Phase 3: Public MVP — artelonga.com.br/co (Epic: CO-20)

### 3a: Core architecture
8. CO-21: Universe CRUD API (slug routing, create, clone, delete) ✅
9. CO-36: **Entry abstraction** (every entity = .md file, SQLite = index)
10. CO-24: Content/form separation (universe config → presentation, entries → content)

### 3b: Platform features
11. CO-23: Usage gate (100 entries free, then account required)
12. CO-25: Theme gating (Scholarly + Relic free, full set for logged-in)
13. CO-30: Dynamic CSS engine (runtime token generation)

### 3c: Editor & collaboration
14. CO-29: CodeMirror 6 editor (open to all, no login)
15. CO-31: CRDT sync (Yjs + WebSocket, login required + sharing gate)

### 3d: Frontend & i18n
16. CO-26: Web UI i18n (pt-BR / en toggle)
17. CO-22: Template universe (seed data, read-only, "Criar universo" CTA) ✅
18. CO-27: Landing page at /co (hero, login, criar universo)

### 3e: Deploy & quality
19. CO-32: Ansible deployment (provision, deploy, backup)
20. CO-33: E2E test suite (Playwright, full MVP flow)

### 3f: Release
21. CO-28: Open source repo setup (LICENSE, README, CI, Docker)

## Phase 4: Obsidian Ecosystem (v1.1)

22. CO-35: Vault REST API + Clipper support (file CRUD, search, clipper paste)
23. CO-34: Obsidian plugin (sync universe ↔ vault, wikilinks, community submission)

## Dependencies — execution order

```
CO-21 (universe CRUD) ✅
  └── CO-36 (entry abstraction)       ← CRITICAL: new foundation
        ├── CO-24 (content/form)      ← depends on entries
        │     ├── CO-25 (theme gate)
        │     └── CO-30 (dynamic CSS)
        ├── CO-23 (usage gate)        ← counts entries, not table rows
        └── CO-22 (template) ✅
CO-29 (CodeMirror)                    ← independent
  └── CO-31 (CRDT)                    ← after CO-29 + CO-36
CO-26 (i18n)                          ← independent
CO-27 (landing page)                  ← after CO-22 + CO-26
CO-32 (Ansible)                       ← independent
CO-33 (E2E tests)                     ← after all features
CO-28 (OSS release)                   ← last MVP task
  └── CO-35 (vault API)              ← post-MVP
        └── CO-34 (Obsidian plugin)
```

### Parallel execution groups for co auto
- **Group 1:** CO-36 (entry abstraction — critical path, builds on CO-21)
- **Group 2:** CO-24, CO-23, CO-29, CO-26 (after CO-36, except CO-29/CO-26 which are independent)
- **Group 3:** CO-25, CO-30, CO-31 (depend on group 2)
- **Group 4:** CO-27, CO-32 (depend on group 3)
- **Group 5:** CO-33 (E2E, needs everything)
- **Group 6:** CO-28 (release, last MVP)
- **Group 7:** CO-35 → CO-34 (Obsidian)
- **Group 8:** CO-37 (design alignment + Obsidian Tasks compat + v1.0 release tag)
- **Group 9:** CO-38 (Yggdrasil RPG universe)

## Phase 5: Polish, Telemetry, UAT (post-v1.0)

| ID | Task | Priority | Depends on |
|----|------|----------|-----------|
| CO-39 | Markdown rendering pipeline (minor path) | high | — |
| CO-40 | UI adequation (placeholder for spec) | medium | — |
| CO-41 | Deploy quilomboaraucaria as Co universe | high | — |
| CO-42 | Content page redesign (folders, cards, viewer, dados) | critical | CO-39 |
| CO-43 | Hidden dev board (Yuri admin) | high | — |
| CO-44 | UAT environment (yuri/uat, auto-reset) | high | CO-43 |
| CO-45 | UAT → dev change promotion | high | CO-44 |
| CO-46 | User telemetry system | high | — |
| CO-47 | Privacy policy update + tracked data list | high | CO-46 |
| CO-48 | Schema documentation MVP (data only) | medium | — |

### Execution order

```
CO-39 (markdown pipeline)        ← unblocks CO-42
  └── CO-42 (content redesign)   ← critical UX work
CO-43 (dev board)                ← independent
  └── CO-44 (UAT env)            ← needs dev board
        └── CO-45 (UAT→dev sync) ← needs UAT
CO-46 (telemetry)                ← independent
  └── CO-47 (privacy update)     ← needs telemetry data list
CO-41 (quilomboaraucaria)        ← independent
CO-48 (schema docs)              ← independent, foundation work
CO-40 (UI adequation)            ← awaiting spec
```


---

## Completed Tasks (already merged — do NOT re-implement)

- CO-30 — Dynamic CSS engine — token generation from universe config at runtime (DONE, already merged into main)
- CO-2 — Subtask tree rendering with expand/collapse in all views (DONE, already merged into main)
- CO-45 — UAT → dev change promotion — state tracking + version control backend (DONE, already merged into main)
- CO-34 — Obsidian plugin — sync CO universe ↔ Obsidian vault (DONE, already merged into main)
- CO-6 — Add assignee field to task model, API, and UI (DONE, already merged into main)
- CO-41 — Deploy quilomboaraucaria as Co universe — import content + UI from quilombo-blog (DONE, already merged into main)
- CO-24 — Content/form separation — universe config drives presentation, entries drive content (DONE, already merged into main)
- CO-35 — Vault REST API + Obsidian Clipper support (DONE, already merged into main)
- CO-7 — Auth-protect board write operations (DONE, already merged into main)
- CO-25 — Theme gating — Scholarly + Relic default, full set for logged-in users (DONE, already merged into main)
- CO-31 — CRDT sync — Yjs + WebSocket, login required, 'Crie uma conta pra colaborar' (DONE, already merged into main)
- CO-3 — Fix timeline: stable header, dependency arrows, proper zoom (DONE, already merged into main)
- CO-21 — Universe CRUD API — create, list, get, delete with slug routing (DONE, already merged into main)
- CO-44 — UAT environment — yuri/uat account, auto-reset DB, CO board pre-seeded (DONE, already merged into main)
- CO-8 — Delete project API endpoint (DONE, already merged into main)
- CO-38 — Yggdrasil — universe of universes: minigames hub with profiles + rankings (login-gated) (DONE, already merged into main)
- CO-28 — Open source repo setup — LICENSE, README, contributing guide (DONE, already merged into main)
- CO-39 — Markdown rendering pipeline — unify CodeMirror, marked, CRDT, Capacitor/Electron (DONE, already merged into main)
- CO-29 — CodeMirror 6 editor — markdown editing with live preview, open to all (DONE, already merged into main)
- CO-48 — Schema documentation MVP — data only (mermaid ERD rendering deferred) (DONE, already merged into main)
- CO-36 — Entry abstraction — .md files (truth), SQLite (index), protobuf (wire) (DONE, already merged into main)
- CO-4 — Dashboard: velocity chart, completion trend, workload by assignee (DONE, already merged into main)
- CO-43 — Hidden dev board — private universe showing CO development tasks (Yuri only) (DONE, already merged into main)
- CO-26 — Web UI i18n — pt-BR / en toggle for all board strings (DONE, already merged into main)
- CO-32 — Ansible deployment — provision, deploy, backup playbooks for Fly.io + VPS (DONE, already merged into main)
- CO-22 — Template universe — seed data, read-only for visitors, 'Criar universo' CTA (DONE, already merged into main)
- CO-47 — Privacy policy update — telemetry section + comprehensive data tracked list (DONE, already merged into main)
- CO-33 — E2E test suite — Playwright for full MVP flow (DONE, already merged into main)
- CO-1 — Board UI Overhaul (DONE, already merged into main)
- CO-23 — Usage gate — 100 entries free, then account required (DONE, already merged into main)
- CO-46 — Full user telemetry — privacy-respecting tracking for debugging + improvement (DONE, already merged into main)
- CO-37 — Design alignment — Scholarly Automaton + Relic Archive aesthetic for v1.0 release (DONE, already merged into main)
- CO-5 — Integrate variant palette switcher into board UI (DONE, already merged into main)
- CO-27 — Landing page at /co — template board with hero, login, criar universo (DONE, already merged into main)

---

## Execution Instructions

**YOUR TASK IS: CO-49 — User access model spec — deterministic API for anonymous, logged-in, and subscribed users**

IMPORTANT: Only implement CO-49. Do NOT implement or modify any other task.
Dependencies listed in the roadmap (e.g., 'Depends On: GP-8') mean those tasks are ALREADY DONE and merged into main. Their code is already in the codebase. Do not look for them or re-implement them.

Follow the acceptance criteria exactly. Each `- [ ]` item is a required deliverable.
Use conventional commits: the task specifies the commit message format.
Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt` before committing.
After completing all criteria, commit with the specified message.

## Test Isolation Rules

- All tests MUST run without opening network ports. Use in-process test servers (e.g., `axum::test::TestClient`, `tower::ServiceExt`) instead of spawning HTTP listeners.
- Never bind to `0.0.0.0`. If a test requires a port, bind to `127.0.0.1` only.
- Use temp directories for test databases (e.g., `tempfile::tempdir()`) — never write to user paths.
- Tests must be fully deterministic: no sleeps, no real network calls, no system time dependencies.
- Set `JWT_SECRET=test-secret` and `RUST_LOG=off` in test harness setup.