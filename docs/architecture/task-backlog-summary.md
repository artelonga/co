# Task backlog — review summary (2026-05-18)

Grouped: **repo → scope → epic → user stories**. Each user story is a single PR with a conventional-commit shape and a semver implication. Epics group related stories — close when all stories are merged.

Six repos, **5 of them get co-auto orchestration**. Comunicacao is **reframed as a live CO universe** (not its own git-tracked repo) — see Section 7.

---

## Master rollup

| Repo | Epics | User stories | Conventional types | Net semver |
|---|---:|---:|---|---|
| co | 5 | 12 | refactor (8) · feat (2) · docs (2) | minor (2.9.0) |
| rfq-gateway | 3 | 10 | refactor (7) · feat (2) · chore (1) | minor (0.6.0) |
| quilombo-blog | 4 | 12 | feat (3) · refactor (6) · docs (2) · chore (1) | minor (0.6.0) |
| ArteLonga | 3 | 10 | feat (3) · refactor (5) · chore (2) | minor (0.3.0) |
| yggdrasil | 3 | 9 | refactor (5) · feat (2) · chore (2) | minor (1.0.0) |
| **comunicacao** | **1 (migration)** | **3** | **chore (3)** | n/a — universe, not semver-versioned |
| **Total** | **19** | **56** | | |

5 P0 user stories across the 5 code repos can launch in parallel as Wave 1; full sequencing in Section 8.

---

## 1. CO — co-web platform

**Theme:** decompose god-files, type the API, segregate state. Five epics, 12 stories.

### Epic CO-227: Server decomposition (`refactor(server):`)

The 1570-LoC `server.rs` god-file is THE unblocker — every downstream refactor lands cleanly only after it splits. Five stories, sequential within the epic.

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| CO-215 | Split `server.rs` into `server/{router, state, validation, uat_boot, seed_orchestrator}.rs` | `refactor(server)!:` | patch | ~1600 LoC | **P0** |
| CO-216 | Break `storage ↔ server` import cycle via `StorageContext` trait | `refactor(storage):` | patch | ~300 LoC | **P0** |
| CO-219 | Promote `chat_routes.rs` (2063 LoC) into `chat/` module folder | `refactor(chat):` | patch | ~2k LoC | P1 |
| CO-221 | Slim `AppStateInner` (18 fields) via segregated sub-states | `refactor(state):` | patch | ~400 LoC | P1 |
| CO-224 | Promote routes into context folders (`auth/`, `content/`, `social/`, `admin/`) | `refactor(routes):` | patch | meta-task | P2 |

**Epic acceptance:** no Rust file in `co-web/src/` exceeds 500 LoC; no top-level `*_routes.rs` outside their feature folder; `cargo test -p co-web` + `cargo clippy -- -D warnings` clean throughout.

### Epic CO-228: Type safety (`refactor(types):` + `chore(spa):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| CO-217 | Top-20 `serde_json::Value` handler payloads → typed `Deserialize` structs | `refactor(types):` | patch | scoped per site | P0 |
| CO-218 | Migrate SPA to TypeScript (incremental, file-by-file) | `chore(spa):` | patch | ~7k LoC progressive | P1 |

**Epic acceptance:** custom clippy lint flags `serde_json::Value` in handler signatures (allow-list); `co-web/static/variants/a/` has TS infrastructure + at least 5 modules migrated.

### Epic CO-229: Event-driven workers (`feat(events):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| CO-220 | Replace cross-feature direct calls with in-process event bus | `feat(events):` | **minor** | ~200 LoC | P1 |
| CO-223 | Define shared `Worker` trait + lifecycle (embedding + notification + push workers) | `refactor(workers):` | patch | ~300 LoC | P1 |

**Epic acceptance:** 3+ existing direct calls (`invitation → notification`, `proposal → notification`, `entry-write → embedding`) routed through the event bus; all workers implement `Worker` with start/stop/health.

### Epic CO-230: Auth unification (`refactor(auth):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| CO-222 | Unify auth gating into a single typed extractor | `refactor(auth):` | patch | ~150 LoC | P1 |

**Epic acceptance:** every protected handler uses `AuthenticatedUser` or `OwnerOnly` extractor; no in-handler `resolve_role()` calls remain.

### Epic CO-231: Documentation (`docs:`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| CO-225 | Document AppState composition pattern + `MODULES.md` | `docs:` | patch | docs-only | P2 |
| CO-226 | Add OpenAPI coverage for auth + admin + chat to interactions registry | `feat(openapi):` | **minor** | spec + tests | P2 |

**Epic acceptance:** `MODULES.md` lists every module + responsibility; `/api/v1/interactions/openapi.json` covers ≥80% of routes.

**Net CO bump:** **2.9.0** (two `feat:` stories — event bus, OpenAPI coverage).

