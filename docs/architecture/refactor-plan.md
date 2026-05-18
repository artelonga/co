# CO — Refactor Plan: gap analysis vs seven principles

**Snapshot:** 2026-05-13 · workspace 2.8.1
**Next CO-N ID base:** `work/co/project.yaml` → `next_id: 214` → new tasks start at **CO-214** (note CO-214 already used for token exchange in the codebase comments; reserving **CO-215 onward** here to be safe).
**Audit scope:** Rust side (`co-web/src/`) + SPA (`co-web/static/variants/a/`). The SPA decomposition is already covered by [`docs/architecture-review.md`](../architecture-review.md); this plan does NOT duplicate those phases — it references them and adds tasks for the Rust gaps only, plus two cross-stack tasks that complement the review.

**Principles checklist (recap):**
1. Composition over inheritance
2. Single responsibility (one file = one concern)
3. Static typing (no `serde_json::Value` ad-hoc; SPA → TS)
4. Reduced coupling (no reaching into internals; no god-files)
5. Segregated state (no global mutable; explicit DI over inject-callback)
6. Context-based graph (folders encapsulate features — `modules/auth/`, etc.)
7. Event-driven where signals matter (queues, not direct cross-feature calls)

---

## Top-line findings

| # | Finding | Principles violated |
|---|---|---|
| F1 | `server.rs` (1570 LoC) is a god-file: router + AppState + handler defs + validators + UAT bootstrap + seed orchestration | 2, 4 |
| F2 | `AppStateInner` has 18 unrelated fields injected into *every* handler via `State<AppState>` | 1, 4, 5 |
| F3 | 7 route modules are 1k+ LoC each (chat 2063, vault 1519, reference 1469, entry 1372, telemetry 1354, dm 1143, invitations 1152) — single file = many concerns | 2, 4 |
| F4 | `storage.rs` ↔ `server.rs` import cycle (storage imports `crate::server::AppState`); blocks lifting storage into its own sub-crate | 4 |
| F5 | 180 occurrences of `serde_json::Value` in Rust handlers — many are typed payloads being passed as opaque maps | 3 |
| F6 | Cross-feature direct calls (invitation → notification, proposal → notification, entry-write → embedding) — no event bus, just function calls | 7 |
| F7 | SPA inject-callback IoC + 19 ad-hoc `injectXxxCallbacks()` patterns — already documented in architecture-review.md | 1, 5 |
| F8 | No `modules/` folder hierarchy in `co-web/src/` — 90 flat files; only `storage/`, `server/`, `recovery_routes/`, `onboarding_routes/` use the dir pattern | 6 |
| F9 | Mixed auth gating styles — sometimes `.layer(require_auth)`, sometimes `require_auth_with_token`, sometimes in-handler `resolve_role()`; no unifying type | 4, 5 |
| F10 | Two workers panic-poison risk addressed (per `feedback_no_panic_under_mutex`) but worker pattern still varies (channel, DB-polling, in-process) — no shared "Worker" trait | 1, 7 |

---

## Proposed CO-N tasks (priority order)

### HIGH priority

---

#### CO-215 — Split `server.rs` into `server/{router, state, validation, uat_boot, seed_orchestrator}.rs`

- **Principles:** 2 (SRP), 4 (coupling)
- **Scope:** Move `build_router()` into `server/router.rs`, `AppStateInner` + lock helpers into `server/state.rs`, the validator functions (validate_task_title/description/comment/project_name/etc.) into `server/validation.rs`, the UAT startup tasks into `server/uat_boot.rs`, the seed-on-boot orchestration into `server/seed_orchestrator.rs`. `server/mod.rs` re-exports the same public surface so the rest of the workspace doesn't need to change.
- **Acceptance:**
  - `co-web/src/server.rs` is replaced by `co-web/src/server/` directory.
  - No file in `server/` exceeds 500 LoC.
  - `cargo test -p co-web` passes unchanged.
  - `cargo clippy -p co-web -- -D warnings` clean.
  - Public re-exports from `crate::server::{AppState, AppStateInner, start_server, build_router}` unchanged.
