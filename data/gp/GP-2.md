---
id: 2
title: Initialize Rust workspace structure
status: in_progress
priority: critical
parent: 1
labels:
  - base-app
  - rust
  - setup
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T18:37:53.261573+00:00
---

GIVEN the game project currently has a flat Cargo.toml with lib + bin targets,
WHEN I restructure into a Cargo workspace with core, server, and plugin crates,
THEN:
- [ ] `Cargo.toml` is a workspace root with members: `core`, `server`
- [ ] `core/` crate contains shared types (User, Universe, GameStats, Plugin trait)
- [ ] `server/` crate is the Axum HTTP binary importing `core`
- [ ] Existing storage layer (redb + XChaCha20 + Argon2id) moves to `core/`
- [ ] Existing protobuf schema compiles in `core/` build.rs
- [ ] `cargo build` succeeds with no errors
- [ ] `cargo test` passes (existing tests still work)
- [ ] Version: core v0.1.0, server v0.1.0
