# Module Organisation Pattern

This document describes the module layout convention used in `co-web/src/`.

## Flat modules vs. folder modules

**Flat module** (`foo.rs`) — used when a feature has a single responsibility and
fits comfortably in one file (< ~400 LoC).

**Folder module** (`foo/mod.rs` + sub-files) — used when a feature has multiple
distinct concerns or when a single file would exceed ~400 LoC.

## Folder module layout

```
foo/
├── mod.rs          # module declarations, re-exports of the public surface
├── <concern_a>.rs  # one file per concern
├── <concern_b>.rs
└── tests/
    ├── mod.rs      # declares test sub-modules
    ├── support.rs  # shared test helpers (pub fns, no #[test])
    └── <group>.rs  # one file per test group (< 500 LoC each)
```

### Rules

1. **No file exceeds 500 LoC** (enforced by CI via `wc -l`).
2. `mod.rs` only declares sub-modules and re-exports the module's public API.
   No business logic lives in `mod.rs`.
3. `tests/mod.rs` declares all test sub-modules. Test files use
   `use super::support::*` to import shared helpers.
4. Helper functions in `tests/support.rs` and `tests/ws_support.rs` must be
   `pub` so test sub-modules can import them.
5. The module's public surface is re-exported from `mod.rs`, so callers outside
   the module use `crate::foo::bar` rather than `crate::foo::internal::bar`.

## Example: `chat/`

The chat module was extracted from `chat_routes.rs` (2067 LoC) and
`chat_ws.rs` (1215 LoC) in CO-219.

```
chat/
├── mod.rs          # re-exports chat_router, chat_ws_handler, ChatEvent
├── permissions.rs  # resolve_role, can_read, can_post, can_manage_rooms, lock_storage
├── routes.rs       # chat_router() only — wires HTTP routes to handlers
├── rooms.rs        # room CRUD handlers + types
├── members.rs      # list_room_members_handler
├── messages.rs     # message handlers + types
├── ws.rs           # WebSocket upgrade handler + ChatEvent enum
└── tests/
    ├── mod.rs
    ├── support.rs      # REST test helpers (build_test_router, insert_user, …)
    ├── ws_support.rs   # WS test helpers (make_state, spawn_server, ws_connect, …)
    ├── rooms.rs        # room tests (1-6, 14-16)
    ├── messages.rs     # list/post message tests (7-13)
    ├── edits.rs        # rate-limit + edit tests (17-22)
    ├── delete.rs       # delete + broadcast tests (23-31)
    ├── ws_basic.rs     # WS auth gate + event tests (1-7)
    └── ws_presence.rs  # WS presence/typing tests (8-11)
```

### Key design decisions

- `permissions.rs` is the single source of truth for role helpers; `ws.rs`
  imports from it instead of duplicating `resolve_role`/`can_read`.
- `ChatEvent` lives in `ws.rs` and is re-exported from `mod.rs` so
  `server/state.rs` can reference `crate::chat::ChatEvent`.
- Tests are a subtree of the chat module (`#[cfg(test)] mod tests;` in
  `mod.rs`), not a separate integration test crate, so they have access to
  private items.
