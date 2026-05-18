#!/usr/bin/env python3
"""Generate task specs for the architecture-refactor backlog across 5 repos.

Idempotent: rewrites files in-place. Run from anywhere.

Writes:
  /Users/artelonga/projects/rfq-gateway/work/rfq/RFQ-{14..26}.md          (13 files)
  /Users/artelonga/projects/quilombo-blog/work/qb/QB-{1..15}.md           (15 files + project.yaml + _universe.yaml)
  /Users/artelonga/projects/ArteLonga/work/artelonga/AL-{51..60,64..66}.md (13 files)
  /Users/artelonga/projects/yggdrasil/work/yggdrasil/YG-{38..49}.md       (12 files)

Updates project.yaml next_id per repo.
Fixes CO-{215..231} status: backlog -> todo.
"""
import os
import re
from pathlib import Path

CO  = Path("/Users/artelonga/projects/co/work/co")
RFQ = Path("/Users/artelonga/projects/rfq-gateway/work/rfq")
QB  = Path("/Users/artelonga/projects/quilombo-blog/work/qb")
AL  = Path("/Users/artelonga/projects/ArteLonga/work/artelonga")
YG  = Path("/Users/artelonga/projects/yggdrasil/work/yggdrasil")

CREATED = "2026-05-18T00:00:00Z"

