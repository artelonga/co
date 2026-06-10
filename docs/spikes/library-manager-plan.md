# CO-390 — Day 1 Plan: Layered Architecture Spike (entries module)

**Date**: 2026-06-10
**Author**: Claude (automated via co-auto)
**Branch**: `feat/CO-390-spike-layered-architecture-domain-dto-re`
**Case study reference**: `docs/spikes/library-manager-case-study.md`
**Decision output**: `docs/spikes/library-manager-decision.md`

---

## Inventory — current entries shape

### Files (before spike)

| File | LOC | Concern mix |
|------|-----|-------------|
| `core/src/entry.rs` | 541 | Domain entity + file I/O + parsing + tests |
| `co-web/src/content/entry_routes.rs` | 1924 | HTTP handlers + business rules + DTO types + protobuf encoding |
| `co-web/src/content/entry_index.rs` | 1239 | SQLite index + EntryRow DTO + date indexing + tests |
| **Total** | **3704** | Three files mixing all concerns |

### Concern map (current state)

```
entry_routes.rs
├── DTO types (CreateEntryBody, UpdateEntryBody, EntryListQuery, EntryListResponse, ...)
├── Business rules
│   ├── Anonymous 100-entry quota check          ← EXTRACT to service
│   ├── Event-bus universe read-only check       ← EXTRACT to service
│   ├── Manifest-based frontmatter validation    ← EXTRACT to service
│   ├── public/ convention anon filter           ← EXTRACT to service
│   ├── published-only anon filter               ← EXTRACT to service
│   └── review-status visibility filter          ← EXTRACT to service
├── HTTP handlers (list, get, create, update, delete, history, tags, tree, ...)
│   ├── Storage orchestration (read-write-event-telemetry-cache)
│   └── Protocol negotiation (JSON vs protobuf)
└── Protobuf helpers (entry_row_to_proto, json_value_to_proto)

entry_index.rs
├── EntryRow (database row = API DTO today)      ← SPLIT into domain + DTO
├── EntryIndex struct (SQLite operations)        ← BECOMES repository impl
├── EntryEventRow, TagCount, TreeNode
└── SQL helpers (date normalization, etc.)
```

---

## Target structure (spike deliverables)

```
co-web/src/
├── domain/
│   ├── mod.rs
│   └── entity/
│       ├── mod.rs
│       └── entry.rs          NEW: EntryDomain — pure type, no axum/rusqlite
├── dto/
│   ├── mod.rs
│   └── entries/
│       ├── mod.rs
│       ├── create_request.rs  NEW: POST /entries body
│       ├── create_response.rs NEW: 201 response
│       ├── update_request.rs  NEW: PUT /entries/:path body
│       ├── filter.rs          NEW: GET /entries query params
│       ├── basic.rs           NEW: list-view entry (fewer fields)
│       └── info.rs            NEW: detail-view entry (with relations)
├── repository/
│   ├── mod.rs
│   └── entry_repository.rs   NEW: EntryRepository trait + SqliteEntryRepository
├── service/
│   ├── mod.rs
│   └── entry_service.rs      NEW: EntryService — pure business rules
├── mapper/
│   ├── mod.rs
│   └── entry_mapper.rs       NEW: EntryMapper — domain ↔ DTO conversions
└── content/
    └── entry_routes.rs       MODIFIED: thin controller, delegates to service
```

### New files: 12
### Modified files: 2 (entry_routes.rs + lib.rs)
### Deleted files: 0 (spike branch only — no merge)

---

## File-by-file plan

### `co-web/src/domain/entity/entry.rs`

```rust
pub struct EntryDomain {
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    pub title: Option<String>,
    pub frontmatter: serde_json::Value,
    pub body: String,
    pub body_hash: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}
```

**Invariant**: no `use axum::*`, no `use rusqlite::*`.
**Purpose**: decouple internal refactors from the HTTP/DB surface.

### `co-web/src/dto/entries/`

Six DTOs mirroring the library-manager pattern:

| File | Type | Description |
|------|------|-------------|
| `create_request.rs` | `CreateEntryRequest` | `POST /entries` body |
| `create_response.rs` | `CreateEntryResponse` | 201 body — wire-identical to current `EntryRow` |
| `update_request.rs` | `UpdateEntryRequest` | `PUT /entries/:path` body |
| `filter.rs` | `EntryFilter` | `GET /entries?…` typed query params |
| `basic.rs` | `EntryBasicDto` | List-view row (path, type, title, updated_at) |
| `info.rs` | `EntryInfoDto` | Detail-view (full + outbound relations) |

Wire compatibility invariant: `CreateEntryResponse` must serialize identically to
the current `EntryRow` response. Verified by the JSON fixture comparison in Day 3.

### `co-web/src/repository/entry_repository.rs`

