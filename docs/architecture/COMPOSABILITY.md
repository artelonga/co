# CO Composability — the external-consumer map

> How external services (universes, Yggdrasil, CLI/mobile, future S3) consume
> `co` components **without forking core code**. Produced by the **Mythos** epic
> (CO-430), 2026-06-13. Every seam below is `trait at the boundary, impl behind
> it` — you add an `impl` + a registration, never edit an existing match/spawn.

## 1. Reusable crates (depend on these, not the server)

| Crate | What it gives you | Server-free? |
|-------|-------------------|--------------|
| **`co`** (`core/`) | Domain types + the **`Universo`** trait + `UniversoFactory`; Graph/Node/Edge, Feature/Schema, deploy adapters | ✅ no axum/tokio/rusqlite (`grep -r axum core/src` empty) |
| **`game-core`** | 2D engine (Universe/Session/Interpreter/Renderer) + games + the **`Plugin`** trait | ✅ **axum-free since CO-436** — `Plugin::routes()` returns portable `Vec<RouteDescriptor>`, not `axum::Router`. `cargo tree -p game-core \| grep axum` = empty |

External Rust consumers (Yggdrasil, a CLI, a mobile runtime) depend on `co`/`game-core`
directly. They do **not** pull `co-web` (the server) to reuse the domain or the engine.

## 2. Extension seams (registry + trait + default impl)

Add a new variant by implementing the trait and registering it — the boot wires
the defaults, your `impl` joins them, no edit to the boot block.

| Seam | Trait | Where | Add a new one by |
|------|-------|-------|------------------|
| Universe backend | `UniversoFactory` → `Universo` | `core` + co-web | inject a factory into `AppState` (default = filesystem `UniversoLocal`) |
| Per-universe storage | `EntryStore` (`SqliteEntryStore` default) | `co-web/src/repository` | `UniversePool::entry_store()` is the factory — swap the backend (SQLite → S3/Postgres) behind the trait |
| Event subscriber | `EdaSubscriber` | `co-web/src/eda/subscriber_registry.rs` | `impl EdaSubscriber` + `registry.register(...)`; boot iterates `default_registry()` |
| Content source | `SourceAdapter` (github default) | `co-web/src/platform/source.rs` | `impl SourceAdapter` (gitlab/notion/…) + register |
| Admin auth | `AdminAuthProvider` (github PAT default) | `co-web/src/infra/admin_auth.rs` | `impl` for SAML/OIDC + inject |
| Rate limiting | `RateLimiter` (in-process default) | `co-web/src/platform/rate_limit.rs` | `impl` (Redis sliding-window) |
| Secrets / config | `SecretsProvider` + `CoServerConfig` | `co-web/src/infra/secrets.rs` | the **only** `std::env::var` read is `EnvSecretsProvider`; swap it (Vault/AWS SM) at boot |
| Blob store | `BlobStore` (LocalFS default, R2/S3) | `co-web/src/infra/blob.rs` | config-selected backend |
| Auth | `AuthProvider` | `co-web/src/infra/auth.rs` | OAuth/SAML impls |
| Telemetry | `tracing` + OTLP exporter | `co-web/src/infra/telemetry.rs` | OTLP env (CO-291) |
| Game plugin | `Plugin` → `Vec<RouteDescriptor>` | `game-core/src/plugin.rs` | framework-agnostic; the host translates descriptors to its router |

## 3. Integration contracts (no code, just talk to the running server)

- **HTTP / Vault API** — Obsidian-compatible, transport-agnostic:
  `GET/PUT/DELETE /api/v1/universes/{slug}/vault/{*path}`, `POST …/publish`,
  universe CRUD, `/usage/*` (admin). Auth: bearer API token (vault) or session JWT.
- **Event bus** — `EdaBus`: JSON `Event { event_type, universe_key, payload,
  visibility, … }`. Spawn an `EdaSubscriber` to consume; publish via the bus.
  Federation already runs this way (`yggdrasil_notes`, `comunicacao_live`).
- **Cross-universe links** — canonical `key::path` (storage/frontmatter),
  `[[key::path]]` (markdown body).

## 4. Known residuals (tracked, not hidden)

- **Layered template** (domain/dto/repository/service/mapper) covers
  entries + references + relations. `universes` + `vault` are partially layered —
  finish is incremental (no consumer impact; the seams above are stable).
- **Raw SQL in content handlers** (workspace/assets/vault/op_log/template) →
  **CO-441** (move to storage methods; security/auth/eda already done in CO-433).

## 5. Productizing the seams

- **StaaS — storage as a service for partners** → [`staas-partners.md`](staas-partners.md)
  (CO-460): multi-tenant design that productizes the **Per-universe storage** and
  **Blob store** seams above (`StorageBackend`/`BlobStore`) into partner namespaces,
  with capability-scoped auth (CO-448), metering (CO-453), and quota (CO-80).