def write(path: Path, content: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    print(f"  wrote {path}")

def labels_block(items):
    return "\n".join(f"  - {l}" for l in items)

def render_story(*, id, key, parent, title, commit, semver, priority, module,
                 labels, role, need, so_that, principles, scope, acceptance,
                 blast, status="todo", blocked_reason=None):
    fm_extra = ""
    if blocked_reason:
        fm_extra = f'\nblocked_reason: "{blocked_reason}"'
    return f"""---
id: {id}
title: "{title}"
type: user-story
status: {status}{fm_extra}
priority: {priority}
conventional_commit: "{commit}"
semver_bump: {semver}
labels:
{labels_block(labels)}
module: {module}
parent: {parent}
created_at: {CREATED}
updated_at: {CREATED}
---

## As

{role}

## I Need

{need}

## So That

{so_that}

## Context

- **Principles:** {principles}
- **Scope:** {scope}

## Acceptance

{acceptance}

## Blast radius

{blast}
"""

def render_epic(*, id, title, commit_default, semver, priority, module, labels,
                goal, children, acceptance):
    children_block = "\n".join(f"- {c}" for c in children)
    return f"""---
id: {id}
title: "{title}"
type: epic
status: todo
priority: {priority}
conventional_commit_default: "{commit_default}"
semver_bump_aggregate: {semver}
labels:
{labels_block(labels)}
module: {module}
created_at: {CREATED}
updated_at: {CREATED}
---

## Goal

{goal}

## Children

{children_block}

## Acceptance

{acceptance}
"""

def update_project_next_id(path: Path, new_id: int):
    if not path.exists():
        print(f"  project.yaml missing at {path}; skipping next_id update")
        return
    text = path.read_text()
    new_text = re.sub(r"^next_id:\s*\d+\s*$", f"next_id: {new_id}", text, flags=re.M)
    path.write_text(new_text)
    print(f"  updated {path} next_id -> {new_id}")

# ============================================================
# Fix CO status: backlog -> todo (CO-215..231)
# ============================================================
def fix_co_status():
    print("Fixing CO-215..231 status backlog -> todo")
    for n in range(215, 232):
        p = CO / f"CO-{n}.md"
        if not p.exists():
            continue
        t = p.read_text()
        t2 = t.replace("status: backlog\n", "status: todo\n", 1)
        if t != t2:
            p.write_text(t2)
            print(f"  patched {p.name}")

# ============================================================
# RFQ-gateway
# ============================================================
def gen_rfq():
    print("\n=== RFQ-gateway ===")
    write(RFQ / "RFQ-14.md", render_story(
        id=14, key="RFQ", parent=24,
        title="Split selic_conviction.rs (1521 LoC) into a feature folder",
        commit="refactor(strategies)!:", semver="patch",
        priority="high", status="blocked", blocked_reason="Hedix DNS — gate per CLAUDE.md prod canary handoff",
        module="rfq-gateway",
        labels=["type:refactor", "module:strategies"],
        role="A maintainer of rfq-gateway",
        need="`selic_conviction.rs` (1521 LoC) decomposed into `src/strategies/selic_conviction/` feature folder",
        so_that="The single biggest SRP violator becomes maintainable; the JSONL sink leak is contained inside the feature.",
        principles="§2 (SRP), §6 (folders encapsulate features)",
        scope=(
            "Convert `src/strategies/selic_conviction.rs` into `src/strategies/selic_conviction/` with at minimum:\n\n"
            "- `mod.rs` — re-exports + trait impl\n"
            "- `pricing.rs` — Gaussian + per-strike caps math\n"
            "- `budget.rs` — `ConvictionCaps`, incentive cap, runtime mutation API (the override entry-point)\n"
            "- `incentive_ledger.rs` — JSONL writes (currently mixed into the strategy file; the only place a strategy does I/O)\n"
            "- `allowlist.rs` — `mm_performance_allowlist` + `hedix_incentive_allowlist` types and gates"
        ),
        acceptance=(
            "- No file exceeds 400 LoC.\n"
            "- `cargo test -p rfq-gateway --lib selic_conviction` passes unchanged.\n"
            "- `cargo clippy -- -D warnings` clean.\n"
            "- Public API of `strategies::SelicConvictionStrategy` unchanged (callers in `main.rs` and `routes/admin.rs` compile untouched)."
        ),
        blast="Medium. Strategy is hot-path on every `SELIC-*` / `SMK*` RFQ. Touching it ahead of the prod canary inherits review risk. Land **after** the Hedix DNS canary unblocks (per `CLAUDE.md` handoff section).",
    ))

    write(RFQ / "RFQ-15.md", render_story(
        id=15, key="RFQ", parent=24,
        title="Move hedix-incentive JSONL sink out of strategies/",
        commit="refactor(observability):", semver="patch",
        priority="high", status="blocked", blocked_reason="Hedix DNS — paired with RFQ-14",
        module="rfq-gateway",
        labels=["type:refactor", "module:observability"],
        role="A maintainer of rfq-gateway",
        need="The `hedix_incentive-YYYY-MM-DD.jsonl` writes routed through `observability::persist` instead of inline in the strategy",
        so_that="Strategies become pure (no `std::fs` / `tokio::fs`); the only `strategies → observability::persist` dep-graph edge dissolves.",
        principles="§1 (composition), §4 (coupling), §6 (features)",
        scope=(
            "`selic_conviction.rs` writes its own `hedix_incentive-YYYY-MM-DD.jsonl` ring file. That is the only `strategies → "
            "interactions/observability::persist` edge in the dep graph and the reason `as-is.md §4` flags a side-effect leak. Either:\n\n"
            "- (a) push the writes through `observability::persist::spawn_persistence_task` on a new ring "
            "(`HedixIncentiveLedgerRing`), or\n"
            "- (b) accept it as an explicit `Arc<dyn IncentiveLedger>` injected at boot, mockable in tests."
        ),
        acceptance=(
            "- `strategies::*` no longer references `std::fs` or `tokio::fs` directly.\n"
            "- Existing JSONL file format unchanged (operators have tooling on it).\n"
            "- New trait + injection point covered by a unit test that swaps a `MemoryLedger`."
        ),
        blast="Low. Pure refactor with the format preserved.",
    ))

    write(RFQ / "RFQ-16.md", render_story(
        id=16, key="RFQ", parent=24,
        title="Split routes/admin.rs (1099 LoC, 17 handlers) by surface",
        commit="refactor(admin):", semver="patch",
        priority="medium", module="rfq-gateway",
        labels=["type:refactor", "module:admin"],
        role="A maintainer of rfq-gateway",
        need="`routes/admin.rs` (1099 LoC, 17 handlers) split into surface-aligned files under `routes/admin/`",
        so_that="Each admin surface (intel, observability, ops, json-shape) lives in a single-responsibility file under 300 LoC.",
        principles="§2 (SRP), §6 (features)",
        scope=(
            "Split into:\n\n"
            "- `routes/admin/mod.rs` (router assembly)\n"
            "- `routes/admin/intel.rs` — `intel_cache`, `intel_mapping`, `intel_inventory`, `intel_discovery`, "
            "`intel_signals`, `intel_history`, `intel_copom`, `intel_tab` (8 handlers)\n"
            "- `routes/admin/observability.rs` — `fills`, `rejections`, `inbound`, `interactions`, `latency_metrics` (5 handlers)\n"
            "- `routes/admin/ops.rs` — `reconciliation_report`, `run_e2e`, `budget_override`, `changelog` (4 handlers)\n"
            "- `routes/admin/json_shape.rs` — the ad-hoc `Vec<serde_json::Value>` shaping helpers (eliminate or move to typed "
            "`utoipa::ToSchema` structs — see RFQ-17)"
        ),
        acceptance="- No file >300 LoC.\n- `lib.rs` `Router::new()` lines stay identical (routes test verifies this).",
        blast="Low. Compile-time only.",
    ))

    write(RFQ / "RFQ-17.md", render_story(
        id=17, key="RFQ", parent=25,
        title="Replace serde_json::Value payloads with typed structs",
        commit="refactor(types):", semver="patch",
        priority="medium", module="rfq-gateway",
        labels=["type:refactor", "module:types"],
        role="A maintainer of rfq-gateway",
        need="Owned-output sites stop returning `serde_json::Value`; external/free-form sites keep it with documentation",
        so_that="OpenAPI coverage improves automatically; admin and inventory outputs gain compile-time guardrails.",
        principles="§3 (static typing)",
        scope=(
            "Audit shows `serde_json::Value` in:\n\n"
            "| Site                                     | Justified?                              | Action                                        |\n"
            "|------------------------------------------|-----------------------------------------|-----------------------------------------------|\n"
            "| `interactions/record.rs:78,85,98`        | Yes — captures opaque outbound bodies   | Keep; add doc comment.                        |\n"
            "| `hedix/client.rs:214,238,265,346,409,437,568` | Yes — Hedix error bodies are free-form | Keep; narrow `api_error()` to a typed `HedixErrorBody` where possible. |\n"
            "| `observability/inbound.rs:40` (`response_body: Value`) | Borderline — we own the response | Replace with `serde_json::value::RawValue` (cheaper) or with an enum over `QuoteResponse \\| ErrorResponse`. |\n"
            "| `strategies/selic_conviction.rs:257` (`inventory_snapshot -> Vec<Value>`) | No — internal type | Return `Vec<InventorySnapshotEntry>` (typed). |\n"
            "| `routes/admin.rs:604,607,910,993` (`Vec<Value>` shaping) | No — admin output | Define `utoipa::ToSchema`-derived structs; appears in OpenAPI for free. |"
        ),
        acceptance="- Zero `serde_json::Value` occurrences outside `interactions/`, `hedix/client.rs` error path, and the changelog parser.",
        blast="Low. Schema additions are additive on the wire.",
    ))

    write(RFQ / "RFQ-18.md", render_story(
        id=18, key="RFQ", parent=25,
        title="Decompose AppState (24 fields) into role-based bundles",
        commit="refactor(state):", semver="patch",
        priority="medium", module="rfq-gateway",
        labels=["type:refactor", "module:state"],
        role="A maintainer of rfq-gateway",
        need="`AppState`'s 24 fields grouped into 6 role-aligned sub-states",
        so_that="`build_app_full` arity drops; admin handlers gain semantic context; coupling between layers becomes inspectable.",
        principles="§4 (coupling), §5 (segregated state)",
        scope=(
            "`routes::quote::AppState` has 24 fields covering pricing strategy, hedix client, allowlists, deviation guard, "
            "intel handle, platform status, three observability rings, interactions ring, COPOM aggregator, conviction handle, "
            "env tier, prefix list. Group into:\n\n"
            "- `QuoteCore` — strategy, pricing, store, default_valid_for_seconds\n"
            "- `Gates` — ticker_allowlist, selic_allowlist, max_price_deviation_cents, conviction_prefix_list, platform_status (+ max_age)\n"
            "- `ObservabilityCtx` — fills, rejections, inbound, interactions\n"
            "- `IntelCtx` — intel handle, copom, selic_conviction handle\n"
            "- `HedixCtx` — hedix client, reconciliation_report\n"
            "- `Meta` — env_tier\n\n"
            "`AppState { core, gates, obs, intel, hedix, meta }` — 6 fields."
        ),
        acceptance=(
            "- All `state.<field>` reads in `routes/quote.rs` rewrite to `state.<group>.<field>`.\n"
            "- `build_app_full` arity drops from 9 to ≤4 (`config`, `bundles`, optional overrides)."
        ),
        blast="Medium. Touches every admin handler.",
    ))

    write(RFQ / "RFQ-19.md", render_story(
        id=19, key="RFQ", parent=25,
        title="Single BuildArgs struct instead of 4 builder arities",
        commit="refactor(build):", semver="patch",
        priority="low", module="rfq-gateway",
        labels=["type:refactor", "module:build"],
        role="A maintainer of rfq-gateway",
        need="5 `build_app*` variants (2–9 args each) collapsed into a single `build_app(BuildArgs)` API",
        so_that="Test setup becomes uniform; production main and tests share one entry point.",
        principles="§4 (coupling)",
        scope=(
            "`lib.rs` exposes `build_app`, `build_app_with_strategy`, `build_app_with`, `build_app_with_intel`, `build_app_full` "
            "(5 variants, 2–9 args each). Collapse to:\n\n"
            "```rust\n"
            "pub fn build_app(args: BuildArgs) -> Router;\n"
            "// where BuildArgs has only `config: &RfqConfig` required, rest Option<>\n"
            "```\n\n"
            "Existing test call sites get tiny `BuildArgs::for_test(config).with_strategy(s)` constructors."
        ),
        acceptance=(
            "- Every existing builder kept as `#[deprecated]` thin wrapper for one release, then removed.\n"
            "- `main.rs` `build_app_full(&config, strategy, hedix, …)` is now `build_app(args)`."
        ),
        blast="Low–Medium. Widespread but mechanical edits.",
    ))

    write(RFQ / "RFQ-20.md", render_story(
        id=20, key="RFQ", parent=26,
        title="Event-driven inbound RFQ → quote → fill → reconciler chain",
        commit="feat(events):", semver="minor",
        priority="low", module="rfq-gateway",
        labels=["type:feat", "module:events"],
        role="A maintainer of rfq-gateway",
        need="A `QuoteLifecycleEvent` broadcast spine that consumers subscribe to (rings, reconciler, strategy `record_fill`)",
        so_that="Reconciler latency drops to O(ms); the inline handler boilerplate shrinks; replay/observability gain a real spine.",
        principles="§7 (event-driven)",
        scope=(
            "Today: handler executes pricing inline, pushes to three rings inline, then returns. Reconciler runs on a 60s timer "
            "**independent** of fills. Proposed: introduce a `QuoteLifecycleEvent` enum (`Received`, `Validated`, `Priced{decision}`, "
            "`Accepted{quote}`, `Rejected{reason}`, `Filled{position_delta}`) emitted onto a tokio broadcast channel. Existing "
            "consumers (rings, reconciler, strategy `record_fill`) subscribe instead of being called directly.\n\n"
            "- Strategy `record_fill` becomes a subscriber, not an in-handler call.\n"
            "- Reconciler reacts to `Filled` events plus its own 60s tick instead of relying purely on the tick.\n"
            "- New `JsonlSinkSubscriber` replaces the explicit `inbound.push(); fills.push();` boilerplate in `routes::quote`."
        ),
        acceptance=(
            "- `routes::quote::create_quote` shrinks to ≤150 LoC (currently 250+).\n"
            "- Reconciler latency from fill → detection drops from O(60s) to O(ms) for the event path.\n"
            "- Backpressure: bounded broadcast channel (no `unbounded_channel`); drop-oldest with WARN log."
        ),
        blast="High. Touches hot path; alters fill-recording timing semantics. Defer until after RFQ-14/15/18; don't conflate.",
    ))

    write(RFQ / "RFQ-21.md", render_story(
        id=21, key="RFQ", parent=24,
        title="Consolidate SELIC feature into src/features/selic/",
        commit="refactor(selic):", semver="patch",
        priority="low", module="rfq-gateway",
        labels=["type:refactor", "module:selic"],
        role="A maintainer of rfq-gateway",
        need="SELIC concerns (spread across 7 places today) collected under `src/features/selic/`",
        so_that="The largest cross-cutting feature finally lives in one place; future SELIC changes touch one folder.",
        principles="§6 (folders encapsulate features)",
        scope=(
            "SELIC concerns live in 7 places today:\n\n"
            "- `strategies/operator_selic.rs`\n"
            "- `strategies/selic_conviction.rs`\n"
            "- `strategies/catalyst_guarded.rs`\n"
            "- `strategies/ipca_guarded.rs`\n"
            "- `strategies/prefix_dispatch.rs` (default prefix is `SELIC-`)\n"
            "- `intel/aggregator/copom.rs` + `intel/feeds/copom/*`\n"
            "- `docs/selic_strikes.yaml`\n\n"
            "Propose `src/features/selic/` with submodules `pricing/`, `catalyst/`, `dispatch/`, `aggregator/`, `feeds/`. Existing "
            "`strategies::QuoteStrategy` trait stays in `strategies/` (it's the seam, not the feature). PrefixDispatch stays "
            "generic; SELIC is just its largest client."
        ),
        acceptance="- `grep -r SELIC src/strategies` returns nothing; everything moves under `src/features/selic/`.",
        blast="Low (file moves + re-exports), but large diff. Best done in a quiet window.",
    ))

    write(RFQ / "RFQ-22.md", render_story(
        id=22, key="RFQ", parent=25,
        title="Pull FairValueCache out of AppState shared mutable",
        commit="refactor(cache):", semver="patch",
        priority="low", module="rfq-gateway",
        labels=["type:refactor", "module:cache"],
        role="A maintainer of rfq-gateway",
        need="`FairValueCache` split into reader/writer halves that strategies cannot accidentally write through",
        so_that="The cache is morally an *output* of the intel layer; the type system should reflect that.",
        principles="§5 (segregated state)",
        scope=(
            "`FairValueCache` is a `DashMap` shared across the intel runtime, every strategy that reads it, the COPOM aggregator "
            "(separate cache instance), and admin endpoints. The cache is morally an *output channel* of the intel layer, but it's "
            "plumbed as direct read/write. Introduce a reader/writer split:\n\n"
            "- `FairValueReader` (admin + strategies) — clone-friendly Arc, no write API exposed\n"
            "- `FairValueWriter` (intel runtime + COPOM aggregator only)"
        ),
        acceptance="- Any compile error if a `strategies::*` file tries to call `cache.put(...)`.",
        blast="Low. Type-system only.",
    ))

    write(RFQ / "RFQ-23.md", render_story(
        id=23, key="RFQ", parent=25,
        title="Replace RwLock<Option<T>> init-once cells with OnceLock",
        commit="chore(concurrency):", semver="patch",
        priority="low", module="rfq-gateway",
        labels=["type:chore", "module:concurrency"],
        role="A maintainer of rfq-gateway",
        need="`RwLock<Option<T>>` cells that are init-once-then-read replaced with `OnceLock`",
        so_that="The shape matches the semantics; eliminates the poison-on-init-panic footgun.",
        principles="§5 (segregated state)",
        scope=(
            "`main.rs` creates 3 `Arc<RwLock<Option<…>>>`: `conviction_cell`, `reconciliation_report`, and the implicit cell inside "
            "`build_app_with`. The first two are init-once-then-read; `RwLock<Option<T>>` is the wrong shape. Use "
            "`tokio::sync::OnceCell` or `std::sync::OnceLock`. The poison-under-mutex feedback from "
            "`~/.claude/.../feedback_no_panic_under_mutex.md` makes this a latent footgun — a panic during seeding would poison "
            "`conviction_cell` for the lifetime of the process."
        ),
        acceptance="- Zero `RwLock<Option<T>>` patterns where T is set exactly once.",
        blast="Very low.",
    ))

    # --- RFQ Epics ---
    write(RFQ / "RFQ-24.md", render_epic(
        id=24, title="Epic — Strategy decomposition",
        commit_default="refactor(strategies):", semver="patch",
        priority="high", module="rfq-gateway",
        labels=["type:refactor", "epic"],
        goal=("Untangle the `selic_conviction.rs` god-file (1521 LoC), pull the JSONL sink leak out of strategies, "
              "decompose `routes/admin.rs`, and finally collect SELIC into one feature folder. Strategies become pure compute "
              "with no disk I/O."),
        children=[
            "RFQ-14 — Split selic_conviction.rs into a feature folder",
            "RFQ-15 — Move hedix-incentive JSONL sink out of strategies/",
            "RFQ-16 — Split routes/admin.rs by surface",
            "RFQ-21 — Consolidate SELIC feature into src/features/selic/",
        ],
        acceptance="No strategy file exceeds 500 LoC; strategies do NO disk I/O (all writes through `observability::persist`).",
    ))

    write(RFQ / "RFQ-25.md", render_epic(
        id=25, title="Epic — State + builder normalization",
        commit_default="refactor(state):", semver="patch",
        priority="medium", module="rfq-gateway",
        labels=["type:refactor", "epic"],
        goal=("AppState's 24 fields collapse into 6 role bundles; the 5 `build_app*` variants collapse into one; "
              "FairValueCache gains a reader/writer split; init-once cells get the right concurrency primitive."),
        children=[
            "RFQ-17 — Replace serde_json::Value payloads with typed structs",
            "RFQ-18 — Decompose AppState into role-based bundles",
            "RFQ-19 — Single BuildArgs struct instead of 4 builder arities",
            "RFQ-22 — Pull FairValueCache out of AppState shared mutable",
            "RFQ-23 — Audit Arc<RwLock<Option<T>>>; replace with OnceLock where init-once",
        ],
        acceptance=("`AppState` has ≤6 fields; `build_app_full` is the only builder; no `RwLock<Option<T>>` for init-once cells; "
                    "`FairValueCache` exposes typed read-only handles to strategies."),
    ))

    write(RFQ / "RFQ-26.md", render_epic(
        id=26, title="Epic — Event-driven quote chain",
        commit_default="feat(events):", semver="minor",
        priority="low", module="rfq-gateway",
        labels=["type:feat", "epic"],
        goal=("Replace the request-response quote/fill/reconciler chain with a `QuoteLifecycleEvent` broadcast spine. "
              "Reconciler latency drops to O(ms); fill recording becomes a subscriber, not an inline call."),
        children=["RFQ-20 — Event-driven inbound RFQ → quote → fill → reconciler chain"],
        acceptance=("Quote chain runs through a `RfqEvent` bus; reconciler subscribes; no direct `quote_engine.handle(req)` "
                    "calls remain; bounded broadcast channel with drop-oldest backpressure."),
    ))

    update_project_next_id(RFQ / "project.yaml", 27)

# ============================================================
# Quilombo-blog (BOOTSTRAP)
# ============================================================
def gen_qb():
    print("\n=== quilombo-blog (bootstrap) ===")
    QB.mkdir(parents=True, exist_ok=True)

    write(QB / "project.yaml", """---
name: Quilombo Blog
key: QB
description: >-
  Community site for Quilombo Araucária — SvelteKit + SQLite, deployed on Fly.io.
  This space tracks architecture-refactor work driving the v0.5 → v0.6 cycle and
  ongoing operational hardening.
created_at: 2026-05-18T00:00:00Z
next_id: 16
---
""")

    write(QB / "_universe.yaml", """---
slug: qb
name: Quilombo Blog — Dev Board
description: >-
  Architecture-refactor backlog for the quilombo-blog SvelteKit app.
  Tracks epics (QB-13..16), user-stories (QB-1..12), and post-refactor tasks.
visibility: private
schema: schema.yaml
created_at: 2026-05-18T00:00:00Z
---
""")

    write(QB / "QB-1.md", render_story(
        id=1, key="QB", parent=13,
        title="Generate openapi.yaml from route catalog",
        commit="feat(api):", semver="minor",
        priority="high", module="web",
        labels=["type:feat", "module:api"],
        role="A maintainer of quilombo-blog",
        need="A `scripts/generate-openapi.ts` that walks `src/routes/**` and emits an OpenAPI 3.1 spec from `docs/architecture/api-catalog.md`",
        so_that="The catalog stops drifting; a typed `co-client.ts` (QB-2) and AL-55 codegen both consume the same spec.",
        principles="§3 (static typing), §4 (reduced coupling)",
        scope=("Add `scripts/generate-openapi.ts` that walks `src/routes/**` and emits an OpenAPI 3.1 spec, seeded from "
               "`docs/architecture/api-catalog.md`. Wire `npm run check` to fail if catalog drifts."),
        acceptance=(
            "- `npm run openapi:gen` produces `openapi.yaml` covering all 31 `+server.ts` endpoints + page POST actions.\n"
            "- Manifest matches catalog row-by-row (script asserts).\n"
            "- README references the spec."
        ),
        blast="Tiny — additive, no runtime change.",
    ))

    write(QB / "QB-2.md", render_story(
        id=2, key="QB", parent=13,
        title="Carve co-client.ts and inject via event.locals",
        commit="feat(co-client):", semver="minor",
        priority="high", module="web",
        labels=["type:feat", "module:co-client"],
        role="A maintainer of quilombo-blog",
        need="A typed `CoClient` interface with `exchangeSession`, `postWebhook`, `verifyJwt` injected via `event.locals.co`",
        so_that="CO-214 session exchange has a clean wire point; no third singleton pattern; JWT cache is per-client-instance.",
        principles="§4 (reduced coupling), §3 (static typing)",
        scope=("Create `src/lib/server/co-client.ts` with a `CoClient` interface exposing `exchangeSession(localSession): "
               "Promise<CoJwt>`, `postWebhook(event, payload)`, and `verifyJwt(token)`. Mount one instance per process in "
               "`hooks.server.ts`, attach to `event.locals.co`. Update `app.d.ts` to declare `co: CoClient | null` in `Locals`."),
        acceptance=(
            "- `co-client.ts` exports a factory `criarCoClient(env): CoClient` — no module-level singleton.\n"
            "- Bearer JWTs cached in-process with TTL (Map<usuario_id, {jwt, expira_em}>); cache is **on the client instance**, "
            "not a module global.\n"
            "- Type-tested: `event.locals.co?.postWebhook(...)` compiles in any `+page.server.ts`."
        ),
        blast="Small — new file + 2 line additions to hooks + `app.d.ts`.",
    ))

    write(QB / "QB-3.md", render_story(
        id=3, key="QB", parent=14,
        title="Centralize db() access; delete per-module singletons",
        commit="refactor(db)!:", semver="patch",
        priority="high", module="web",
        labels=["type:refactor", "module:db"],
        role="A maintainer of quilombo-blog",
        need="The 6 module-local `_db` singletons across `conteudo.ts`, `sync.ts`, etc. replaced with a single import from `db/index.ts`",
        so_that="One place to add instrumentation; one place to set WAL pragma; no risk of divergent boot orders.",
        principles="§5 (segregated state), §4 (coupling)",
        scope=("Promote `db/index.ts`'s connection to the single owner. Replace the 6 module-local `let _db: Database.Database "
               "| null = null` definitions (`conteudo.ts`, `sync.ts`, `migracao-fotos.ts`, `import-videos.ts`, `video.ts`, "
               "`radio.ts`) with `import { db } from './db'`. Same SQLite file handle, same WAL pragma, one place to add "
               "instrumentation."),
        acceptance=(
            "- `grep -n \"new Database(\" src/lib/server/` returns only `db/index.ts`.\n"
            "- All tests still pass; no behavioural change.\n"
            "- Migration test confirms WAL mode is set exactly once."
        ),
        blast=("Medium — 6 files touched but pattern is mechanical. Risk: boot-order changes if `db/index.ts` migrations now run "
               "on first sync access. Mitigation: keep boot IIFE."),
    ))

    write(QB / "QB-4.md", render_story(
        id=4, key="QB", parent=13,
        title="Zod-validate every JSON.parse",
        commit="refactor(types):", semver="patch",
        priority="high", module="web",
        labels=["type:refactor", "module:types"],
        role="A maintainer of quilombo-blog",
        need="The 10 raw `JSON.parse` sites replaced with zod-validated `safeParseJson(x, Schema)` calls",
        so_that="Bad input produces structured 400s instead of 500s downstream; schemas become inspectable artifacts.",
        principles="§3 (static typing)",
        scope=("Add `zod` (~12 KB gzipped) and replace the 10 raw `JSON.parse` sites with named schemas. New file "
               "`src/lib/server/schemas/` (one per domain: `sync.ts`, `videos.ts`, `conteudo.ts`)."),
        acceptance=(
            "- `JSON.parse(` count in `src/` is 0 (allow `safeParseJson(x, Schema)`).\n"
            "- Failing parse logs a structured warning with the route path and returns HTTP 400 (where applicable) instead of a "
            "500 from a downstream `.includes` on `undefined`."
        ),
        blast="Small — defensive change. Adds one dep.",
    ))

    write(QB / "QB-5.md", render_story(
        id=5, key="QB", parent=14,
        title="Move boot-time migrations out of hooks.server.ts top-level IIFE",
        commit="refactor(boot):", semver="patch",
        priority="medium", module="web",
        labels=["type:refactor", "module:boot"],
        role="A maintainer of quilombo-blog",
        need="Migrations triggered from an explicit `iniciar()` entrypoint with health-endpoint state, not a top-level async IIFE",
        so_that="A panicked migration becomes observable in `flyctl status`; boot order is explicit.",
        principles="§5 (segregated state)",
        scope=("Replace the async IIFE in `hooks.server.ts` (lines 18–28) with an explicit `await iniciar()` called from a small "
               "`src/server.ts` entrypoint the Dockerfile already runs, OR convert each migration to lazy on-first-need. Errors "
               "must propagate to a health-endpoint state, not `console.error`."),
        acceptance=(
            "- `hooks.server.ts` has zero top-level side effects.\n"
            "- `/api/versao` reports `boot_status: 'ok' | 'pending' | 'degraded'`.\n"
            "- Failed boot migration is observable in `flyctl status`."
        ),
        blast="Medium — startup ordering. Test on UAT first per repo convention.",
    ))

    write(QB / "QB-6.md", render_story(
        id=6, key="QB", parent=13,
        title="Type App.PageData per route group; eliminate : any in routes",
        commit="refactor(types):", semver="patch",
        priority="medium", module="web",
        labels=["type:refactor", "module:types"],
        role="A maintainer of quilombo-blog",
        need="Discriminated `App.PageData` unions per layout boundary; the 31 `: any` route occurrences trimmed to ≤5",
        so_that="`+page.svelte` files get typed `data` props; svelte-check warning count drops monotonically.",
        principles="§3 (static typing)",
        scope=("Define discriminated PageData unions in `src/app.d.ts` per layout boundary (`/`, `/admin`, `/sync`). Audit the "
               "31 `: any` / `as any` occurrences in `src/routes/`; replace with generated types from QB-1's OpenAPI or "
               "hand-rolled interfaces."),
        acceptance=(
            "- `grep -E ': any|as any' src/routes/` returns ≤ 5 (explicit `// eslint:any` allowed for legitimate dynamic JSON "
            "post-zod).\n"
            "- `npx svelte-check` warning count strictly decreases."
        ),
        blast="Small — purely additive types.",
    ))

    write(QB / "QB-7.md", render_story(
        id=7, key="QB", parent=15,
        title="Disk-cache the on-demand Sharp resizes",
        commit="feat(perf):", semver="minor",
        priority="medium", module="web",
        labels=["type:feat", "module:perf"],
        role="A maintainer of quilombo-blog",
        need="`/api/fotos/[size]/[arquivo]` cache results to `${UPLOAD_DIR}/cache/${size}/${arquivo}` with ETag/304",
        so_that="Repeated photo views don't re-run Sharp; cache invalidates on source mtime change.",
        principles="§4 (reduced coupling)",
        scope=("`/api/fotos/[size]/[arquivo]` (in `imagens.ts`) currently runs Sharp per request. Cache result to "
               "`${UPLOAD_DIR}/cache/${size}/${arquivo}`, ETag/304 on subsequent reads."),
        acceptance=(
            "- Repeated GET of same `(size, arquivo)` reads cache file, not Sharp.\n"
            "- Cache is invalidated when source mtime changes.\n"
            "- New e2e test in `tests/` asserts the second request is < 50 ms."
        ),
        blast="Small — single endpoint.",
    ))

    write(QB / "QB-8.md", render_story(
        id=8, key="QB", parent=14,
        title="Split conteudo.ts (735 LOC) by domain",
        commit="refactor(conteudo):", semver="patch",
        priority="medium", module="web",
        labels=["type:refactor", "module:conteudo"],
        role="A maintainer of quilombo-blog",
        need="`conteudo.ts` carved into `conteudo/{posts,paginas,parser,index}.ts` per Principle 6",
        so_that="Each domain becomes ≤250 LOC; barrel re-export keeps imports stable.",
        principles="§2 (SRP), §6 (folders encapsulate features)",
        scope=("Single Responsibility violation. Carve into:\n\n"
               "- `conteudo/posts.ts` (`listarPosts`, `lerPost`, `salvarPost`, `excluirPost`)\n"
               "- `conteudo/paginas.ts` (`listarPaginas`, `lerPagina`)\n"
               "- `conteudo/parser.ts` (frontmatter normalization, photo-from-md extraction)\n"
               "- `conteudo/index.ts` (barrel re-export for back-compat)"),
        acceptance=(
            "- No file > 250 LOC.\n"
            "- Public API unchanged (`import { listarPosts } from '$lib/server/conteudo'` still resolves via barrel).\n"
            "- All tests pass without modification."
        ),
        blast="Small — mechanical, behavior-preserving.",
    ))

    write(QB / "QB-9.md", render_story(
        id=9, key="QB", parent=14,
        title="Adopt domain folders for sync, fotos, videos",
        commit="refactor(structure):", semver="patch",
        priority="medium", module="web",
        labels=["type:refactor", "module:structure"],
        role="A maintainer of quilombo-blog",
        need="`sync.ts` (584 LOC), `migracao-fotos.ts`, `import-videos.ts` etc. moved into per-domain folders",
        so_that="Each domain folder ≤4 files; per-folder `index.ts` defines the public surface.",
        principles="§6 (folders encapsulate features), §2 (SRP)",
        scope=("Same shape as QB-8. Split `sync.ts` (584 LOC) into `sync/hlc.ts`, `sync/ops.ts`, `sync/reducer.ts`, "
               "`sync/merge.ts`, `sync/blobs.ts`. Split `migracao-fotos.ts` + parts of `imagens.ts` into a `fotos/` folder. Same "
               "for `videos/`."),
        acceptance=(
            "- `lib/server/{fotos,videos,sync,conteudo}/` exist, each ≤ 4 files.\n"
            "- Per-folder `index.ts` defines the public surface."
        ),
        blast="Medium — many imports update, no behaviour change.",
    ))

    write(QB / "QB-10.md", render_story(
        id=10, key="QB", parent=15,
        title="Generalize op-log beyond fotos",
        commit="refactor(sync):", semver="patch",
        priority="low", module="web",
        labels=["type:refactor", "module:sync"],
        role="A maintainer of quilombo-blog",
        need="The op-log spine extended from fotos-only to `encontros`, `publicacoes`, `comentarios`",
        so_that="Two nodes sync the entire content graph, not just photos; CO can subscribe to change events post-CO-214.",
        principles="§7 (event-driven)",
        scope=("Today only foto upload/delete emits ops. Expand emitters to `encontros` (slug, titulo, data changes), "
               "`publicacoes` (post create/update/delete), and `comentarios`. Use the already-built reducer + 3-way merge so "
               "two nodes can sync the entire content graph, not just photos."),
        acceptance=(
            "- Migration v007 adds `entidade_tipo` index over `operacoes`.\n"
            "- `/sync/api/proposta` accepts ops for all four entity types.\n"
            "- Existing direct `db.update(eventos)...run()` calls in `/admin/encontros/+page.server.ts` route through "
            "`emitOp('encontro.update')`.\n"
            "- UAT-to-prod sync round-trip preserves a new event end-to-end."
        ),
        blast=("Large — touches every admin write path. Land **after** QB-2 (so CO can subscribe to ops), QB-3 (so emit + write "
               "share one tx), QB-9 (so the sync module is clean)."),
    ))

    write(QB / "QB-11.md", render_story(
        id=11, key="QB", parent=15,
        title="Centralize rate-limit middleware",
        commit="refactor(security):", semver="patch",
        priority="low", module="web",
        labels=["type:refactor", "module:security"],
        role="A maintainer of quilombo-blog",
        need="The `hashIP`-keyed rate limiter inlined in `/api/comentarios/+server.ts` extracted to `src/lib/server/rate-limit.ts`",
        so_that="A single implementation covers `comentarios`, `midia/track`, `/contato`; behavior is uniform.",
        principles="§2 (SRP), §4 (coupling)",
        scope=("Move the `hashIP`-keyed rate limit currently inlined in `/api/comentarios/+server.ts` into "
               "`src/lib/server/rate-limit.ts` with `limitarPorIp(event, { capacidade, janelaMs })`. Apply to "
               "`/api/midia/track` and `/contato`."),
        acceptance=(
            "- Single implementation; two more endpoints adopt it.\n"
            "- Tested: 11th comment in an hour returns 429."
        ),
        blast="Small.",
    ))

    write(QB / "QB-12.md", render_story(
        id=12, key="QB", parent=15,
        title="Drop the unused root Caddyfile or wire it in",
        commit="chore:", semver="patch",
        priority="low", module="web",
        labels=["type:chore"],
        role="A maintainer of quilombo-blog",
        need="`Caddyfile` either deleted or moved under `infra/` with a real docker-compose wiring",
        so_that="No stale config; README documents reality (Fly-only TLS).",
        principles="§6 (folders encapsulate features)",
        scope=("`Caddyfile` references `quilombo:3000` (a Docker Compose service name) but production uses Fly's "
               "`[http_service]` directly. Either delete it to remove confusion or move it under `infra/` with a "
               "`docker-compose.yml` that actually uses it."),
        acceptance=(
            "- README documents reality (Fly-only TLS).\n"
            "- No file references a non-existent service."
        ),
        blast="Tiny.",
    ))

    # --- QB Epics ---
    write(QB / "QB-13.md", render_epic(
        id=13, title="Epic — Contract + typing",
        commit_default="feat(api):", semver="minor",
        priority="high", module="web",
        labels=["type:feat", "epic"],
        goal=("Establish OpenAPI as the cross-system contract, type the boundaries, and carve `co-client.ts` so CO-214's "
              "session-exchange wire-up lands clean. After this epic, every `JSON.parse` flows through zod and every page-data "
              "shape lives in `App.PageData`."),
        children=[
            "QB-1 — Generate openapi.yaml from route catalog",
            "QB-2 — Carve co-client.ts and inject via event.locals",
            "QB-4 — Zod-validate every JSON.parse",
            "QB-6 — Type App.PageData per route group",
        ],
        acceptance=("`openapi.yaml` exists and matches routes row-by-row; `co-client.ts` is the only file calling CO's API; "
                    "zero `: any` in route handlers (allow-listed exceptions ≤5); zero raw `JSON.parse` in `src/`."),
    ))

    write(QB / "QB-14.md", render_epic(
        id=14, title="Epic — Server-side state consolidation",
        commit_default="refactor(server):", semver="patch",
        priority="high", module="web",
        labels=["type:refactor", "epic"],
        goal=("Eliminate the 6 per-module DB singletons, move boot-time migrations out of the top-level IIFE, and split the "
              "two largest server files (`conteudo.ts`, `sync.ts`) into domain folders. State becomes inspectable."),
        children=[
            "QB-3 — Centralize db() access; delete per-module singletons",
            "QB-5 — Move boot-time migrations out of hooks.server.ts top-level IIFE",
            "QB-8 — Split conteudo.ts (735 LOC) by domain",
            "QB-9 — Adopt domain folders for sync, fotos, videos",
        ],
        acceptance=("`grep new Database( src/lib/server/` returns only `db/index.ts`; `hooks.server.ts` has zero top-level "
                    "side-effects; `/api/versao` reports `boot_status`; no server file exceeds 300 LOC."),
    ))

    write(QB / "QB-15.md", render_epic(
        id=15, title="Epic — Operational hygiene",
        commit_default="chore:", semver="minor",
        priority="medium", module="web",
        labels=["type:chore", "epic"],
        goal=("Add a Sharp disk cache for repeated photo views, generalize the op-log beyond fotos so CO can subscribe to "
              "all content changes, centralize the rate-limit middleware, and clean up the stale Caddyfile."),
        children=[
            "QB-7 — Disk-cache the on-demand Sharp resizes",
            "QB-10 — Generalize op-log beyond fotos",
            "QB-11 — Centralize rate-limit middleware",
            "QB-12 — Drop the unused root Caddyfile or wire it in",
        ],
        acceptance=("Sharp resizes cached on disk with ETag/304; op-log emits for all four entity types; one rate-limit "
                    "implementation covers three endpoints; Caddyfile either deleted or moved under `infra/`."),
    ))

# ============================================================
# ArteLonga
# ============================================================
def gen_al():
    print("\n=== ArteLonga ===")

    write(AL / "AL-51.md", render_story(
        id=51, key="AL", parent=64,
        title="Emit funnel events for lead + signup",
        commit="feat(telemetry):", semver="minor",
        priority="high", module="site",
        labels=["type:feat", "module:telemetry"],
        role="A maintainer of artelonga.com.br",
        need="The contact form and signup flow emit `lead_submit`, `signup_request`, `signup_verify_success`, `signup_verify_failed`, `signup_google_start`, `lead_submit_failed` to `window.AL_track`",
        so_that="Funnel attribution becomes computable; lead conversion rate and signup completion rate appear in `co`'s telemetry/events table.",
        principles="§7 (event-driven where signals matter)",
        scope=("In `contato/index.html` inline script and `assets/al-signup.js`, call `window.AL_track('lead_submit', {servico, "
               "parceiro, channel})` on successful POST, and `lead_submit_failed` on the catch branch. In `al-signup.js`, emit "
               "`signup_request`, `signup_verify_success`, `signup_verify_failed`, `signup_google_start`."),
        acceptance=(
            "- Each of the 6 events emitted exactly once per user action.\n"
            "- Events appear in `co`'s `telemetry/events` table.\n"
            "- No-op when `AL_track` is undefined (analytics opt-out / DNT).\n"
            "- Playwright test asserts `window.AL_track` was called with expected name+props."
        ),
        blast="Low. Add-only. Both forms have fallback paths that already swallow errors.",
    ))

    write(AL / "AL-52.md", render_story(
        id=52, key="AL", parent=65,
        title="Extract contato/index.html script + critical CSS into shared files",
        commit="refactor(contato):", semver="patch",
        priority="medium", module="site",
        labels=["type:refactor", "module:contato"],
        role="A maintainer of artelonga.com.br",
        need="The 250-line inlined CSS and 120-line inline script in `contato/index.html` extracted to `pages.css` + `src/pages/contato.ts`",
        so_that="The last page-monolith disappears; the only critical JS that isn't TS becomes TS.",
        principles="§2 (SRP), §3 (static typing)",
        scope=("Move the 250-line inlined CSS in `contato/index.html` into `assets/pages.css` under a `.contato-*` namespace. "
               "Move the 120-line inline `<script>` into `src/pages/contato.ts` and wire via `data-page=\"contato\"` in the "
               "dispatcher. Keep one inlined fallback rule to prevent the documented CLS, but trim from 250 lines to ~15 (just "
               "`body { padding-bottom }` and `.site-footer { position: fixed }`)."),
        acceptance=(
            "- `/contato/` renders with identical layout (visual regression via Playwright screenshot).\n"
            "- No CLS regression in Lighthouse CI (CLS < 0.1 holds).\n"
            "- File `contato/index.html` ≤ 30 lines.\n"
            "- Inline script removed; logic lives in `src/pages/contato.ts`, typed against `Lead` interface in `src/types.ts`.\n"
            "- Form submission emits AL-51 events."
        ),
        blast="Medium — contact form is a revenue surface. Must screenshot-test.",
    ))

    write(AL / "AL-53.md", render_story(
        id=53, key="AL", parent=66,
        title="Centralize storage keys and namespacing",
        commit="refactor(storage)!:", semver="patch",
        priority="medium", module="site",
        labels=["type:refactor", "module:storage"],
        role="A maintainer of artelonga.com.br",
        need="One TS module owning every `al_*` key (`vid`, `sid`, `optOut`, `utm`, `eventQueue`); CI audit forbids hard-coded keys outside that file and the analytics constants block",
        so_that="Adding a new storage key becomes a one-place edit; key drifts are detected at PR time.",
        principles="§5 (segregated state)",
        scope=("Create `src/lib/storage.ts` exporting typed wrappers `vid()`, `sid()`, `optOut()`, `utm()`, `eventQueue()`, "
               "etc. — one module owns all `al_*` keys. `analytics.js` keeps its own copy (can't import TS from vanilla IIFE) "
               "but the key list lives in a `src/lib/storage-keys.ts` const referenced by build-time test."),
        acceptance=(
            "- All reads/writes of `al_vid`/`al_sid`/`al_optout`/`al_evq_v1`/`al_utm` go through one of: (a) the new "
            "`storage.ts` (from TS), or (b) the constants block in `analytics.js`.\n"
            "- Audit script `tools/audit-storage-keys.mjs` greps for hard-coded keys outside those two files; fails CI if any.\n"
            "- No behavioral change."
        ),
        blast="Low. Pure refactor.",
    ))

    write(AL / "AL-54.md", render_story(
        id=54, key="AL", parent=65,
        title="Split assets/data.js into per-collection modules",
        commit="refactor(assets):", semver="patch",
        priority="low", module="site",
        labels=["type:refactor", "module:assets"],
        role="A maintainer of artelonga.com.br",
        need="`data.js` (3372 LOC) split into 8 per-collection files (`data.people.js`, etc.); bootstrap injects only what the page needs",
        so_that="PR review becomes possible; per-page bundles shrink; lazy-loading on profile pages becomes available.",
        principles="§2 (SRP), §4 (reduced coupling)",
        scope=("Today `data.js` exports `window.AL` containing 8+ collections. Split into `data.people.js`, "
               "`data.communities.js`, `data.services.js`, `data.missions.js`, `data.solutions.js`, `data.finances.js`, "
               "`data.portfolio.js`, `data.popularity.js` — `bootstrap.js` injects only the ones the current `data-page` needs "
               "(or all of them, with HTTP/2 multiplexing they're free). Pages get smaller bundles."),
        acceptance=(
            "- `window.AL` API surface unchanged (combined at runtime from modules).\n"
            "- Pages load only what they read (per matrix in `as-is.md`).\n"
            "- Total transfer for `/` drops below previous baseline (Lighthouse byte budget verifies).\n"
            "- All bake scripts updated to write to per-collection files."
        ),
        blast="Medium — touches every bake script + bootstrap. Mitigation: keep the combined `data.js` as a transitional shim that re-exports from the new files.",
    ))

    write(AL / "AL-55.md", render_story(
        id=55, key="AL", parent=66,
        title="OpenAPI codegen for src/types.ts",
        commit="feat(types):", semver="minor",
        priority="low", module="site",
        labels=["type:feat", "module:types"],
        role="A maintainer of artelonga.com.br",
        need="`src/types.gen.ts` generated from `openapi/artelonga.yaml` by `openapi-typescript`",
        so_that="A known drift surface closes; CLAUDE.md's TODO note for codegen is resolved.",
        principles="§3 (static typing), §4 (reduced coupling)",
        scope=("Add `openapi-typescript` devDep. `npm run gen-types` produces `src/types.gen.ts`. Replace hand-mirrored "
               "`src/types.ts` with a re-export from the generated file (or delete entirely, importing from `src/types.gen.ts`)."),
        acceptance=(
            "- `tsc --noEmit` passes against generated types.\n"
            "- Hand-written `src/types.ts` removed or reduced to UI-only types not in the OpenAPI.\n"
            "- `npm run gen-types` is part of `npm run bake`.\n"
            "- Pre-commit hook checks `types.gen.ts` is in sync with `openapi/artelonga.yaml`."
        ),
        blast="Low — types only, no runtime change.",
    ))

    write(AL / "AL-56.md", render_story(
        id=56, key="AL", parent=66,
        title="Migrate analytics.js and al-signup.js to TypeScript",
        commit="refactor(ts):", semver="patch",
        priority="low", module="site",
        labels=["type:refactor", "module:ts"],
        role="A maintainer of artelonga.com.br",
        need="Both vanilla-JS runtime files moved into `src/runtime/` as TS with typed public APIs",
        so_that="The largest vanilla-JS files left after AL-22 disappear; AL-53 becomes trivial; pages get autocomplete on `window.AL_track`.",
        principles="§3 (static typing)",
        scope=("Move both files into `src/runtime/` as TS, compile to `assets/analytics.js` and `assets/al-signup.js` via the "
               "same Vite config (or a sibling config). Type the public APIs (`window.AL_track`, "
               "`window.AL_experiments.variant`, `window.AL_analytics.info`, etc.) into a `src/runtime/types.ts` that's also "
               "imported from page code."),
        acceptance=(
            "- Both files produced by build, behavior unchanged.\n"
            "- `window.AL_track` and `window.AL_analytics` typed in `src/types.ts` so pages get autocomplete.\n"
            "- All existing analytics test scenarios pass (DNT, opt-out, batching, beacon on pagehide)."
        ),
        blast="Medium — analytics is load-bearing for the entire telemetry pipeline. Mitigation: snapshot the compiled output before/after and diff for behavioral equivalence.",
    ))

    write(AL / "AL-57.md", render_story(
        id=57, key="AL", parent=64,
        title="Backlink index + reverse-reference data",
        commit="feat(content):", semver="minor",
        priority="low", module="site",
        labels=["type:feat", "module:content"],
        role="A maintainer of artelonga.com.br",
        need="A `tools/bake-backlinks.mjs` script producing `assets/backlinks.json` (reverse-reference graph for every handle)",
        so_that="Profile pages render \"Mencionado em\" backlinks; the bidirectional graph becomes inspectable.",
        principles="§4 (reduced coupling — making coupling explicit)",
        scope=("Today `audit-handles.mjs` walks forward references (service.responsavel, citacoes.autor, communities, "
               "parcerias, etc.). Add a `tools/bake-backlinks.mjs` that produces `assets/backlinks.json`: for each handle, "
               "list every entry that references it. Render a \"Mencionado em\" section on profile pages."),
        acceptance=(
            "- `backlinks.json` regenerated on every bake.\n"
            "- Profile pages render up to N backlinks under a collapsible.\n"
            "- Schema for backlink entry in OpenAPI."
        ),
        blast="Low. Add-only.",
    ))

    write(AL / "AL-58.md", render_story(
        id=58, key="AL", parent=65,
        title="Replace inline <style> blocks in entrar/ and faca-parte/",
        commit="refactor(pages):", semver="patch",
        priority="low", module="site",
        labels=["type:refactor", "module:pages"],
        role="A maintainer of artelonga.com.br",
        need="The smaller inline `<style>` blocks in `entrar/index.html` and `faca-parte/index.html` moved to `pages.css`",
        so_that="The page-shell ≤30-line convention applies uniformly.",
        principles="§2 (SRP)",
        scope=("Same treatment as AL-52 for the two other pages that ship inline styles. Inline blocks are smaller than "
               "`contato/`, so risk is lower."),
        acceptance="- Both shells ≤ 30 lines; styles moved to `pages.css` under `.entrar-*` and `.fp-*` namespaces.",
        blast="Low.",
    ))

    write(AL / "AL-59.md", render_story(
        id=59, key="AL", parent=65,
        title="Folder-level feature manifests",
        commit="chore(structure):", semver="patch",
        priority="low", module="site",
        labels=["type:chore", "module:structure"],
        role="A maintainer of artelonga.com.br",
        need="A `_feature.yaml` per cross-cutting folder (`servicos/`, `solucoes/`, `missoes/`) declaring schema + bake script + renderer",
        so_that="Onboarding cost drops; \"add a service\" instructions live next to the data.",
        principles="§6 (folders encapsulate features)",
        scope=("Add a `_feature.yaml` (optional) per cross-cutting folder (`servicos/`, `solucoes/`, `missoes/`) declaring: "
               "the OpenAPI schema this folder's entries conform to, the bake script that targets it, the page renderer that "
               "consumes it. Today this knowledge is implicit (spread across CLAUDE.md sections)."),
        acceptance=("- Three folder manifests; CLAUDE.md regenerated from them; new \"add a `<thing>`\" instructions "
                    "discoverable by reading `<folder>/_feature.yaml`."),
        blast="Trivial. Docs only.",
    ))

    write(AL / "AL-60.md", render_story(
        id=60, key="AL", parent=65,
        title="Remove dist/showcase.js from the repo",
        commit="chore:", semver="patch",
        priority="low", module="site",
        labels=["type:chore"],
        role="A maintainer of artelonga.com.br",
        need="`dist/showcase.js` (Vite build artifact) removed from version control; built on demand or omitted from prod",
        so_that="Deploy artifacts don't live next to source.",
        principles="§2 (SRP — deploy artifact vs source)",
        scope=("`dist/showcase.js` is a Vite build artifact used only by `/design/`. Move out of the deploy path or build on "
               "demand. Today it's committed and served."),
        acceptance="- `dist/` in `.gitignore`; `/design/` either omitted from production or built into `assets/` like `renderer.js`.",
        blast="Trivial.",
    ))

    # --- AL Epics ---
    write(AL / "AL-64.md", render_epic(
        id=64, title="Epic — Funnel observability",
        commit_default="feat(telemetry):", semver="minor",
        priority="high", module="site",
        labels=["type:feat", "epic"],
        goal=("Close the funnel attribution gap: lead-submit and signup events emit to telemetry, and reverse-reference "
              "backlinks make the content graph bidirectional. After this epic, conversion rate is computable end-to-end."),
        children=[
            "AL-51 — Emit funnel events for lead + signup",
            "AL-57 — Backlink index + reverse-reference data",
        ],
        acceptance=("6 funnel events appear in `co`'s `telemetry/events`; `backlinks.json` regenerated per bake; profile "
                    "pages render up to N backlinks."),
    ))

    write(AL / "AL-65.md", render_epic(
        id=65, title="Epic — Page-shell hygiene",
        commit_default="refactor(pages):", semver="patch",
        priority="medium", module="site",
        labels=["type:refactor", "epic"],
        goal=("Eliminate the page-monolith problem: extract `contato/`'s inline CSS+JS, do the same for `entrar/` and "
              "`faca-parte/`, split `data.js` (3372 LOC) into per-collection modules, add folder-level feature manifests, "
              "and remove the committed Vite artifact."),
        children=[
            "AL-52 — Extract contato/ inline CSS + JS",
            "AL-54 — Split assets/data.js into per-collection modules",
            "AL-58 — Replace inline <style> blocks in entrar/ and faca-parte/",
            "AL-59 — Folder-level feature manifests",
            "AL-60 — Remove dist/showcase.js from the repo",
        ],
        acceptance="Every page shell ≤30 lines; no inline `<style>` >15 lines; `data.js` carved into per-collection files; cross-cutting folders have manifests.",
    ))

    write(AL / "AL-66.md", render_epic(
        id=66, title="Epic — Type + key centralization",
        commit_default="refactor(client):", semver="patch",
        priority="medium", module="site",
        labels=["type:refactor", "epic"],
        goal=("Centralize storage-key access in one TS module (CI-enforced), migrate the last vanilla-JS runtime files to TS, "
              "and codegen `src/types.ts` from `openapi/artelonga.yaml`. After this epic, every storage key and every API "
              "type has a single source of truth."),
        children=[
            "AL-53 — Centralize storage keys and namespacing",
            "AL-55 — OpenAPI codegen for src/types.ts",
            "AL-56 — Migrate analytics.js and al-signup.js to TypeScript",
        ],
        acceptance=("`al_*` keys live in one module; `tools/audit-storage-keys.mjs` enforces it in CI; `src/types.gen.ts` "
                    "produced from OpenAPI; no vanilla-JS runtime files outside the build output."),
    ))

    update_project_next_id(AL / "project.yaml", 67)

# ============================================================
# yggdrasil
# ============================================================
def gen_yg():
    print("\n=== yggdrasil ===")

    write(YG / "YG-38.md", render_story(
        id=38, key="YG", parent=47,
        title="Pin game-core to git rev + delete path dep + drop fly.toml hack",
        commit="chore(deps)!:", semver="patch",
        priority="high", module="yggdrasil",
        labels=["type:chore", "module:deps"],
        role="A maintainer of yggdrasil",
        need="`Cargo.toml` switched from `path = \"../co/game-core\"` to `game-core = { git = ..., rev = ... }`; `fly.toml` parent-dir build trick removed",
        so_that="CI runs from a clean checkout without `co/` adjacent; long-open YG-17 closes.",
        principles="§4 (reduced coupling)",
        scope=("YG-17 already specifies the chore. Status today is `todo`, release `0.5.0`, but workspace is at `0.9.0` and "
               "still on the path dep. Either re-confirm YG-17 acceptance or supersede it."),
        acceptance=(
            "- `Cargo.toml` has `game-core = { git = \"https://github.com/artelonga/co\", rev = \"<sha>\" }`.\n"
            "- `fly.toml` build comment removed (no more parent-dir trick).\n"
            "- `docs/DEPENDENCIES.md` documents the bump policy.\n"
            "- CI green on a clean checkout with no `co/` clone next to the repo."
        ),
        blast=("Workspace-wide rebuild; surfaces every compile-time type assumption against `game-core`. Risk: drift between "
               "local `co/` and pinned rev causes confusing \"works locally, breaks in CI\" until policy is documented."),
    ))

    write(YG / "YG-39.md", render_story(
        id=39, key="YG", parent=48,
        title="Promote YggGame to a real adapter trait used by all four games",
        commit="refactor(games)!:", semver="patch",
        priority="medium", module="yggdrasil",
        labels=["type:refactor", "module:games"],
        role="A maintainer of yggdrasil",
        need="`YggGame` trait moved to its own module and implemented by snake, tetris, invaders, and poker; route boilerplate collapsed into `make_session_router::<G>()`",
        so_that="Each new game becomes a trait impl plus 30 LOC of route, not a full route module duplicate.",
        principles="§1 (composition), §5 (segregated state)",
        scope=("`YggGame` is declared in `yggdrasil-core/src/games/snake.rs` and re-exported, but only `YggSnake` implements it. "
               "`YggTetris`, `YggInvaders`, `YggPoker` each expose ad-hoc `tick`/`render_json`/`score` shapes, and the route "
               "layer special-cases each. Move `YggGame` to its own module (`yggdrasil-core/src/games/adapter.rs`), implement "
               "it for all four, then generalise `snake_routes`/`tetris_routes`/`invaders_routes` into a single "
               "`make_session_router::<G: YggGame>(…)`."),
        acceptance=(
            "- One `pub trait YggGame` in `yggdrasil-core::games::adapter`.\n"
            "- 4 impls (`YggSnake`, `YggTetris`, `YggInvaders`, `YggPoker` or its single-player sub-component).\n"
            "- `make_session_router::<G>()` exists; `snake_routes`, `tetris_routes`, `invaders_routes` collapse to ~30 LOC "
            "each (state struct + boot).\n"
            "- No behaviour change visible to clients (same JSON shapes, same routes)."
        ),
        blast="Single-player game routes only. Poker untouched unless we want it. Backwards-compatible at the HTTP layer.",
    ))

    write(YG / "YG-40.md", render_story(
        id=40, key="YG", parent=48,
        title="Split poker_routes.rs (1189 LOC) by responsibility",
        commit="refactor(poker):", semver="patch",
        priority="medium", module="yggdrasil",
        labels=["type:refactor", "module:poker"],
        role="A maintainer of yggdrasil",
        need="`poker_routes.rs` split into `poker/{state, routes, chip_flow, serialization, tests}.rs`; core also moves to a folder",
        so_that="The 1189-LOC monolith becomes inspectable; chip flow, lifecycle, and auth each get their own file.",
        principles="§2 (SRP), §6 (folders per feature)",
        scope=("Today `yggdrasil-web/src/games/poker_routes.rs` mixes: HTTP handlers, `PokerState` lifecycle (seeding, "
               "persistence boot), sementes credit/debit on sit/stand, hole-card auth, snapshot serialisation, and inline "
               "test app builders. Move to `yggdrasil-web/src/games/poker/` with files: `state.rs`, `routes.rs`, "
               "`chip_flow.rs` (sementes ↔ table), `serialization.rs`, `tests.rs`. Mirror with "
               "`yggdrasil-core/src/games/poker/` (already partially done — 5 files, but not a folder)."),
        acceptance=(
            "- `poker_routes.rs` ≤ 250 LOC after split.\n"
            "- `yggdrasil-core/src/games/poker.rs` becomes `poker/mod.rs` re-exporting the existing 5 sibling files.\n"
            "- Test coverage preserved (count `cargo test -p yggdrasil-web poker` before/after)."
        ),
        blast="Internal only (no public API or route change). Touches one big file plus a couple of imports.",
    ))

    write(YG / "YG-41.md", render_story(
        id=41, key="YG", parent=48,
        title="Introduce event spine (tokio::sync::broadcast) and WS for poker",
        commit="feat(realtime):", semver="minor",
        priority="medium", module="yggdrasil",
        labels=["type:feat", "module:realtime"],
        role="A maintainer of yggdrasil",
        need="Per-table `tokio::sync::broadcast::Sender<TableEvent>` plus `GET /api/v1/poker/lobbies/{id}/ws` upgrading to WebSocket",
        so_that="Poker stops being HTTP polling; the user-story brief's \"WebSocket session layer\" finally exists.",
        principles="§7 (event-driven), §2 (SRP), §5 (segregated state)",
        scope=("Poker today is HTTP polling (`GET .../hand` repeatedly). The user-story brief mentions a \"WebSocket session "
               "layer\" — it doesn't exist. Introduce per-table `tokio::sync::broadcast::Sender<TableEvent>` inside "
               "`PokerTable`; expose `GET /api/v1/poker/lobbies/{id}/ws` upgrading to WebSocket; emit "
               "`TableEvent::{Seated, HandStarted, ActionTaken, HandEnded}` from mutators. Existing HTTP poll endpoints stay "
               "(for backwards compat) but become thin reads of the same in-memory state."),
        acceptance=(
            "- `TableEvent` enum (statically typed; no `serde_json::Value`).\n"
            "- One `broadcast::Sender` per `PokerTable`, segregated state.\n"
            "- `/api/v1/poker/lobbies/{id}/ws` upgrades, streams JSON events.\n"
            "- One integration test: two clients sit at the same table, both receive a `HandStarted` event when blinds post.\n"
            "- Frontend `static/universos/poker.html` switched to WS subscribe (poll remains as fallback for v1)."
        ),
        blast=("Largest of the set. Adds runtime tasks per table, Tokio scheduling concerns, and surface area for race "
               "conditions in poker mutations. Should land after YG-40."),
    ))

    write(YG / "YG-42.md", render_story(
        id=42, key="YG", parent=49,
        title="Replace serde_json::Value in game state payloads with concrete types",
        commit="refactor(types):", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:refactor", "module:types"],
        role="A maintainer of yggdrasil",
        need="`StartResponse.state` and `TickResponse.state` typed via per-game `Serialize` structs; `map_to_value` deleted",
        so_that="Single-player games gain compile-time guardrails; the API stops round-tripping through serde_json::Value.",
        principles="§3 (static typing)",
        scope=("`StartResponse.state: serde_json::Value` and `TickResponse.state: serde_json::Value` in `games/common.rs` "
               "propagate through every single-player route. Each game produces JSON via a `render_json() -> String` then "
               "re-parses it (`map_to_value`). Replace with a per-game `GameState` struct that implements `Serialize`, and "
               "parameterise `StartResponse<S>`/`TickResponse<S>`."),
        acceptance=(
            "- `pub trait YggGame { type State: Serialize; fn render(&self) -> Self::State; }`.\n"
            "- `map_to_value` deleted.\n"
            "- Same wire JSON (regression test against current snapshots)."
        ),
        blast="Single-player games only; poker is unaffected (it already uses concrete `Serialize` structs).",
    ))

    write(YG / "YG-43.md", render_story(
        id=43, key="YG", parent=48,
        title="Carve out lobby/ folder; collapse core::lobby ↔ web::lobby_routes split",
        commit="refactor(lobby):", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:refactor", "module:lobby"],
        role="A maintainer of yggdrasil",
        need="Lobby concerns collected under `yggdrasil-core/src/lobby/` and `yggdrasil-web/src/lobby/` per-feature folders",
        so_that="`main.rs`'s HTML-serving for lobby moves inside the feature; lobby strings live in core, not web.",
        principles="§6 (feature folders), §2 (SRP)",
        scope=("`yggdrasil-core/src/lobby.rs` builds the `Universe`; `yggdrasil-web/src/lobby_routes.rs` exposes it. Today "
               "they are files at different layers, with no obvious feature folder. Introduce "
               "`yggdrasil-core/src/lobby/{mod,grid,portals}.rs` and `yggdrasil-web/src/lobby/{mod,routes,html}.rs`. The HTML "
               "serving in `main.rs` (`serve_lobby`) moves into `web::lobby::routes` too."),
        acceptance=(
            "- `main.rs` only calls `lobby::router()` for both HTML and JSON.\n"
            "- All lobby-related strings (`\"Escolha um universo para entrar\"`, portal positions) live in `core::lobby`."
        ),
        blast="Cosmetic; no behaviour change.",
    ))

    write(YG / "YG-44.md", render_story(
        id=44, key="YG", parent=49,
        title="Segregate per-game DB connections behind a ScoresStore trait",
        commit="refactor(scores):", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:refactor", "module:scores"],
        role="A maintainer of yggdrasil",
        need="One `Arc<dyn ScoresStore>` (or generic) injected into all four game states instead of four parallel `Connection::open` calls",
        so_that="The four-connection accident becomes intentional; tests get an in-memory impl.",
        principles="§5 (segregated state), §4 (coupling)",
        scope=("`make_snake_state`, `make_tetris_state`, `make_invaders_state`, and `ScoresState` each open their own "
               "`Mutex<rusqlite::Connection>` to the same `yggdrasil.db`. SQLite tolerates this but the design is "
               "unintentional. Either (a) one shared `Arc<Mutex<Connection>>` injected into all four, or (b) a `ScoresStore` "
               "trait abstraction (so prod uses shared SQLite, tests use in-memory) is cleaner."),
        acceptance=(
            "- One `Arc<dyn ScoresStore>` (or generic) passed to all four game states; four parallel `Connection::open` "
            "calls deleted.\n"
            "- In-memory test impl provided."
        ),
        blast="Boot logic in `main.rs` + the four `make_*_state` factories.",
    ))

    write(YG / "YG-45.md", render_story(
        id=45, key="YG", parent=49,
        title="Trim auth.rs and api/me.rs (each >600 LOC)",
        commit="refactor(auth):", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:refactor", "module:auth"],
        role="A maintainer of yggdrasil",
        need="`auth.rs` split into `auth/{jwt, magic_link, state}.rs`; tests moved to sibling test files",
        so_that="No file exceeds 300 LOC; sign/verify and magic-link request/verify each live in one place.",
        principles="§2 (SRP)",
        scope=("Both files mix domain logic, HTTP handlers, and large `#[cfg(test)]` blocks. Move tests into sibling "
               "`tests/auth.rs` and `tests/me.rs` (integration test crates) or `_tests.rs` modules. Inside `auth.rs`, split: "
               "`auth/jwt.rs` (sign/verify), `auth/magic_link.rs` (request/verify code), `auth/state.rs` (`AuthState`, DB "
               "schema)."),
        acceptance=(
            "- No single file > 300 LOC after split (target, not strict).\n"
            "- All tests still pass."
        ),
        blast="Imports only; no public API change.",
    ))

    write(YG / "YG-46.md", render_story(
        id=46, key="YG", parent=49,
        title="Document 'no per-game DB' + correct the persistence model",
        commit="docs:", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:docs"],
        role="A maintainer of yggdrasil",
        need="`docs/architecture/data-model.md` accurately describing the two-DB layout (`yggdrasil.db` + `yggdrasil-sementes.db`)",
        so_that="The per-game DB myth retires; new contributors learn the real persistence shape.",
        principles="(audit hygiene)",
        scope=("The story brief assumes per-game SQLite files. Reality is one shared `yggdrasil.db` + one "
               "`yggdrasil-sementes.db`. Either update `docs/ARQUITETURA-UNIVERSOS.md` to reflect reality, or actually split "
               "the DBs per game (probably not worth it for SQLite). Cheap doc-only task to retire the confusion."),
        acceptance=("- `docs/ARQUITETURA-UNIVERSOS.md` (or new `docs/architecture/data-model.md`) describes the two-DB layout, "
                    "the shared `scores` table, and poker's own tables."),
        blast="Zero — docs only.",
    ))

    # --- YG Epics ---
    write(YG / "YG-47.md", render_epic(
        id=47, title="Epic — Cross-repo coupling (1.0.0 closure)",
        commit_default="chore(deps):", semver="major",
        priority="high", module="yggdrasil",
        labels=["type:chore", "epic", "release:1.0"],
        goal=("Close the long-open YG-17 path-dependency. Yggdrasil moves from `path = \"../co/game-core\"` to a pinned git "
              "rev; CI can finally run on a clean checkout. This is the stability commitment that earns the 1.0.0 jump."),
        children=["YG-38 — Pin game-core to git rev + delete path dep + drop fly.toml hack"],
        acceptance=("`Cargo.toml` references `game-core` by git rev; `fly.toml` build comment gone; "
                    "`docs/DEPENDENCIES.md` documents bump policy; CI green on a clean checkout without adjacent `co/`."),
    ))

    write(YG / "YG-48.md", render_epic(
        id=48, title="Epic — Game adapter + multiplayer",
        commit_default="refactor(games):", semver="minor",
        priority="medium", module="yggdrasil",
        labels=["type:refactor", "epic"],
        goal=("Promote `YggGame` to a real adapter trait used by all four games, split `poker_routes.rs` by responsibility, "
              "introduce a `tokio::sync::broadcast` event spine plus a WS upgrade route for live poker, and carve out a "
              "proper `lobby/` folder. After this epic, adding a new game costs 30 LOC of route + a trait impl."),
        children=[
            "YG-39 — Promote YggGame to a real adapter trait",
            "YG-40 — Split poker_routes.rs (1189 LOC) by responsibility",
            "YG-41 — Introduce event spine + WS for poker",
            "YG-43 — Carve out lobby/ folder; collapse the layer split",
        ],
        acceptance=("All 4 games implement `YggGame`; route boilerplate collapses to ~30 LOC each; poker has WS streaming with "
                    "typed `TableEvent`; lobby lives in one feature folder per layer."),
    ))

    write(YG / "YG-49.md", render_epic(
        id=49, title="Epic — State + types",
        commit_default="refactor(types):", semver="patch",
        priority="low", module="yggdrasil",
        labels=["type:refactor", "epic"],
        goal=("Drop the `serde_json::Value` in single-player game state payloads, segregate per-game DB connections behind a "
              "`ScoresStore` trait, trim `auth.rs`/`api/me.rs` below 300 LOC, and correct the data-model documentation."),
        children=[
            "YG-42 — Replace serde_json::Value in game state with concrete types",
            "YG-44 — Segregate per-game DB connections via ScoresStore trait",
            "YG-45 — Trim auth.rs and api/me.rs",
            "YG-46 — Document 'no per-game DB' + correct the persistence model",
        ],
        acceptance=("No `serde_json::Value` in game state payloads; one `Arc<dyn ScoresStore>` shared across game states; "
                    "no file >300 LOC in `auth/` or `api/me/`; data-model doc reflects reality."),
    ))

    update_project_next_id(YG / "project.yaml", 50)

# ============================================================
# Run
# ============================================================
if __name__ == "__main__":
    fix_co_status()
    gen_rfq()
    gen_qb()
    gen_al()
    gen_yg()
    print("\nDone.")