---

## 2. RFQ-gateway — pricing engine

**Theme:** untangle the `selic_conviction` god-file + decompose AppState + event-drive the quote chain. Three epics, 10 stories. Held on Hedix DNS.

### Epic RFQ-24: Strategy decomposition (`refactor(strategies):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| RFQ-14 | Split `selic_conviction.rs` (1521 LoC) into folder (pricing/budget/ledger/allowlist) | `refactor(strategies)!:` | patch | ~1500 LoC | **P0** (held) |
| RFQ-15 | Move hedix-incentive JSONL sink out of `strategies/` → `observability::persist` | `refactor(observability):` | patch | ~100 LoC | **P0** (held) |
| RFQ-16 | Split `routes/admin.rs` (1099 LoC, 17 handlers) by surface | `refactor(admin):` | patch | ~1100 LoC | P1 |
| RFQ-21 | Consolidate SELIC feature into `src/features/selic/` | `refactor(selic):` | patch | meta-task | P2 |

**Epic acceptance:** no strategy file exceeds 500 LoC; strategies do NO disk I/O (all writes through `observability::persist`).

### Epic RFQ-25: State + builder normalization (`refactor(state):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| RFQ-17 | Replace `serde_json::Value` payloads with typed structs | `refactor(types):` | patch | scoped | P1 |
| RFQ-18 | Decompose `AppState` (24 fields) into 6 role-based bundles | `refactor(state):` | patch | ~400 LoC | P1 |
| RFQ-19 | Single `BuildArgs` struct instead of 4 builder arities | `refactor(build):` | patch | ~100 LoC | P1 |
| RFQ-22 | Pull `FairValueCache` out of `AppState` shared mutable | `refactor(cache):` | patch | ~150 LoC | P2 |
| RFQ-23 | Audit `Arc<RwLock<Option<T>>>` cells; replace with `OnceLock` where init-once | `chore(concurrency):` | patch | ~50 LoC | P2 |

### Epic RFQ-26: Event-driven quote chain (`feat(events):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| RFQ-20 | Event-driven inbound RFQ → quote → fill → reconciler chain | `feat(events):` | **minor** | ~500 LoC | P2 |

**Epic acceptance:** quote chain runs through a `RfqEvent` bus; reconciler subscribes; no direct `quote_engine.handle(req)` calls remain.

**Net RFQ bump:** **0.6.0** (one `feat:` story — event chain).

---

## 3. quilombo-blog — community site

**Theme:** type the boundaries, centralize DB access, generate OpenAPI, prep for CO-214 wire-up. Four epics, 12 stories.

### Epic QB-13: Contract + typing (`feat(api):` + `refactor(types):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| QB-1 | Generate `openapi.yaml` from route catalog | `feat(api):` | **minor** | gen + commit | **P0** |
| QB-2 | Carve `co-client.ts` and inject via `event.locals` | `feat(co-client):` | **minor** | new module | **P0** (blocks CO-214 wire-up) |
| QB-4 | Zod-validate every `JSON.parse` | `refactor(types):` | patch | every `+server.ts` | P1 |
| QB-6 | Type `App.PageData` per route group; eliminate `: any` in routes | `refactor(types):` | patch | type-only | P2 |

**Epic acceptance:** `openapi.yaml` exists + matches routes; `co-client.ts` is the only file calling co's API; zero `: any` in route handlers.

### Epic QB-14: Server-side state consolidation (`refactor(server):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| QB-3 | Centralize `db()` access; delete per-module singletons | `refactor(db)!:` | patch | ~7 modules | **P0** |
| QB-5 | Move boot-time migrations out of `hooks.server.ts` top-level IIFE | `refactor(boot):` | patch | ~100 LoC | P1 |
| QB-8 | Split `conteudo.ts` (735 LOC) by domain | `refactor(conteudo):` | patch | ~700 LoC | P2 |
| QB-9 | Adopt domain folders for `sync`, `fotos`, `videos` | `refactor(structure):` | patch | meta-task | P2 |

### Epic QB-15: Operational hygiene (`chore:` + `feat:`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| QB-7 | Disk-cache the on-demand Sharp resizes | `feat(perf):` | **minor** | ~80 LoC | P1 |
| QB-10 | Generalize op-log beyond `fotos` | `refactor(sync):` | patch | scoped | P2 |
| QB-11 | Centralize rate-limit middleware | `refactor(security):` | patch | ~100 LoC | P2 |
| QB-12 | Drop the unused root `Caddyfile` or wire it in | `chore:` | patch | 1 file | P3 |

### Epic QB-16: Documentation (`docs:`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| _shipped via audit_ | `docs/architecture/*` already produced | `docs(architecture):` | patch | 611 lines | done |

