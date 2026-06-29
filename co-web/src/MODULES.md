# co-web Module Patterns

This document codifies the five architectural patterns used in `co-web/src/`.
New code must follow these patterns; PR reviewers point here as the source of truth.

## 1. Directory Pattern (CO-215, CO-219)

**Flat module** (`foo.rs`) — used when a feature has a single responsibility and
fits comfortably in one file (< ~400 LoC).

**Folder module** (`foo/mod.rs` + sub-files) — used when a feature has multiple
distinct concerns or when a single file would exceed ~400 LoC.

```
foo/
├── mod.rs          # module declarations + public re-exports only
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
4. Helper functions in `tests/support.rs` must be `pub` so test sub-modules can
   import them.
5. The module's public surface is re-exported from `mod.rs`, so callers use
   `crate::foo::Bar` rather than `crate::foo::internal::Bar`.

### Example: `chat/` (CO-219)

`chat_routes.rs` (2063 LoC) + `chat_ws.rs` (1215 LoC) were promoted to:

```
chat/
├── mod.rs          # re-exports chat_router, chat_ws_handler, ChatEvent
├── permissions.rs  # resolve_role, can_read, can_post, can_manage_rooms
├── routes.rs       # chat_router() — wires HTTP routes to handlers
├── rooms.rs        # room CRUD handlers + types
├── members.rs      # list_room_members_handler
├── messages.rs     # message handlers + types
├── ws.rs           # WebSocket upgrade handler + ChatEvent enum
└── tests/
    ├── mod.rs
    ├── support.rs      # REST test helpers
    ├── ws_support.rs   # WS test helpers
    ├── rooms.rs        # room tests
    ├── messages.rs     # list/post message tests
    ├── edits.rs        # rate-limit + edit tests
    ├── delete.rs       # delete + broadcast tests
    ├── ws_basic.rs     # WS auth gate + event tests
    └── ws_presence.rs  # WS presence/typing tests
```

`permissions.rs` is the single source of truth for role helpers; `ws.rs`
imports from it instead of duplicating `resolve_role`.

---

## 2. Sub-state Pattern (CO-221)

Replace an 18-field `AppStateInner` god-state with four focused sub-states.
Handlers declare exactly the sub-state they need via Axum's `FromRef<AppState>`.

```rust
pub struct AppStateInner {
    pub core: Arc<CoreState>,        // storage, config, auth, event_bus
    pub realtime: Arc<RealtimeState>,// CRDT rooms, sync rooms, chat WS
    pub index: Arc<IndexState>,      // LRU cache, embedding service + sender
    pub integrations: Arc<IntegrationsState>, // mail, geo, plugins, JWT, workers
}
pub struct AppState(pub Arc<AppStateInner>);

