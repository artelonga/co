# Case Study — `alineaos/gerenciador-de-bibliotecas` as Central-Schema Reference

**Source**: <https://github.com/alineaos/gerenciador-de-bibliotecas>
**Stack**: Java 21 + Spring Boot + MySQL + JWT/RSA + Swagger + Docker
**Date analyzed**: 2026-06-06
**Companion**: CO-390 (the spike spec that proposes the proof-of-concept refactor)

## Why this repo

It's a clean, educational reference for the **central-schema + layered-architecture** pattern. Every entity (`User`, `Book`, `Genre`, `Loan`, `BookGenre`) has:
- Explicit DB schema with constraints (UK, FK, NOT NULL)
- Domain entity (JPA) — the source of truth shape
- DTO families per use case (Create / Update / Filter / Info / Basic / Response)
- Mapper layer translating domain ↔ DTO
- Repository for DB access
- Service for business rules
- Controller for HTTP surface
- Security layer (JWT/role-based)

CO currently mixes most of these into the route handler files. The case study asks: **what would adopting this layering look like for CO, in Rust + axum?**

## Patterns identified (8)

### Pattern 1 — Explicit DTO families per entity

Library-manager has 6 DTOs per entity:

| DTO | Purpose |
|---|---|
| `BookCreateRequest` | Input shape for `POST /books` |
| `BookCreateResponse` | Output shape after create |
| `BookUpdateRequest` | Input for `PUT /books/{id}` |
| `BookFilter` | Typed query params for `GET /books?...` |
| `BookBasicResponse` | Minimal response (for lists) |
| `BookInfoResponse` | Full response (for detail) |

**CO today**: one `Entry` struct serves all these purposes. Trade-offs are implicit (over-fetch, under-fetch, optional field bloat).

**Value of adopting**: explicit types prevent over/under-fetch, make client SDKs trivial to generate from OpenAPI (CO-350 catalog gets richer), surface intent at PR-review time.

**Cost**: more types to maintain (~6× per entity), more mapper code.

### Pattern 2 — Layered directory structure

```
src/main/java/librarymanager/
├── config/         JpaConfig, OpenApiConfig
├── controller/     BookController, GenreController, LoanController, UserController
├── domain/
│   ├── entity/     Book, Genre, BookGenre, Loan, User
│   └── enums/      LoanStatus, UserRole + converters
├── dto/            books/, genres/, loans/, users/, errors/
├── exception/      GlobalHandlerException
├── mapper/         BookMapper, etc.
├── repository/     BookRepository extends JpaRepository
├── security/       JwtService, SecurityConfig, etc.
└── service/        BookService — business rules
```

**CO today**: `co-web/src/` has features as siblings (`content/`, `auth/`, `admin/`) but within each feature, routes/DTOs/business logic/storage are not strictly separated.

**Value**: easier to navigate, clearer responsibilities, easier to test layers in isolation.

**Cost**: bigger directory tree, more files (~5× per feature), Rust's module system needs care to avoid deep nesting.

### Pattern 3 — Domain entities distinct from DB rows

Spring's `@Entity` maps Java classes to DB tables but the entity class is THE shape — the DTO layer adapts it for transport.

**CO today**: `Entry` struct comes from `rusqlite::Row` conversion; same struct serves DB + transport.

**Value**: changing the DB layout doesn't ripple into the API surface immediately; domain refactors stay internal.

**Cost**: explicit mapping code (the Mapper pattern).

### Pattern 4 — Repository interface above DB

`BookRepository extends JpaRepository<Book, Long>` — Spring generates queries from method names; custom queries are explicit `@Query`.

**CO today**: `Storage` struct holds a `Connection`; queries are inlined per use case across route handlers.

**Value of adopting (partially)**: clearer "data access surface" per entity; integration tests can mock the repository layer.

**Cost**: in Rust, repositories are typically traits; auto-generated query methods don't exist (would need a macro or sqlx-style approach).

### Pattern 5 — Service layer for business rules

`LoanService` enforces:
- "A new loan only if user has zero active loans"
- "A loan can be renewed exactly once"
- "Status transitions: BORROWED → RENEWED → OVERDUE → RETURNED"

These are in `LoanService`, NOT in `LoanController`.

**CO today**: business rules are inlined in route handlers (e.g. "anonymous user limit 100 entries" lives in the POST handler).

**Value**: rules become unit-testable without HTTP setup; rules can be reused across HTTP + WebSocket + CLI surfaces.

**Cost**: indirection; what was 1 file is now 3.

### Pattern 6 — Background workers for state transitions

Hourly cron checks `LOAN.due_at < now()` and updates status to `OVERDUE`. Scheduled via Spring's `@Scheduled`.

**CO today**: CO-337 (sister-repo sync), CO-365 (backup), CO-373 (Yggdrasil notes) already use this pattern via tokio tasks.

**Value**: ✅ already aligned.

**Cost**: n/a.

### Pattern 7 — Centralized exception handling

`GlobalHandlerException` returns typed error DTOs (`DefaultMessageError`, `ValidationMessageError`) consistently across all endpoints.

**CO today**: `AppError` enum exists; route handlers use `?` propagation; error responses are mostly consistent but not exhaustively documented in OpenAPI.

**Value of adopting fully**: every error shape is in OpenAPI; client SDKs handle them uniformly.

**Cost**: discipline; needs an exception → response mapping table per endpoint.

### Pattern 8 — Security at the security layer (not in controllers)

JWT validation + role check happens BEFORE the controller runs. `@PreAuthorize("hasRole('ADMIN')")` annotation gates the endpoint declaratively.

**CO today**: `require_admin` / `require_auth` middleware layer in axum routers. Similar in spirit.

