# Cross-repo architecture audit — 2026-05-17

Six parallel audits, one shape. Each repo now has `docs/architecture/{as-is,api-catalog,refactor-plan}.md`. This document rolls them up so we can see the system whole and prioritize across boundaries.

## Repos audited

| Repo | Path | Stack | Lines of audit |
|---|---|---|---|
| `co` | `/Users/artelonga/projects/co` | Rust workspace (Axum + SQLite) + vanilla SPA | 862 |
| `rfq-gateway` | `/Users/artelonga/projects/rfq-gateway` | Rust + Axum + market adapters | 619 |
| `quilombo-blog` | `/Users/artelonga/projects/quilombo-blog` | SvelteKit + Node 22 + SQLite | 611 |
| `ArteLonga` | `/Users/artelonga/projects/ArteLonga` | Vanilla JS + opt-in TS, GitHub Pages | 641 |
| `yggdrasil` | `/Users/artelonga/projects/yggdrasil` | Rust + Axum + per-game SQLite | 563 |
| `comunicacao` | `/Users/artelonga/projects/comunicacao` | CO content universe (no app) | 443 |
| **Total** | | | **3 739 lines** |

Audits wrote no code, committed nothing. All files staged for review.

---

## Cross-cutting findings — same disease, six bodies

### 1. God-files everywhere (SRP violation, top of every list)

| Repo | Worst offender | LoC | What's mixed |
|---|---|---|---|
| `co` | `co-web/src/server.rs` | 1 570 | router + `AppStateInner` (18 unrelated fields) + handler validators + UAT bootstrap + seed orchestration |
| `co` (also) | `chat_routes.rs`, `vault_routes.rs`, `reference_routes.rs`, `entry_routes.rs`, `telemetry.rs`, `dm_routes.rs`, `invitations.js` | 1 152–2 063 each | route + storage + serialization + validation in one file each |
| `rfq-gateway` | `src/strategies/selic_conviction.rs` | 1 521 | pricing math + budget + allowlists + runtime mutation + JSONL ledger I/O |
| `rfq-gateway` (also) | `routes/admin.rs` | 1 099 | 17 unrelated admin handlers |
| `quilombo-blog` | `hooks.server.ts` | small but critical | top-level async IIFE runs 4 migrations + swallows errors |
| `ArteLonga` | `contato/index.html` | 484 | inline critical CSS + 120 lines inline `<script>` |
| `yggdrasil` | — | — | the SRP problem here is the OPPOSITE: per-game logic NOT yet split into a trait |
| `comunicacao` | `mbya/` | 4 619 files | checked-in mirror of the Arandu repo with no manifest governance |

**Recommendation:** the megafile fix is mechanically the same in every repo — extract by concern, not by line count. The agents propose `CO-215`, `RFQ-14`, and `AL-52` as the entry points; they're independent and parallelizable.

### 2. State coupling — three flavors, same disease

- **Rust apps (`co`, `rfq-gateway`)**: god `AppState` structs. CO has 18 fields, rfq has 24. Every handler takes `State<AppState>` and most reach into 2–4 fields. Storage↔server import cycle in CO blocks lifting storage into its own crate.
- **Node app (`quilombo-blog`)**: 7 `lib/server/` modules each open their own `new Database(DATABASE_PATH)` singleton. No shared accessor. Centralized instrumentation impossible.
- **Browser app (`ArteLonga`)**: 5 `al_*` localStorage keys scattered across files; no registry; one past migration already lost data.

**Recommendation:** introduce explicit dependency injection at the seam — the user's "autoload singleton" / "abstract services for DI" requirement. CO-218 (state segregation via role-bundles) + RFQ-18 (24-field AppState → 6 role-bundles) + QB-3 (centralize `db()` access) + AL-53 (storage key registry) are the four parallel tasks.

### 3. Static typing gaps

| Repo | Symptom | Count / examples |
|---|---|---|
| `co` | `serde_json::Value` as opaque handler payload | 180 occurrences across handlers |
| `co` (SPA) | Vanilla JS, ~7 000 lines | Whole SPA needs TS migration; existing `docs/architecture-review.md` covers this |
| `rfq-gateway` | typed protos exist, but trait-object soup at strategy layer | scoped to strategy chain |
| `quilombo-blog` | TypeScript but no zod on JSON parse boundaries | every `+server.ts` body parse is unchecked |
| `quilombo-blog` | `$page.data` missing types | LoadEvent return types unspecified in many routes |
| `ArteLonga` | `contato/index.html` 120-line inline JS | bypasses the TS opt-in in `src/` |

**Recommendation:** static typing is the principle most likely to ROT silently. Propose adding a CI gate per repo that fails on:
- Rust: `clippy::dyn_any_implementations` + a custom lint scanning for `serde_json::Value` in handler signatures (allow-list rather than greenlight).
- TS: zod-on-parse rule (zod inferred types on every fetch + every body parse).
- Browser: no inline `<script>` over N lines.