impl FromRef<AppState> for Arc<CoreState> { ... }
impl FromRef<AppState> for Arc<RealtimeState> { ... }
// … same for IndexState, IntegrationsState
```

**Usage:** a handler that only reads storage + auth declares `State<Arc<CoreState>>`,
not `State<AppState>`. This makes the dependency explicit and keeps test setup
minimal.

```rust
async fn list_entries(State(core): State<Arc<CoreState>>, ...) { ... }
```

See `co-web/src/server/state.rs` for the full definitions.

---

## 3. Extractor Pattern (CO-222)

Express auth requirements in handler signatures via typed Axum extractors.
No more ad-hoc middleware layers or in-handler `resolve_role()` guards.

| Extractor | Requirement | Failure |
|-----------|-------------|---------|
| `AuthedUser` | Any authenticated user (JWT or session cookie) | 401 |
| `OwnerOf` | Authenticated + owns `{slug}` universe | 403 |
| `AdminUser` | Authenticated + `tier == "admin"` in JWT claims | 403 |
| `TokenOrJwtUser` | Authenticated via JWT **or** long-lived API token | 401 |

All four implement `FromRequestParts` and live in `co-web/src/auth/extractors.rs`.

```rust
// Auth requirement visible at a glance — no separate middleware layer
pub async fn delete_universe(
    owner: OwnerOf,          // 401 if not logged in, 403 if not owner
    State(core): State<Arc<CoreState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, AppError> { ... }
```

`OwnerOf` requires `{slug}` in the matched route pattern; returning 500 otherwise
is intentional — it is a programmer error, not a runtime one.

---

## 4. Event Bus Pattern (CO-220)

Decouple cross-feature signaling via an in-process publish-subscribe bus.
Publishers emit `DomainEvent` values; listeners run in dedicated `tokio::spawn`
tasks and react without polluting the emitting handler.

```rust
pub enum DomainEvent {
    EntryWritten { universe_key, path, body, body_hash },
    EntryDeleted { universe_key, path },
    NotificationRequested { recipient_id, kind, universe_key, ... },
    InvitationAccepted { universe_key, user_id },
    ProposalDecided { universe_key, proposal_path, action, proposer_id },
    AssetUploaded { universe_key, sha256, mime, size_bytes, user_id, filename },
}
```

`Bus` wraps a Tokio broadcast channel (capacity 2048). Listeners subscribe with
an `EventFilter` to receive only relevant variants.

```rust
// Publisher (invitation_routes.rs)
core.event_bus.publish(DomainEvent::NotificationRequested { ... });

// Listener (server/mod.rs — started at boot)
tokio::spawn(async move {
    let mut rx = s.core.event_bus.subscribe(EventFilter::Notification);
    while let Some(DomainEvent::NotificationRequested { .. }) = rx.recv().await {
        storage.create_notification(...);
    }
});
```

**Rule:** route handlers publish events only; they never call sibling route
modules directly. Side effects (notifications, embeddings, reference cards) are
implemented in listeners, not in the originating handler.

See `co-web/src/events.rs` for `Bus`, `DomainEvent`, `EventFilter`, `BusReceiver`.
Listeners are wired in `co-web/src/server/mod.rs`.

---

## 5. Worker Trait (CO-223)

All background polling workers implement a single trait so the supervisor can
manage lifecycle uniformly.

```rust
#[async_trait]
pub trait Worker: Send + 'static {
    fn name(&self) -> &'static str;
    fn interval(&self) -> Duration;
    async fn tick(&mut self) -> anyhow::Result<()>;
}
```

The five concrete workers are in `co-web/src/workers.rs`:

| Worker | Interval | Responsibility |
|--------|----------|----------------|
| `EmbeddingWorker` | 30 s | Probe embedding OS-thread; forward entry jobs |
| `EmailWorker` | 60 s | Deliver queued email digests |
| `PushWorker` | 10 s | Deliver queued web-push notifications |
| `WebhookWorker` | 5 s | Deliver outbound webhook payloads with backoff |
| `JobQueueWorker` | 3 s | Process `doc_gen` / `apply_template_all` jobs |

`WorkerSupervisor` (in `co-web/src/worker_supervisor.rs`) runs each worker in its
own `tokio::spawn` loop. A panic in one worker is caught via `JoinHandle::await`,
logged, and the worker is restarted with exponential backoff (5 s → 60 s max).
Status (tick count, error count, panic count, last tick) is exposed at
`GET /api/v1/admin/workers/status`.

```rust
// Registration at startup (server/mod.rs)
let sup = &state.integrations.worker_supervisor;
sup.spawn(EmbeddingWorker::new(...));
sup.spawn(EmailWorker::new(state.clone()));
sup.spawn(PushWorker::new(state.clone()));
sup.spawn(WebhookWorker::new(state.clone())?);
sup.spawn(JobQueueWorker::new(state.clone()));
```

**Rule:** new background polling work must implement `Worker` and register via
`sup.spawn(...)`. Never `tokio::spawn` a raw loop in a route module.

---

## 6. Platform vs. Universes Architecture Map (CO-265)

`co-web/src/` is split into two distinct layers:

**Platform** — reusable infrastructure shared by all universes:

| Directory | Purpose |
|-----------|---------|
| `auth/` | Authentication, extractors, onboarding, recovery |
| `content/` | Entries, vault, relations, references, universe CRUD |
| `social/` | Chat, DMs, invitations, notifications, sync/WS |
| `admin/` | Admin dashboard, gestão API, telemetry, UAT |
| `integrations/` | Email, GitHub, Google, OIDC, webhooks |
| `platform/` | Cross-cutting infra: config, error, events, embedding, workers |

**Universes** (`universes/`) — universe-specific extensions not part of the CO platform:

| Directory | Universe | Purpose |
|-----------|---------|---------|
| `universes/game/` | Yggdrasil | Leaderboard models + routes |

**Rule:** if a Rust module is specific to one universe (data models, custom routes, business logic that only applies to that universe's content), it belongs under `universes/<slug>/`. If it is reusable by any universe, it belongs in one of the platform directories above. CO is the **generalist** software: tenant-specific backends do not ship in this repo — a tenant's real app lives in its own standalone repo, and its content is served generically via the universe/content APIs.

**Re-exports:** `lib.rs` re-exports each universe sub-module at the crate root so existing call sites continue to compile unchanged.

**Future:** v2 will extract each `universes/<slug>/` into its own `co-universes-<slug>` crate (tracked as CO-N).

---

## 7. SPA Module Map (CO-259)

The three central SPA modules in `co-web/static/variants/a/modules/` were split
into folder-based submodules. Each `.js` proxy at the old path re-exports from
its `<name>/index.js` — no existing `import { … } from './sidebar.js'` needs to
change.

### `sidebar/`

| File | Contents |
|------|----------|
| `index.js` | Re-exports — preserves all `from './sidebar.js'` imports |
| `sections.js` | `buildChildMap`, `renderSectionHtml`, `renderUniverseItemHtml`, `renderInviteRowHtml`, `renderDiscoverableItemHtml` |
| `render.js` | `renderSidebar`, `injectSidebarCallbacks`, `injectSetUniverseSlugInUrl` |
| `header.js` | `renderHeader`, `renderHeaderUserArea`, `renderUsageCount`, `incrementLocalUsageCount`, `injectShowLoginModal` |
| `badge.js` | `renderUserBadge` |
| `mini-calendar.js` | `renderMiniCalendar`, `injectScrollToDate` |
| `wire.js` | `setupHamburgerMenu` |

### `state/`

| File | Contents |
|------|----------|
| `index.js` | Re-exports — preserves all `from './state.js'` imports |
| `shape.js` | The `state` object (all fields + defaults) |
| `universes.js` | `canEditCurrentUniverse` |
| `views.js` | `createViewDefaults()` — view-specific initial state values |

### `api/`

| File | Contents |
|------|----------|
| `index.js` | Assembles `api` object + re-exports `apiFetch`, `injectApiCallbacks` |
| `client.js` | `apiFetch`, `_u`, `injectApiCallbacks` |
| `auth.js` | `me`, `logout`, `loginWithPassword` |
| `tasks.js` | `getTasks`, `createTask`, `updateTask`, `deleteTask`, `getComments`, `createComment`, `getActivity`, `getDashboard`, `bulkUpdateTasks`, `bulkDeleteTasks` |
| `universes.js` | `getProjects`, `getUniverses`, `listUniverses`, `getUniverseInfo`, `getUniverseProjects`, `cloneUniverse`, `claimUniverse`, `getUniverseConfig`, `updateUniverseConfig`, `getPublicacoes`, `getEventos`, `getMissoes`, `getOplog`, `getOpDiff`, `revertToOp` |
| `entries.js` | `getUniverseEntries`, `getEntriesByDate`, `getUniverseManifest` |

**Rule:** parallel tasks that touch SPA behaviour must target one submodule
only. `sections.js`/`render.js`/`wire.js` handle disjoint sidebar concerns;
`auth.js`/`tasks.js`/`universes.js`/`entries.js` handle disjoint API domains.
New view-state fields go in `state/views.js`; universe helpers go in
`state/universes.js`.

---

## 8. R2 Deployer Feature Gate (CO-263)

`StaticOnR2Adapter` (in `core/src/deploy.rs`) and its AWS SDK dependencies are
gated behind the `deploy-r2` Cargo feature to keep the default binary free of
~3 MB of unused AWS SDK code.

**Default build (no feature):** `StaticOnR2Adapter` compiles to a disabled stub.
`from_credentials` returns an adapter whose deploy/rollback calls fail at runtime
with "R2 deployer disabled — rebuild with --features deploy-r2".

**To enable R2 deployment** in a consumer crate:

```toml
[dependencies]
co = { path = "../core", features = ["deploy-r2"] }
```

or from the CLI:

```bash
cargo build -p co-web --features co/deploy-r2
```

**Unit tests** never need the feature: `MockS3Backend` (generated by mockall on
`S3Backend`) is a dev-dependency and requires no AWS SDK. Only the
`#[ignore]` integration test (`test_integration_r2_deploy_and_rollback`) needs
`--features deploy-r2` to compile.