**Net QB bump:** **0.6.0** (three `feat:` stories).

---

## 4. ArteLonga — marketing site

**Theme:** complete the funnel telemetry + extract the contact-form monolith + centralize storage keys. Three epics, 10 stories.

### Epic AL-64: Funnel observability (`feat(telemetry):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| AL-51 | Emit `lead_submit` / `signup_verify_success` telemetry from forms | `feat(telemetry):` | **minor** | additive | **P0** |
| AL-57 | Backlink index + reverse-reference data | `feat(content):` | **minor** | data layer | P2 |

### Epic AL-65: Page-shell hygiene (`refactor(pages):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| AL-52 | Extract `contato/index.html` inline CSS + JS into `pages.css` + `src/pages/contato.ts` | `refactor(contato):` | patch | 484 LoC | P1 |
| AL-58 | Replace inline `<style>` blocks in `entrar/`, `faca-parte/` | `refactor(pages):` | patch | ~200 LoC | P2 |
| AL-54 | Split `assets/data.js` into per-collection modules | `refactor(assets):` | patch | ~300 LoC | P2 |
| AL-59 | Folder-level feature manifests | `chore(structure):` | patch | meta | P3 |
| AL-60 | Remove `dist/showcase.js` from the repo | `chore:` | patch | 1 file | P3 |

### Epic AL-66: Type + key centralization (`refactor(client):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| AL-53 | Centralize all `al_*` storage key access; CI audit forbids hard-coded keys | `refactor(storage)!:` | patch | every `al_*` site | P1 |
| AL-55 | OpenAPI codegen for `src/types.ts` | `feat(types):` | **minor** | gen + commit | P2 |
| AL-56 | Migrate `analytics.js` and `al-signup.js` to TypeScript | `refactor(ts):` | patch | ~600 LoC | P2 |

**Net AL bump:** **0.3.0** (three `feat:` stories).

---

## 5. yggdrasil — game lobby

**Theme:** close the path-dep on co/game-core, generalize the game adapter, add WS for multiplayer. Three epics, 9 stories.

### Epic YG-47: Cross-repo coupling (`chore(deps):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| YG-38 | Pin `game-core` to git rev + delete `path` dep + drop fly.toml hack | `chore(deps)!:` | patch | Cargo.toml + fly.toml | **P0** |

### Epic YG-48: Game adapter + multiplayer (`feat(games):` + `refactor(games):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| YG-39 | Promote `YggGame` to a trait used by all four games | `refactor(games)!:` | patch | per-game | P1 |
| YG-40 | Split `poker_routes.rs` (1189 LoC) by responsibility | `refactor(poker):` | patch | ~1200 LoC | P1 |
| YG-41 | Introduce `tokio::sync::broadcast` event spine + WS upgrade route | `feat(realtime):` | **minor** | ~400 LoC | P1 |
| YG-43 | Carve out `lobby/` folder; collapse `core::lobby` ↔ `web::lobby_routes` | `refactor(lobby):` | patch | ~300 LoC | P2 |

### Epic YG-49: State + types (`refactor:`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| YG-42 | Replace `serde_json::Value` in game state with concrete types | `refactor(types):` | patch | scoped | P2 |
| YG-44 | Segregate per-game DB connections behind a `ScoresStore` trait | `refactor(scores):` | patch | ~200 LoC | P2 |
| YG-45 | Trim `auth.rs` and `api/me.rs` (each >600 LOC) | `refactor(auth):` | patch | ~600 LoC | P2 |
| YG-46 | Document "no per-game DB" + correct the persistence model | `docs:` | patch | docs-only | P3 |