```rust
pub trait EntryRepository: Send + Sync {
    fn find(&self, universe_key: &str, path: &str) -> anyhow::Result<Option<EntryDomain>>;
    fn list(&self, universe_key: &str, entry_type: &str,
            filter: &serde_json::Value, limit: Option<usize>) -> anyhow::Result<Vec<EntryDomain>>;
    fn upsert(&self, universe_key: &str, entry: &EntryDomain) -> anyhow::Result<()>;
    fn delete(&self, universe_key: &str, path: &str) -> anyhow::Result<()>;
    fn count(&self, universe_key: &str, entry_type: Option<&str>) -> i64;
}

pub struct SqliteEntryRepository { ... }
impl EntryRepository for SqliteEntryRepository { ... }
```

**Note**: Rust trait lifetime constraints make this harder than Java's `JpaRepository`.
`EntryIndex<'a>` takes `&'a Connection` — the impl needs to manage connection lifetime
carefully. The spike measures the ergonomic cost of this.

### `co-web/src/service/entry_service.rs`

Extracts the following business rules as unit-testable free functions:

| Rule | Extracted from |
|------|----------------|
| `check_anon_quota(count)` | `create_entry` handler — inline if |
| `check_not_event_bus(source_kind)` | `create_entry` / `update_entry` |
| `validate_entry_type(manifest, type, fm)` | `validate_against_manifest()` — already extracted; move to service |
| `apply_public_convention(entries, is_anon, slug, pub_sub)` | `filter_public_for_anon()` |
| `apply_published_filter(entries, is_anon, anon_pub_only)` | `filter_published_for_anon()` |
| `apply_review_filter(entries, is_owner, viewer_key)` | `filter_review_status()` |

All rules are **pure functions** (no I/O, no async). The service struct wraps them.
Tests can call them directly without HTTP setup.

### `co-web/src/mapper/entry_mapper.rs`

```rust
pub struct EntryMapper;

impl EntryMapper {
    pub fn row_to_domain(row: EntryRow) -> EntryDomain { ... }
    pub fn domain_to_create_response(domain: &EntryDomain) -> CreateEntryResponse { ... }
    pub fn row_to_basic(row: EntryRow) -> EntryBasicDto { ... }
    pub fn row_to_info(row: EntryRow, relations: Vec<RelationRow>) -> EntryInfoDto { ... }
}
```

### `co-web/src/content/entry_routes.rs` (refactored)

Thin controller: `create_entry` delegates to `EntryService`:

```rust
// Business rules → service
EntryService::check_anon_quota(universe.content_count)?;
EntryService::validate_entry_type(manifest.as_deref(), entry_type, &body.frontmatter)?;

// Visibility filters → service  
let entries = EntryService::apply_public_convention(entries, is_anon, &slug, pub_sub);
let entries = EntryService::apply_published_filter(entries, is_anon, anon_published_only);
let entries = EntryService::apply_review_filter(entries, is_owner, &viewer_key);

// Response → mapper
let response = EntryMapper::domain_to_create_response(&domain);
```

Infrastructure concerns (file I/O, event bus, telemetry, cache invalidation) stay in
the controller — they require I/O and are tested via integration tests.

---

## LOC budget estimate

| Layer | Estimated LOC | Notes |
|-------|---------------|-------|
| `domain/entity/entry.rs` | ~30 | Simple struct + basic impl |
| `dto/entries/` (6 files) | ~120 | ~20 LOC per DTO |
| `repository/entry_repository.rs` | ~120 | Trait ~20 + impl ~100 |
| `service/entry_service.rs` | ~120 | 6 business rules extracted |
| `mapper/entry_mapper.rs` | ~80 | 4 mapping functions |
| `entry_routes.rs` delta | ~-100 | Extracted logic |
| **Net new** | **~370** | |

**Hypothesis 3 budget**: < 60% LOC overhead per entity.
Baseline: entry module = ~3704 LOC.
Budget: +2222 LOC max.
Projected: +370 LOC (~10% overhead) — well within budget if hypothesis holds.

---

## Risk register

| Risk | Mitigation |
|------|------------|
| Rust trait lifetimes break `EntryRepository` impl | Use `Arc<Mutex<Connection>>` in impl; note ergonomic cost in decision doc |
| entry_routes.rs refactor breaks existing tests | Keep handler signatures + response shapes byte-identical; run `cargo test` after each change |
| Wire incompatibility | Capture JSON fixture from main before spike; diff after |
| Build time regression | Measure with `time cargo build` before and after |

---

## Success criteria for Day 2

- [ ] All 12 new files compile cleanly
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` green (same tests as main)
- [ ] `create_entry` uses `EntryService::check_anon_quota` + `validate_entry_type`
- [ ] `list_entries` uses `EntryService::apply_*` filter functions
- [ ] Wire compatibility: `CreateEntryResponse` serializes identically to `EntryRow`