- **Blast radius:** ~1600 LoC moved, ~5 LoC of import changes per call site (search for `crate::server::` in route modules — ~30 hits, mostly already qualified).
- **Why high:** Every downstream refactor touches `server.rs`. Splitting it first unblocks F2 + F4.

---

#### CO-216 — Break the `storage` ↔ `server` import cycle

- **Principles:** 4 (coupling)
- **Scope:** `co-web/src/storage.rs` and submodules currently `use crate::server::AppState`. This is the only thing preventing `storage` from being lifted into its own crate. Replace those usages with a narrower trait (`StorageContext`) that only exposes what storage actually needs (data_dir, cache handle, maybe rate_limiter). `AppState` implements it. Storage modules depend on the trait.
- **Acceptance:**
  - No `use crate::server::` in any file under `co-web/src/storage*`.
  - New trait `StorageContext` defined in `storage/context.rs`.
  - All storage methods that took `&AppState` now take `&dyn StorageContext` (or generic `<C: StorageContext>`).
  - `cargo test` passes.
- **Blast radius:** ~50-80 call sites in `storage/*` + a one-line `impl StorageContext for AppStateInner` in server. Net ~300 LoC touched.
- **Why high:** Unblocks lifting storage into a sub-crate (future modularity); fixes the architectural cycle flagged in as-is.md §4.

---

#### CO-217 — Introduce typed request/response structs for the top 20 `serde_json::Value` handler payloads

- **Principles:** 3 (typing)
- **Scope:** Pick the 20 hottest `serde_json::Value` sites in `co-web/src/*_routes.rs` (180 total occurrences — start with telemetry, entry payloads, vault clip, push subscription payloads, oauth responses). Replace with `#[derive(Deserialize)] struct XxxRequest`. Keep `serde_json::Value` only where the payload is genuinely free-form (e.g. plugin manifests, registry passthrough).
- **Acceptance:**
  - 20 named structs added (target list in PR description).
  - Each replaced handler has at least one unit test exercising the parse.
  - Remaining `serde_json::Value` usages have a one-line `// FREEFORM: <reason>` comment.
- **Blast radius:** ~500 LoC added (struct defs + tests), ~200 LoC handler simplification.
- **Why high:** Compounds — once types exist they can flow into the interactions registry → OpenAPI surface (today only entries/vault are covered there).

---

#### CO-218 — Migrate SPA to TypeScript (incremental, file-by-file)

- **Principles:** 3 (typing), 5 (segregated state), 6 (context-based graph)
- **Scope:** Add `tsconfig.json` to `co-web/static/variants/a/`. Convert files in the order suggested by `docs/architecture-review.md` §4: pure helpers first (`url.js`, `helpers.js`, `constants.js`, `state.js`), then API client (`api.js`), then modules/views, then modals last. Build step (esbuild or tsc → js) wired into the workspace build. The existing inject-callback pattern becomes typed callback interfaces.
- **Acceptance:**
  - At least 4 files migrated in the first PR (helpers/url/constants/state).
  - Type-check passes in CI.
  - No runtime behavior change (Playwright e2e green).
  - Subsequent PRs convert one module at a time.
- **Blast radius:** ~1500 LoC touched in first PR; ~7000 LoC total over many PRs.
- **Why high:** This is the cross-stack typing win + sets up real DI for the modules.

---

### MEDIUM priority

---

#### CO-219 — Promote `chat_routes.rs` (2063 LoC) into `chat/` module folder

- **Principles:** 2 (SRP), 6 (context-based graph)
- **Scope:** New folder `co-web/src/chat/` with `routes.rs` (router only), `rooms.rs` (room CRUD), `members.rs`, `messages.rs` (post/edit/delete), `permissions.rs` (resolve_role + visibility), `ws.rs` (move from chat_ws.rs). `chat/mod.rs` re-exports the existing public functions (`chat_router`, `chat_ws_handler`) so server.rs needs no change. Mirror the pattern for the other 1k+ files (vault, reference, entry, telemetry, dm, invitations) in follow-up tasks.
- **Acceptance:**
  - No file in `chat/` exceeds 500 LoC.
  - `chat_router()` and `chat_ws_handler` signatures unchanged.
  - All existing chat tests pass.
  - Document the pattern in a `co-web/src/MODULES.md` so the same shape applies to follow-up extractions.