**When to enable:** wire this feature in the future `deploy.yaml`-driven UAT
revert flow (CO-N+). Until that pipeline is active, keep it disabled.

---

## 8. File-Compat Layer (CO-264)

Every CO universe behaves like a **filesystem-shaped wiki**. Well-known filenames
at any folder level have canonical rendering semantics:

| File path | Renders as |
|-----------|------------|
| `index.md` | Folder home page |
| `CHANGELOG.md` | Universe/folder changelog |
| `README.md` | Universe/folder documentation |
| `LICENSE.md` | Universe/folder license |

### URL conventions

| URL pattern | Resolution |
|-------------|------------|
| `/<universe>/changelog` | Renders the `CHANGELOG.md` entry (case-insensitive alias) |
| `/<universe>/readme` | Renders `README.md` |
| `/<universe>/license` | Renders `LICENSE.md` or `LICENSE` |
| `/<universe>/<folder>/` | Renders `<folder>/index.md` if present; otherwise folder listing |

### Backend: `path_prefix` query parameter

`GET /api/v1/universes/{slug}/entries?path_prefix=public/` returns all entries
whose path starts with `public/`. The filter is applied by
`EntryIndex::query_by_path_prefix` in `co-web/src/content/entry_index.rs`.

### Seeder: root-level well-known files

`Storage::reseed_co_root_files(root_dir)` (in `co-web/src/storage/seed.rs`)
seeds `CHANGELOG.md`, `README.md`, and `LICENSE.md` from the repo root into
the `co` universe as `page` entries on every boot. Called from
`run_co142_refresh` in `seed_orchestrator.rs`.

### SPA routing

`maybeOpenEntryFromUrl` in `app.js` extends the candidate list with well-known
file aliases so `/co/changelog` fetches `CHANGELOG.md` and `/co/public/` fetches
`public/index.md` before falling back to the 404 view.