**Net YG bump:** **1.0.0** (the YG-17 path-dep closure earns 1.0.0; YG-41's WS feature is a minor on top).

---

## 6. comunicacao — content universe (REFRAMED)

**No longer a git repo.** This universe migrates to live inside CO as a first-class universe at `/data/universes/comunicacao` on co.artelonga.com.br. Every write goes through CO's vault PUT → `entry_events` append-only log → Iceberg-compatible event stream (per the trajectory in `co::public/transaction-log.md`).

**Why the reframe:** comunicacao is content, not code. The 9 COM-N tasks the audit proposed are content reorganization (schema normalization, dedup, identity reconciliation) — better done as a one-shot migration script + CO entry_events as the source-of-truth log, NOT as 9 separate co-auto runs.

### Epic COM-10: Migration to CO universe (`chore(migration):`)

| ID | Title | Conventional | Semver | Blast | P |
|---|---|---|---|---:|---|
| COM-1 | Create universe row + invitation + initial vault PUTs from `/Users/artelonga/projects/comunicacao/**/*.md` | `chore(migration):` | n/a | ~50 vault PUTs |  **P0** |
| COM-2 | Reconcile identity in flight (slug `comunicacao` vs `topologia` — pick one) + normalize per-plane manifests | `chore(migration):` | n/a | governance | **P0** |
| COM-3 | Remove or extract the `mbya/` 4 619-file mirror (CO-155 already wires Arandu as runtime facade) | `chore(migration):` | n/a | content | **P0** |

After migration, the remaining COM-4..9 tasks become entries IN the universe (frontmatter validation jobs, cross-plane wikilink extraction, derived views) — managed by CO's own task system, not the legacy git workflow.

**Iceberg trajectory:** once comunicacao lives in CO, every edit appends to `entry_events`. Phase 4/5 of the transaction-log roadmap (`co::public/transaction-log.md`) wires the Kafka export → Parquet → Iceberg catalog. Comunicacao's full edit history becomes time-travel-queryable via `SELECT … FOR TIMESTAMP AS OF …`.

**Net effect:** the comunicacao git-repo path quietly retires once content is in CO. Source backup remains via the daily DR snapshot (CO-143) of the prod volume.

---

## 7. Sequencing — recommended wave order

### Wave 1 (parallel across 5 code repos + comunicacao migration)

P0 stories that unblock the rest. Launch all 6 simultaneously:

| Repo | Story | Why first |
|---|---|---|
| co | **CO-215** split server.rs | unblocks all other CO refactors |
| rfq | **RFQ-17** typed payloads (RFQ-14/15 held on Hedix) | independent + low risk |
| qb | **QB-2** create `co-client.ts` | blocks CO-214 deploy wire-up |
| AL | **AL-51** funnel telemetry | additive, low risk |
| ygg | **YG-38** kill path dep | closes long-open YG-17 |
| comunicacao | **COM-1+2+3** universe migration | one-shot, not co-auto |

### Wave 2 (after Wave 1 PRs merge + main FF'd)

| Repo | Story |
|---|---|
| co | CO-216 (storage cycle) + CO-217 (typed handlers) |
| rfq | RFQ-18 (AppState bundles) + RFQ-19 (BuildArgs) |
| qb | QB-1 (openapi gen) + QB-3 (db centralization) |
| AL | AL-52 (extract contato/) + AL-53 (storage keys) |
| ygg | YG-39 (YggGame trait) + YG-40 (split poker) |

### Wave 3+ (everything else)

Lower-priority stories chained behind their wave-2 dependencies.

---

## 8. What co-auto needs to launch

For the 5 code repos: each needs a task spec file at `work/<space>/<TASK-ID>.md` matching the existing schema (yaml frontmatter: id, title, type, status, priority, labels, module, parent — plus body: ## As / ## I Need / ## So That / ## Context / ## Acceptance).

**Bootstrap gaps to close before Wave 1:**
- `quilombo-blog/work/qb/` — needs `project.yaml` + the 12 QB task specs
- `comunicacao` — NOT bootstrapped for co-auto; instead, COM-1+2+3 execute as an inline migration script run by me (see Section 6)
- The other 4 repos already have `work/<space>/` infrastructure; just need new task spec files added

**Plus:** each repo currently on a feature branch needs `git checkout main && git pull` before co-auto branches a worktree off HEAD.

---

## 9. Open questions for your review

1. **Epic IDs** — I used the next free numbers in each repo's existing scheme (CO-227..231, RFQ-24..26, etc.). Confirm or specify a different convention (e.g., `EPIC-CO-227` or `co-epic-1`).
2. **Conventional commit `!:` (breaking change) flags** — I marked structural splits (CO-215, RFQ-14, QB-3, AL-53, YG-38, YG-39) with `!:` since they change public module surfaces. None bump major except YG-49 (path-dep closure = release 1.0.0). Confirm or downgrade.
3. **Wave 1 picks** — CO-215 is the big swing (1600 LoC). Confirm or substitute a smaller smoke-test pick (e.g., CO-217 typed structs).
4. **Comunicacao reframe** — moves content out of git into a CO universe. Confirm or push back.
5. **Comunicacao migration approach** — I propose inline `mv` + vault PUTs done by me (not co-auto). Alternative: write a CLI under `co-cli` for `co universe import <path>` that does it generically. Worth doing if you'll re-use it for other universe imports.

---

## Next action

Approve the structure (or push back on the 5 open questions). I'll then:
1. Generate all 56 task spec files in their respective `work/<space>/` directories
2. Bootstrap `quilombo-blog/work/qb/`
3. Execute the comunicacao migration (3 sub-steps in Section 6)
4. Switch each code-repo to main + commit task specs + push
5. Launch 5 background co-auto runs

The spec generation is mechanical once the structure is approved — about 60 minutes of file-writing. The migration + launch add another 30 minutes. **Total ~90 minutes of setup before first co-auto session begins.**
