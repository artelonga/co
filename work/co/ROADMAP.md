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