- **Blast radius:** ~3300 LoC moved (chat_routes + chat_ws together), zero LoC change at call sites.
- **Why medium:** Same story for vault, reference, entry, telemetry, dm, invitations — 6 follow-up tasks would each be a copy of this shape. Open as a tracking epic.

---

#### CO-220 — Replace direct cross-feature function calls with an in-process event bus

- **Principles:** 7 (event-driven), 4 (coupling)
- **Scope:** Today `invitation_routes` calls into `notification_routes`, `proposal_routes` calls into `notification_routes`, `entry_routes` write paths call into `embedding_worker::queue()` directly. Introduce a thin event bus (`crate::events::Bus` — a tokio broadcast or an mpsc fan-out) carrying typed `DomainEvent` (EntryWritten, NotificationRequested, InvitationAccepted, ProposalDecided). Move emit/listen patterns to it. Existing handlers emit; workers + sibling modules subscribe.
- **Acceptance:**
  - `crate::events::DomainEvent` enum defined.
  - At least 4 cross-feature direct calls replaced (invitation→notification, proposal→notification, entry-write→embedding, asset-upload→reference-card).
  - Workers subscribe via `bus.subscribe(EventFilter::...)`.
  - Existing tests pass; new tests verify emission + subscription.
- **Blast radius:** ~400 LoC new (events module + plumbing), ~150 LoC simplified in routes.
- **Why medium:** Big win for coupling but only meaningful after CO-215 lands (need a clean home for the bus, probably `server/events.rs`).

---

#### CO-221 — Slim `AppStateInner` via segregated sub-states

- **Principles:** 5 (segregated state), 1 (composition)
- **Scope:** Split `AppStateInner`'s 18 fields into 4 sub-states: `CoreState` (storage, config, auth), `RealtimeState` (doc_rooms, sync_rooms, chat_rooms, chat_presence), `IndexState` (cache, embeddings, embedding_tx), `IntegrationsState` (mail, geo, plugin_registry, game_storage, wae, jwt_key, rate_limiter, experiment). `AppStateInner` keeps `Arc`s to each. Handlers extract only the sub-state they need via `State<Arc<CoreState>>` (axum supports this via `FromRef`).
- **Acceptance:**
  - Four new sub-state structs.
  - At least 10 handlers updated to take a narrow sub-state.
  - `cargo test` passes.
  - Existing `State<AppState>` extractor still works for handlers not yet migrated.
- **Blast radius:** ~200 LoC structural change, opt-in per handler thereafter.
- **Why medium:** Best done after CO-215; the value compounds as more handlers migrate.

---

#### CO-222 — Unify auth gating into a single typed extractor

- **Principles:** 4 (coupling), 5 (state)
- **Scope:** Today auth is enforced via three different shapes — `.layer(require_auth)`, `.layer(require_auth_with_token)`, and in-handler `resolve_role()`. Define a typed axum extractor hierarchy: `AuthedUser`, `OwnerOf(slug)`, `AdminUser`, `TokenOrJwtUser`. Each implements `FromRequestParts`. Handlers express their requirement in the signature, not in the route wiring.
- **Acceptance:**
  - 4 extractor types defined under `auth/extractors.rs`.
  - At least 10 handlers migrated.
  - Existing middleware-based gating still works in parallel during migration.
  - Compile error if a handler claims `OwnerOf` but the slug param is absent.
- **Blast radius:** ~300 LoC for extractors + migration of ~10 handlers per PR.
- **Why medium:** Real correctness win — current pattern lets gates be missing silently.

---

#### CO-223 — Define a shared `Worker` trait + lifecycle

- **Principles:** 1 (composition), 7 (event-driven)
- **Scope:** Workers today follow 3 patterns: mpsc channel (embedding_worker), DB polling (notification_*_worker, webhook_worker), in-process buffer (wae). Define `trait Worker { fn name; async fn tick; async fn run_loop; }` and a `WorkerSupervisor` that owns the JoinHandles + shutdown. Each existing worker implements it. Cross-references `feedback_no_panic_under_mutex` — supervisor catches panics and restarts (without poisoning).
- **Acceptance:**
  - Trait defined.
  - 5 workers migrated.
  - Supervisor tracks last-tick timestamp, exposes `/api/v1/admin/workers/status`.
  - Panic in one worker doesn't bring down siblings.