### 4. API catalog drift — every repo, every time

- `co`: 200+ routes; partial OpenAPI at `/api/v1/interactions/openapi.json` only covers the four entry-CRUD primitives.
- `rfq-gateway`: has `docs/api-catalog.yaml` but `last_reviewed: 2026-05-09`; missing `GET /admin/intel/tab`, `POST /admin/budget/override`, `GET /admin/metrics/latency`.
- `quilombo-blog`: **no `openapi.yaml` exists**, despite a `+server.ts` per public route.
- `ArteLonga`: no inbound API; the consumer-side catalog (calls into co) was missing entirely until this audit.
- `yggdrasil`: no spec.
- `comunicacao`: there's no HTTP API; the "API" is the query surface CO's entries endpoint exposes over this content — also undocumented.

**Recommendation:** make catalog generation a CI requirement. Three viable paths:
- (a) **registry-driven** — CO's `interactions.yaml` model works, copy it.
- (b) **route-scanning** — write a per-stack scanner (axum routes → OpenAPI; SvelteKit `+server.ts` → OpenAPI).
- (c) **proto-first** — define `.proto` per resource, codegen routes from there.

The agent reports suggest starting with **(b)** for SvelteKit (QB-1) and **(a)** for Rust apps.

### 5. Path-dependencies + cross-repo cycles

- **yggdrasil → co**: `Cargo.toml` has `path = "../co/game-core"`. Live debt; YG-17 targeted removal in 0.5.0, still open at 0.9.0. fly.toml works around with a parent-dir build-context hack.
- **co internal**: `storage.rs` imports `crate::server::AppState`. Module cycle. Blocks `storage` from ever becoming its own crate.
- **quilombo-blog → co (upcoming)**: `co-client.ts` doesn't exist yet despite docs claiming it. Will land when CO-214 deploys.
- **artelonga → co**: outbound to `/api/v1/leads`, `/api/v1/auth/onboard-with-email`, `/marketing_events`. Catalogued as consumer contract.

**Recommendation:** the path-dep on `co/game-core` is the highest-leverage cycle to break (YG-38 P0). The storage↔server cycle in CO is internal (CO-216) but blocks downstream refactors. Both should land before the deeper restructuring.

---

## The template that emerged

Every repo now has the same docs layout. Promote this as the standard for every CO-platform repo:

```
docs/
  architecture/
    as-is.md              ← C4 (Context + Containers + Components) + dep graph + cycles flagged
    api-catalog.md        ← every route/endpoint organized by resource + auth + purpose
    refactor-plan.md      ← gap analysis vs the 7 principles + proposed tasks (priority-ordered)
README.md
CLAUDE.md
CHANGELOG.md (Keep-a-Changelog)
```

For content universes (`comunicacao`), the same shape applies but the "API" section documents the QUERY surface (what entries exist, what frontmatter types, what relations) rather than HTTP routes.

**This is the "create a template for all universes and apply" deliverable.** Already applied in this audit pass.

---

## Master backlog — all proposed tasks, prioritized