**Value**: ✅ mostly aligned.

**Cost**: n/a; CO already does this.

## Mapping summary

| Pattern | CO has it? | Adoption recommendation |
|---|---|---|
| 1. DTO families | ❌ no | **adopt** — high client-SDK / API-clarity value |
| 2. Layered directories | ⚠️ partial | **adopt partially** — split per feature, don't force a global tree |
| 3. Domain ≠ DB row | ❌ no | **adopt** — couples internal refactors loose from API |
| 4. Repository pattern | ⚠️ partial | **adopt traits** — Storage trait per domain area |
| 5. Service layer | ❌ no | **adopt** — biggest testability win |
| 6. Background workers | ✅ yes | already aligned |
| 7. Centralized exception handling | ⚠️ partial | **adopt** — improves OpenAPI completeness (CO-350) |
| 8. Security layering | ✅ yes | already aligned |

## What CO-style adoption looks like (Rust + axum)

```
co-web/src/
├── domain/                       # Pure types, no axum/rusqlite deps
│   ├── entity/
│   │   ├── entry.rs              # struct Entry { id, universe_key, path, ... }
│   │   ├── user.rs
│   │   └── universe.rs
│   └── enums/
│       ├── entry_type.rs
│       ├── visibility.rs
│       └── lifecycle.rs
├── dto/                          # Transport types per endpoint
│   ├── entries/
│   │   ├── create_request.rs
│   │   ├── create_response.rs
│   │   ├── update_request.rs
│   │   ├── filter.rs
│   │   ├── basic.rs              # minimal — list views
│   │   └── info.rs               # full — detail views
│   ├── users/...
│   └── errors/
│       ├── default_message.rs
│       └── validation_message.rs
├── repository/                   # DB access traits + impls
│   ├── entry_repository.rs       # trait EntryRepository
│   ├── user_repository.rs
│   └── sqlite_impl/              # concrete impls behind the trait
├── service/                      # Business rules
│   ├── entry_service.rs          # fn create_entry(req, ctx) — validates + persists
│   ├── lifecycle_service.rs      # CO-385-style state transitions
│   └── lead_service.rs
├── controller/                   # HTTP / WS handlers (thin)
│   ├── entry_routes.rs
│   └── user_routes.rs
├── mapper/                       # Domain ↔ DTO
│   ├── entry_mapper.rs
│   └── user_mapper.rs
└── security/                     # Already exists; kept
```

Key invariants:
- `domain/` has NO axum / no rusqlite / no serde-json — pure types
- `dto/` derives `serde::{Serialize, Deserialize}` for transport
- `repository/` traits hide rusqlite; impls live behind `#[cfg]`
- `service/` returns `Result<DomainEntity, ServiceError>`; controller maps to DTO
- `controller/` is ~10 lines per route — validate, call service, map result, return

## Cost analysis

For the entries module (the CO-390 spike scope):

| Metric | Before (estimated) | After (estimated) | Delta |
|---|---|---|---|
| Files touched per entity | ~3 | ~8 | +5 |
| LOC per entity | ~600 | ~900 | +50% |
| Test coverage (unit) | low (HTTP integration tests dominate) | high (services testable in isolation) | ++ |
| Test coverage (integration) | unchanged | unchanged | 0 |
| Build time | 0 | +5-10% (more files, more codegen) | small regression |
| New-contributor ramp time | 1-2 weeks | 1 week (clearer structure) | ✅ |
| OpenAPI completeness | partial (CO-350 enforced) | full (every DTO is documented) | ✅ |
| Migration risk per refactor | high (one big diff) | low (per-layer diffs) | ✅ |

The 50% LOC increase is real but the testability + clarity + OpenAPI gains likely outweigh it.

## Risks

1. **Rust ergonomic friction** — Java/Spring has lots of framework support (`@Service`, `@Repository`, `JpaRepository`). Rust doesn't; the layering is manual. Some patterns translate naturally (traits), others (auto-derived repositories) need either a macro or accepting more boilerplate.
2. **Big-bang refactor failure mode** — Doing all entities at once would be a months-long blocker. Phased adoption (one entity at a time) is safer; CO-390 spike scope is ONE entity (entries).
3. **Backward compatibility on the wire** — DTO families are an internal refactor; the HTTP responses on the wire must stay stable. The spike must include before/after schema comparison to verify zero API drift.
4. **Library-manager is single-app** — CO is multi-deployment (apex, surfaces, federation). Some patterns (e.g. global `JpaConfig`) don't transfer cleanly; need to think per-feature.

## Decision pending — the spike will answer

The CO-390 spike refactors the `entries` module under this layering, measures the metrics above, and recommends:
- **Adopt fully**: refactor all features (~3-6 month effort, paired with CO-227)
- **Adopt partially**: keep DTO families + service layer; skip strict directory tree
- **Reject**: the Rust ergonomic cost dominates the gains for our scale

Recommendation will go into `docs/spikes/library-manager-decision.md` after the spike completes.

## Connection to existing roadmap

This case study INFORMS:
- **CO-227** (server decomposition, ongoing) — the layering pattern IS the decomposition
- **CO-228** (type safety, ongoing) — DTO families maximize type safety at the wire
- **CO-350** (OpenAPI codegen + drift check) — better DTO discipline makes the generated YAML richer

Not in scope:
- v3.0 (Wave 4) — public launch, no refactor pull
- v3.1 (Wave 5) — feature deepening, no refactor pull
- v3.2 (Wave 6) — security epic, only adopt insofar as security DTOs benefit

The spike sits in v3.x "ongoing refactors" track. Its output (yes/partial/no decision) gates whether CO-227 adopts this specific shape or another.
