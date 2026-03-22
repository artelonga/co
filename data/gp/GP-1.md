---
id: 1
title: Base App Architecture & Workspace Setup
status: todo
priority: critical
labels:
  - epic
  - architecture
  - base-app
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

AS A developer building a modular game platform,
I NEED a unified Rust workspace with a plugin-capable server and SvelteKit frontend in a single repo,
SO THAT the base app provides core functionality (auth, profiles, leaderboards, tasks, notes) and universes can be added as self-contained plugins without modifying the core.

## Scope
- Repo 1 (`game`): Rust workspace (core lib, server bin) + SvelteKit web frontend
- Repo 2 (`universes`): Plugin packages — each game (Tetris, Snake, etc.) is a template
- Plugin interface defined as a Rust trait with TOML manifest + optional WASM logic
- Semver: v0.1.0 — initial deployable base app

## Architecture
```
game/                          # Base App Repo
├── Cargo.toml                 # Workspace root
├── core/                      # Shared types, plugin trait, storage
├── server/                    # Axum HTTP server
├── web/                       # SvelteKit frontend
└── plugins/                   # Plugin loader + registry

universes/                     # Plugins Repo
├── template/                  # Starter template for new plugins
├── tetris/                    # Example plugin: Tetris
├── snake/                     # Example plugin: Snake
├── invaders/                  # Example plugin: Space Invaders
├── pointset/                  # Example plugin: PointSet
└── poker/                     # Example plugin: Poker
```

## Out of Scope
- Real-time multiplayer (WebSocket) — deferred
- WASM scripting engine — deferred
- OAuth/social login — deferred