- **Blast radius:** ~600 LoC trait + supervisor + migrations.
- **Why medium:** Operational visibility + safer panic handling.

---

### LOW priority

---

#### CO-224 — Promote routes into context folders (`modules/auth/`, `modules/content/`, `modules/social/`, `modules/admin/`)

- **Principles:** 6 (context-based graph)
- **Scope:** After CO-215 + CO-219 patterns are proven, group the 90 flat files into context folders: `auth/`, `content/` (entries/vault/references/relations/proposals/state/branches), `social/` (chat/dm/notifications/invitations/push), `admin/` (admin/gestao/uat/dev_board/storage_dashboard), `integrations/` (oauth_google/oidc/github_auth/log_drain/webhooks), `platform/` (geo/cache/rate_limit/telemetry/wae/ab/experiment), `quilombo/`, `game/`. Re-export from `lib.rs` to preserve external paths.
- **Acceptance:**
  - No file at the top level of `co-web/src/` except `lib.rs`, `main.rs`, `bin/`, and context folders.
  - All call sites updated (mostly internal).
- **Blast radius:** Mostly mechanical — file moves + import-path updates. ~90 files touched, 0 LoC of logic change.
- **Why low:** Cosmetic until the underlying SRP work (CO-219 follow-ups) lands. Doing this first would be polishing.

---

#### CO-225 — Document the AppState composition pattern + add a `MODULES.md`

- **Principles:** all
- **Scope:** A single `co-web/src/MODULES.md` codifying: the directory pattern (CO-215, CO-219), the sub-state pattern (CO-221), the extractor pattern (CO-222), the event bus pattern (CO-220), the worker trait (CO-223). New code follows it; PR reviewers point to it.
- **Acceptance:**
  - `MODULES.md` exists, ~150 LoC.
  - Cross-linked from `docs/architecture/as-is.md` and `CLAUDE.md`.
- **Blast radius:** Docs only.
- **Why low:** Only meaningful once at least 3 of the above patterns ship.

---

#### CO-226 — Add OpenAPI coverage for auth + admin + chat to the interactions registry

- **Principles:** 3 (typing)
- **Scope:** Today `registry.yaml` covers content ops only. Extend to auth flows, admin endpoints, chat. Generated OpenAPI at `/api/v1/interactions/openapi.json` becomes the typed contract for the SPA's TypeScript migration (CO-218) to consume via codegen.
- **Acceptance:**
  - `registry.yaml` covers ≥80% of routes catalogued in `docs/architecture/api-catalog.md`.
  - OpenAPI surface validates against spec.
- **Blast radius:** YAML edits — ~500 lines of YAML, no Rust changes.
- **Why low:** Big payoff once CO-218 needs it; premature otherwise.

---

## Summary table

| ID | Title | Priority | Blast |
|---|---|---|---|
| CO-215 | Split `server.rs` into folder | HIGH | ~1600 LoC moved |
| CO-216 | Break storage↔server cycle | HIGH | ~300 LoC |
| CO-217 | Typed structs for top-20 Value payloads | HIGH | ~700 LoC |
| CO-218 | SPA → TypeScript (incremental) | HIGH | ~7000 LoC over many PRs |
| CO-219 | Chat module folder (pattern for 6 follow-ups) | MED | ~3300 LoC moved |
| CO-220 | Cross-feature event bus | MED | ~550 LoC |
| CO-221 | Segregate `AppStateInner` into sub-states | MED | ~200 LoC structural |
| CO-222 | Typed auth extractors | MED | ~300 LoC + migrations |
| CO-223 | Shared `Worker` trait + supervisor | MED | ~600 LoC |
| CO-224 | Context folders for routes | LOW | mechanical, 90 files |
| CO-225 | `MODULES.md` pattern doc | LOW | docs only |
| CO-226 | Extend interactions registry coverage | LOW | YAML only |

**Sequencing:** CO-215 → CO-216 → (CO-217 + CO-218 in parallel) → CO-221 → CO-220 + CO-222 + CO-223 → CO-219 (and its 6 siblings) → CO-224 → CO-225 → CO-226.