Tasks are repo-local IDs reserved by each agent (next-available per the repo's task tracker). Priority is harmonized across repos so we can pick the highest-leverage P0s first.

| Pri | Repo | ID | Title | Blast radius |
|---|---|---|---|---|
| **P0** | `co` | CO-215 | Split `server.rs` into `server/{router, state, validation, uat_boot, seed_orchestrator}.rs` | ~1 570 LoC moved; unblocks all CO refactors |
| **P0** | `co` | CO-216 | Break `storage ↔ server` import cycle via `StorageContext` trait | ~50 LoC; enables storage-as-own-crate later |
| **P0** | `rfq-gateway` | RFQ-14 | Split `selic_conviction.rs` into a folder (pricing / budget / ledger / allowlist) | ~1 521 LoC moved; held until Hedix DNS unblocks |
| **P0** | `rfq-gateway` | RFQ-15 | Move hedix-incentive JSONL sink out of `strategies/` into `observability::persist` | ~100 LoC; same hold |
| **P0** | `yggdrasil` | YG-38 | Pin `game-core` to a git rev, delete `path` dep, drop `fly.toml` build-context hack | Cargo.toml + fly.toml + CI; closes YG-17 |
| **P0** | `comunicacao` | COM-1 | Reconcile universe identity (slug + root manifest + per-plane manifests) | governance |
| **P1** | `co` | CO-217 | Replace top-20 `serde_json::Value` handler payloads with typed `Deserialize` structs | feeds OpenAPI registry; reduces lints |
| **P1** | `co` | CO-218 | Decompose `AppStateInner` (18 fields) into role-bundles (DI seam) | enables storage-as-crate |
| **P1** | `rfq-gateway` | RFQ-18 | Decompose `AppState` (24 fields) → 6 role bundles | parallel with CO-218 |
| **P1** | `quilombo-blog` | QB-1 | Generate `openapi.yaml` from `+server.ts` catalog | enables typed clients |
| **P1** | `quilombo-blog` | QB-2 | Create `co-client.ts` (factory + `event.locals.co`) **before** CO-214 lands | blocks the exchange-session wire-up |
| **P1** | `quilombo-blog` | QB-3 | Centralize `db()` access; delete the 6 module-local singletons | unblocks transactional cross-domain writes |
| **P1** | `ArteLonga` | AL-51 | Emit `lead_submit` / `signup_verify_success` telemetry from forms | conversion signal |
| **P1** | `ArteLonga` | AL-52 | Extract `contato/index.html` inline CSS + JS into `pages.css` + `src/pages/contato.ts` | 484 LoC; needs visual regression test |
| **P1** | `ArteLonga` | AL-53 | Centralize all `al_*` storage key access; CI audit forbids hard-coded keys | prevents next migration bug |
| **P1** | `yggdrasil` | YG-39 | Promote `YggGame` to a trait; collapse per-game route boilerplate behind a generic | DRY + extension point |
| **P1** | `yggdrasil` | YG-41 | Introduce `tokio::sync::broadcast` event spine per `PokerTable` + WS upgrade route | replaces HTTP-poll multiplayer |
| **P1** | `comunicacao` | COM-3 | Remove or symlink the `mbya/` 4 619-file mirror | 100× search/embedding cost reduction |
| **P1** | `comunicacao` | COM-4 | Normalize `seed_status` enum + `language_code` requirement | data validation |
| **P2** | `comunicacao` | COM-5 | Extend CO's relation extractor to parse cross-plane wikilinks (`[[../yoruba/terms/iya.md]]`) | enriches relation graph |

20 tasks total. **6 are P0** and parallelizable across repos.

---

## How this maps to the seven principles you specified

| Principle | Tasks that move the needle |
|---|---|
| Composition over inheritance | YG-39 (`YggGame` trait), COM-5 (relation extractor as composable extractor) |
| Single responsibility | CO-215, RFQ-14, AL-52, COM-1 (god-file decomposition across all repos) |
| Static typing | CO-217 (`serde_json::Value` → structs), QB-1 (OpenAPI codegen target) |
| Reduced coupling | CO-216 (cycle break), YG-38 (path-dep removal), QB-3 (DB singleton consolidation) |
| Segregated state | CO-218 (`AppState` role-bundles), RFQ-18 (parallel), AL-53 (storage key registry) |
| Context-based graph (folders encapsulate features) | CO-215 (server/ folder), RFQ-14 (strategies/selic_conviction/ folder), AL-52 (src/pages/) |
| Event-driven | YG-41 (broadcast spine), QB-2 (`co-client.ts` should be event-driven for auth refresh) |

**Plus the cross-cutting:** the template now applied in every repo IS the "folders encapsulate features" principle made concrete at the docs layer.

---

## Suggested execution order (next 4 weeks)

The P0s split naturally into two waves so we don't stomp on each other:

**Wave 1 (parallel, week 1)** — file moves that unblock everything else:
- CO-215 (split `server.rs`)
- RFQ-14 + RFQ-15 (split `selic_conviction.rs` once Hedix DNS unblocks)
- YG-38 (close the `path = ../co/game-core` debt)
- COM-1 + COM-3 (universe identity + remove `mbya/` mirror)

**Wave 2 (parallel, week 2)** — coupling + typing on top of the freshly split files:
- CO-216 + CO-217 + CO-218
- RFQ-18
- QB-1 + QB-2 + QB-3
- AL-51 + AL-52 + AL-53

**Wave 3 (parallel, weeks 3–4)** — event-driven + composition:
- YG-39 + YG-41
- COM-4 + COM-5

Each wave is 4–6 parallel tasks. Wave 1 is purely structural; Wave 2 introduces the DI seam; Wave 3 makes things composable + reactive.

---

## What's NOT in scope (kept honest)

- **No code changed.** Every "task" is a planning artifact — co-auto picks them up next.
- **No commits, no PRs.** Per-agent files are staged for review in each repo.
- **The transaction-log → Kafka/Iceberg trajectory** (`co::public/transaction-log.md`) is its own backlog; not duplicated here.
- **The SPA architecture review** at `co::docs/architecture-review.md` covers the JS-side refactor; this audit references it but doesn't repeat the phases.
- **Migration scripts + tests** are mentioned in each refactor-plan; agents did NOT enumerate them at the level co-auto would need. Each task will need its own scaffolding pass when picked up.

---

## Next action

You can:
1. **Eyeball the per-repo `docs/architecture/refactor-plan.md` files** — each agent put detailed acceptance criteria there
2. **Pick which P0 tasks to launch first** — tell me, I'll generate the co-auto task files
3. **Commit the audit docs** — they're staged in each repo waiting for `git add docs/architecture/`
4. **Iterate on the template** — if `docs/architecture/` should be `docs/arch/` or `architecture/` at the repo root, change it now before it propagates

The audits stand alone. The synthesis is this document. The action is yours.
