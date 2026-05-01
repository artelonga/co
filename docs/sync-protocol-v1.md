# CO Sync Protocol v1

Version: **v1.0**
Status: **Specification — implementation in CO-61**
Authors: CO project
Date: 2026-04-30

---

## Table of Contents

1. [Overview](#overview)
2. [PR Analogy](#pr-analogy)
3. [Foundational Principles](#foundational-principles)
4. [Core Types](#core-types)
   - [Hlc](#hlc-hybrid-logical-clock)
   - [Ator](#ator-actor-identity)
   - [Alvo](#alvo-target)
   - [Operacao](#operacao-operation)
   - [Manifesto](#manifesto)
   - [Proposta](#proposta-sync-proposal)
   - [Conflito](#conflito-conflict)
   - [RelatorioMesclagem](#relatoriomesclagem-merge-report)
5. [Op Catalog v1](#op-catalog-v1)
6. [Merge Algorithm](#merge-algorithm)
7. [Recursive Resolution](#recursive-resolution)
8. [Prod-Wins Policy](#prod-wins-policy)
9. [Copia Semantics](#copia-semantics)
10. [Idempotency and Atomicity](#idempotency-and-atomicity)
11. [Transport](#transport)
12. [Auth](#auth)
13. [Reducer Rules](#reducer-rules)
14. [Test Vectors](#test-vectors)
15. [Compatibility Notes](#compatibility-notes)
16. [Semver](#semver)

---

## Overview

The CO Sync Protocol v1 is a **local-first, op-log-based synchronization protocol** for content universes. It governs how two CO nodes — for example a UAT staging environment and a production server, a desktop client and a cloud peer, or two federated instances — exchange changes safely without losing work.

The immediate driver is **UAT → prod sync** for the quilombo-blog workflow: content is developed and reviewed on UAT, then promoted to production. The protocol is designed to compose with CRDT-based rich-text editing (CO-66), git-backed canonical state (CO-68), and future federated peers (CO-67) without rework.

**Key properties:**

- Every change is an immutable `Operacao` with a globally unique ID and a Hybrid Logical Clock timestamp.
- State is derived from the op log. Any state is rebuildable from `(ops, Estado₀)`.
- Synchronization is expressed as a `Proposta` (a batch of ops from a sender) and a `RelatorioMesclagem` (the merge report from the target).
- Conflicts are first-class data. They are detected deterministically and resolved via a `conflito.resolver` op — itself appended to the log.
- Prod wins by default. UAT changes require explicit promotion.

---

## PR Analogy

The sync flow mirrors a GitHub pull request:

| Git/GitHub concept | CO Sync equivalent |
|---|---|
| Feature branch | UAT node's op log since `base_hlc` |
| Base branch | Prod node's op log |
| `git merge-base` | `base_hlc` — the last HLC both nodes share |
| Pull request | `Proposta` |
| Merge review / conflicts tab | `RelatorioMesclagem` |
| Approved and merged | `aplicadas` op IDs — landed cleanly |
| Merge conflict marker | `Conflito` — two ops touched the same entity concurrently |
| Resolve conflict | `conflito.resolver` op — appended to the log |
| Force push with author's version | `estrategia: "uat"` — UAT wins |
| Accept upstream | `estrategia: "prod"` — prod wins (default) |
| Cherry-pick to new branch | `estrategia: "copia"` — synthesize new entity |

A `Proposta` is a pull request from UAT to prod. The server processes it, computes the three-way merge, and returns a `RelatorioMesclagem`. Clean ops are applied. Conflicting ops are parked and surfaced to the admin for resolution.

---

## Foundational Principles

1. **Op log is canonical. State is derived.**
   The reducer `(ops, Estado₀) → Estado` is a pure function. Any state at any point in time is rebuildable from the complete op log. This makes the system fully auditable and reversible.

2. **Every change is an op.**
   Content edits, schema extensions, config changes, and even conflict resolutions are all `Operacao` records. One primitive, infinite expressivity.

3. **Merge is a function, not a feature.**
   Given a `base_hlc` and two op sets, the `mesclar` function returns `(aplicadas, conflitos, novas_ops_remotas)` deterministically. No hidden state. No race conditions.

4. **Resolutions are ops.**
   A `conflito.resolver` op is appended to the log like any other op. This makes resolution recursive by construction and auditable. Reverting a resolution means emitting another resolver op — the DAG records the full override chain.

5. **Prod-wins by default.**
   Production traffic always takes precedence. UAT changes require explicit promotion via `conflito.resolver{estrategia:"uat"}`. This prevents accidental overwrite of production data.

6. **Causality is explicit.**
   Every `Operacao` carries a `pai` list of causal parent op IDs. The `mesclar` function uses transitive causal ancestry to distinguish concurrent ops (potential conflict) from causally ordered ops (safe apply). If UAT's op has prod's op in its ancestry, it is not concurrent — it is a deliberate follow-on change.

7. **Idempotency everywhere.**
   Re-submitting the same `Proposta.id` is a no-op. Duplicate op IDs within a proposta are deduplicated. All state-mutating endpoints are wrapped in SQLite transactions.

---

## Core Types

### Hlc (Hybrid Logical Clock)

Hybrid Logical Clock per Kulkarni et al. 2014. Provides a total order over events across distributed nodes combining physical wall-clock time with a logical counter.

**Rust type:**
```rust
pub struct Hlc {
    pub wall_ms: u64,   // milliseconds since Unix epoch
    pub counter: u32,   // monotonically increasing per wall tick
    pub node: u128,     // node identity (same bits as node UUID)
}
```

**Serialization:** `{wall_ms}:{counter}:{node_hex32}` — a 32-hex-char lowercase node field with zero-padding.

Examples:
- `1714500000000:0:aaaaaaaa000000000000000000000000`
- `1714500000001:3:bbbbbbbb000000000000000000000000`
- `1000:0:00000000000000000000000000000000` — zero base HLC used in fixtures

**Ordering rules (total):**
1. Higher `wall_ms` wins.
2. At equal `wall_ms`, higher `counter` wins.
3. At equal `wall_ms` and `counter`, higher `node` value wins (tie-break).

**Update rules on receive:**
```
local_hlc.wall_ms = max(local_wall, received.wall_ms)
if local_hlc.wall_ms == received.wall_ms:
    local_hlc.counter = max(local_hlc.counter, received.counter) + 1
else:
    local_hlc.counter = 0
```

**Update rules on local event:**
```
new_wall = max(local_wall_ms_now, local_hlc.wall_ms)
if new_wall == local_hlc.wall_ms:
    local_hlc.counter += 1
else:
    local_hlc.counter = 0
local_hlc.wall_ms = new_wall
```

**JSON Schema:** `docs/sync-protocol-v1/schemas/hlc.json`

---

### Ator (Actor Identity)

Identifies who emitted an operation. Every installation generates a stable `node_id` UUID at first boot. Authenticated users are tracked via `user_id`. System-emitted ops (migrations, server-side resolvers) have `user_id: null`.

**Rust type:**
```rust
pub struct Ator {
    pub node_id: Uuid,
    pub user_id: Option<Uuid>,
}
```

**Serialization:** JSON object `{ "node_id": "...", "user_id": "..." | null }`.

**JSON Schema:** `docs/sync-protocol-v1/schemas/ator.json`

---

### Alvo (Target)

The addressed entity and optional field for an operation. Two ops conflict if and only if they share the same `Alvo` and are not causally related.

**Rust type:**
```rust
pub struct Alvo {
    pub tipo: String,        // entity type: "foto", "relato", "conflito", "schema"
    pub id: String,          // entity identifier: slug, SHA-256, conflict hash
    pub campo: Option<String>, // field name, or null for the whole entity
}
```

**Examples:**
- `{ "tipo": "foto", "id": "<sha256>", "campo": null }` — whole photo entity
- `{ "tipo": "relato", "id": "oficina-agua", "campo": "titulo" }` — a specific field
- `{ "tipo": "relato", "id": "r1", "campo": "destaque" }` — destaque field on relato r1
- `{ "tipo": "conflito", "id": "c1", "campo": null }` — a conflict record
- `{ "tipo": "schema", "id": "v1", "campo": null }` — the schema itself

**Conflict scope:** Ops on the same `tipo`+`id` but different `campo` values do NOT conflict with each other (e.g., updating `titulo` and `destaque` separately is safe). Ops with `campo: null` conflict with any op on the same entity regardless of campo.

**JSON Schema:** `docs/sync-protocol-v1/schemas/alvo.json`

---

### Operacao (Operation)

A single immutable record in the op log. Once appended, an `Operacao` is never modified or deleted. Reversals are expressed as new ops.

**Rust type:**
```rust
pub struct Operacao {
    pub id: Uuid,                       // UUIDv7 preferred (time-sortable)
    pub hlc: Hlc,                       // emission timestamp
    pub ator: Ator,                     // who emitted this op
    pub tipo: String,                   // namespaced: "foto.adicionar", "conteudo.atualizar"
    pub alvo: Alvo,                     // what entity/field this op addresses
    pub args: serde_json::Value,        // tipo-specific payload (see Op Catalog)
    pub pai: Vec<Uuid>,                 // causal parent op IDs
    pub assinatura: Option<Ed25519Sig>, // reserved for federation (v1.1)
}
```

**Immutability:** Op IDs are unique. `INSERT OR IGNORE` is used when persisting; re-submitting the same ID is safe.

**Causal parents (`pai`):** Typically 1 parent (the last op the emitter knew about). Merge ops may carry multiple parents. An empty `pai` means the op is causally independent.

**JSON Schema:** `docs/sync-protocol-v1/schemas/operacao.json`

---

### Manifesto

A compact peer state summary. Peers exchange manifestos to detect divergence without transferring full op lists. If two peers share the same `ops_merkle_root`, they are in sync.

**Rust type:**
```rust
pub struct Manifesto {
    pub node_id: Uuid,
    pub hlc_atual: Hlc,               // most recent HLC this node has seen
    pub ops_count: u64,               // total ops in this node's log
    pub ops_merkle_root: [u8; 32],    // SHA-256 of op DAG root
    pub blobs_count: u64,             // total blobs stored
    pub blobs_merkle_root: [u8; 32],  // SHA-256 of blob manifest
    pub schema_version: String,       // e.g. "v1.2"
    pub protocol_version: String,     // always "v1.0" for this spec
}
```

**Serialization of byte arrays:** 64-char lowercase hex strings.

**JSON Schema:** `docs/sync-protocol-v1/schemas/manifesto.json`

---

### Proposta (Sync Proposal)

A sync proposal sent by a source peer (UAT) to a target peer (prod). Semantically equivalent to a pull request. Contains all ops the sender has emitted since the shared `base_hlc`, plus a list of blob digests the sender holds (so the target can request missing ones).

**Rust type:**
```rust
pub struct Proposta {
    pub id: Uuid,                          // stable ID; re-submitting same ID = no-op
    pub peer_origem: Uuid,                 // sender's node_id
    pub base_hlc: Hlc,                     // last HLC the two peers shared
    pub ops: Vec<Operacao>,                // sender's ops since base_hlc
    pub blobs_disponiveis: Vec<Sha256>,    // blobs the sender can provide
    pub criado_em: DateTime<Utc>,
}
```

**Workflow:**
1. Sender fetches `GET /sync/api/manifesto` from target to determine `base_hlc`.
2. Sender constructs `Proposta` with its ops since `base_hlc`.
3. Sender posts `POST /sync/api/proposta`.
4. Target responds with `RelatorioMesclagem`.
5. For each `blobs_solicitados`, sender uploads the blob via `POST /sync/api/proposta/{id}/blob/{sha256}`.
6. Admin resolves any `conflitos` via `POST /sync/api/proposta/{id}/resolver`.

**JSON Schema:** `docs/sync-protocol-v1/schemas/proposta.json`

---

### Conflito (Conflict)

A conflict record created by the merge function when two concurrent ops (neither is a causal ancestor of the other) address the same `Alvo`. The conflict is itself data in the system — it can be displayed to the admin, resolved by emitting a `conflito.resolver` op, or left pending.

**Rust type:**
```rust
pub struct Conflito {
    pub id: Uuid,            // deterministic: SHA-256 of sorted op IDs, first 16 bytes
    pub op_local: Uuid,      // target (prod) op
    pub op_remota: Uuid,     // sender (UAT) op
    pub alvo: Alvo,          // the entity/field in contention
    pub opcoes: Vec<String>, // always ["prod", "uat", "copia", "manual"]
    pub sugestao: String,    // always "prod" (prod-wins default)
}
```

**Deterministic ID:** The conflict ID is computed as:
```
SHA-256(sorted(op_ids))[:16] with version=5 and RFC-4122 variant bits
```
This ensures the same pair of conflicting ops always produces the same conflict ID, making conflicts idempotent across re-submissions.

**Resolution options:**
- `"prod"` — keep prod's op, discard UAT's op.
- `"uat"` — keep UAT's op, override prod's op.
- `"copia"` — synthesize a new entity that preserves both sides' intent (e.g., rename UAT's entity to `r1-copy`).
- `"manual"` — admin provides custom resolution in `conflito.resolver.detalhes`.

**JSON Schema:** `docs/sync-protocol-v1/schemas/conflito.json`

---

### RelatorioMesclagem (Merge Report)

The response to a `Proposta`. Returned immediately after processing. Contains:

- Which remote (UAT) ops applied cleanly (`aplicadas`).
- Which remote ops conflicted with local (prod) ops (`conflitos`).
- Which local (prod) ops the sender did not know about (`novas_ops_remotas`) — used to update the sender's display.
- Which blob digests the target needs the sender to upload (`blobs_solicitados`).

**Rust type:**
```rust
pub struct RelatorioMesclagem {
    pub proposta_id: Uuid,
    pub aplicadas: Vec<Uuid>,           // UAT op IDs that merged cleanly
    pub conflitos: Vec<Conflito>,       // UAT ops in conflict with prod ops
    pub novas_ops_remotas: Vec<Operacao>, // prod ops since base_hlc (for UAT's awareness)
    pub blobs_solicitados: Vec<Sha256>, // blobs to upload
}
```

**JSON Schema:** `docs/sync-protocol-v1/schemas/relatorio_mesclagem.json`

---

## Op Catalog v1

All op types are namespaced with a dot-separated path. The namespace is the entity domain; the verb follows. New namespaces can be added without modifying the merge algorithm.

### foto.*

| Op | Args | Notes |
|---|---|---|
| `foto.adicionar` | `{ sha256, mime, tamanho, dados_blob_ref }` | Registers a new blob. `sha256` is the content address. |
| `foto.remover` | `{ sha256 }` | Marks a blob as deleted. Blob bytes may be garbage-collected. |
| `foto.vincular` | `{ sha256, alvo_tipo?, alvo_slug?, indice?, destaque? }` | Associates a foto with content. Targeting a `campo` (e.g., `destaque`) narrows conflict scope. |
| `foto.desvincular` | `{ sha256, alvo_tipo, alvo_slug }` | Removes the association. |

### conteudo.*

| Op | Args | Notes |
|---|---|---|
| `conteudo.criar` | `{ tipo, slug, meta, corpo }` | Creates a new content entity. `slug` becomes the `Alvo.id`. |
| `conteudo.atualizar` | `{ tipo, slug, meta_delta?, corpo? }` | Partial update. `meta_delta` is a JSON merge patch. |
| `conteudo.excluir` | `{ tipo, slug }` | Marks content as deleted. Reducer ignores deleted entities in state projection. |

### conflito.*

| Op | Args | Notes |
|---|---|---|
| `conflito.detectado` | `{ conflito_id, ops: [op_id], alvo, campo }` | Emitted by the server; records the conflict in the log. |
| `conflito.resolver` | `{ conflito_id, estrategia, ops_efetivas: [op_id], ops_superadas: [op_id], detalhes? }` | Resolution op. `detalhes` may carry `novo_slug`, `campos_fundidos`, `comentario`. |

### schema.*

| Op | Args | Notes |
|---|---|---|
| `schema.estender` | `{ propriedades_adicionadas, tipos_adicionados }` | Extends the universe's schema. Commutes with all content ops (different `Alvo.tipo`). |

---

## Merge Algorithm

The `mesclar` function is the core of the protocol. It is a **pure function** — same inputs always produce the same output. No I/O. No database calls. The calling layer (HTTP handler, in-memory store) is responsible for persistence.

```
function mesclar(proposta_id, base_hlc, ops_local, ops_remota) -> RelatorioMesclagem:

    # 0. Deduplicate remote ops by ID (idempotent re-submission)
    ops_remota = deduplicate_by_id(ops_remota)

    # 1. Partition local ops: those strictly after base_hlc
    ops_local_pos_base = [op for op in ops_local if op.hlc > base_hlc]

    # 2. Build combined op lookup for causality walks
    all_ops = {op.id: op for op in ops_local + ops_remota}

    # 3. For each remote op, check for concurrent local ops on the same Alvo
    aplicadas, conflitos = [], []
    for r in ops_remota:
        concorrentes = [
            l for l in ops_local_pos_base
            if l.alvo == r.alvo
            and not causal_ancestor(l.id, r, all_ops)
            and not causal_ancestor(r.id, l, all_ops)
        ]
        if not concorrentes:
            aplicadas.append(r.id)
        else:
            conflitos.append(Conflito {
                id: conflito_id_de(r.id, [l.id for l in concorrentes]),
                op_local: concorrentes[0].id,
                op_remota: r.id,
                alvo: r.alvo,
                opcoes: ["prod", "uat", "copia", "manual"],
                sugestao: "prod",
            })

    # 4. Collect blob requests for applied foto.adicionar ops not already local
    blobs_solicitados = [
        op.args["sha256"]
        for op in ops_remota
        if op.id in aplicadas
        and op.tipo == "foto.adicionar"
        and op.args["sha256"] not in blobs_locais
    ]

    return RelatorioMesclagem {
        proposta_id,
        aplicadas,
        conflitos,
        novas_ops_remotas: ops_local_pos_base,
        blobs_solicitados,
    }
```

**`causal_ancestor(op_a_id, op_b, all_ops)`:** Returns true iff `op_a_id` is reachable by walking `op_b.pai` transitively. Uses a visited set to handle DAG structure without infinite loops.

**Complexity:** O(|ops_remota| × |ops_local_pos_base|) naive. For production use with large op sets, index `ops_local_pos_base` by `alvo` to achieve O((|local| + |remota|) log n).

**Determinism:** The merge result is fully deterministic given the same inputs. This is essential for testing via JSON fixtures and for auditability.

---

## Recursive Resolution

A `conflito.resolver` op with `estrategia: "copia"` may synthesize new `conteudo.criar` ops for the duplicated entity. These synthesized ops may themselves conflict with existing prod ops (e.g., the copy's slug already exists on prod). This creates a recursive conflict.

The protocol handles this by design:

1. The admin resolves conflict C1 via `conflito.resolver{estrategia:"copia", ...}`.
2. The resolver emits sub-ops (new `conteudo.criar` with slug `r1-copy`).
3. Merge is re-run on the sub-ops.
4. If `r1-copy` conflicts with an existing prod op, a new `Conflito` C2 is created.
5. The admin resolves C2.

**Termination theorem:** Resolution always terminates.

**Proof:** Each `conflito.resolver` op has `hlc_resolver > max(hlc_op_a, hlc_op_b)` by HLC monotonicity. Each emitted sub-op has `hlc > hlc_resolver`. There are finitely many ops in any `Proposta`. Since HLC is strictly increasing and the set of ops is finite, the resolver DAG has bounded depth ≤ |proposta.ops|. QED.

---

## Prod-Wins Policy

When two concurrent ops address the same `Alvo` (same `tipo`, `id`, and `campo`), and neither is a causal ancestor of the other, the merge algorithm creates a `Conflito` with `sugestao: "prod"`.

This is the **prod-wins policy**: by default, production data takes precedence. The production environment serves live user traffic; its state is authoritative. UAT changes are staging changes that require explicit promotion.

**Rationale:**
- Prevents accidental overwrite of production data during UAT deploys.
- Aligns with the mental model: UAT is a candidate; prod is truth.
- Easy to override: an admin can always pick `estrategia: "uat"` to promote UAT's version.

**Override mechanism:**
```json
POST /sync/api/proposta/{id}/resolver
{
  "resolucoes": [
    {
      "conflito_id": "...",
      "estrategia": "uat",
      "ops_efetivas": ["<uat-op-id>"],
      "ops_superadas": ["<prod-op-id>"]
    }
  ]
}
```

The server appends a `conflito.resolver` op to the log. The reducer marks `ops_superadas` as overridden. On the next state projection, UAT's op is applied and prod's op is skipped.

---

## Copia Semantics

`estrategia: "copia"` is for cases where **neither side's op should win outright** — typically when UAT deleted an entity that prod updated, or when both sides created an entity with the same slug but different content.

**Scenario:** UAT deletes `relato/r1`; prod updates `relato/r1` concurrently.
- `estrategia: "prod"` keeps the update, discards the delete.
- `estrategia: "uat"` applies the delete, loses the prod update.
- `estrategia: "copia"` synthesizes `r1-copy` on UAT's side (or a new slug chosen by the admin) and keeps prod's `r1` intact.

**How `copia` works:**

1. Admin resolves the conflict with `estrategia: "copia"` and provides `detalhes.novo_slug = "r1-copy"`.
2. Server emits a `conflito.resolver` op appended to the log.
3. Reducer interprets this resolver: the UAT entity is renamed to `r1-copy` (new `conteudo.criar` op emitted by the resolver), and prod's entity `r1` is retained.
4. The `r1-copy` synthesis may conflict with an existing prod entity — if so, a nested `Conflito` is created (see Recursive Resolution).

**Admin UX:** The resolver endpoint accepts a `detalhes` object:
```json
{
  "estrategia": "copia",
  "detalhes": {
    "novo_slug": "r1-copy",
    "comentario": "Renaming UAT's r1 to r1-copy to preserve both versions"
  }
}
```

---

## Idempotency and Atomicity

| Operation | Guarantee | Mechanism |
|---|---|---|
| `POST /sync/api/proposta` | Idempotent by `Proposta.id` | `INSERT OR IGNORE` on op table; proposta status check |
| `POST /sync/api/proposta/{id}/resolver` | Idempotent by resolver op ID | Same; resolver op UUID is stable |
| Sync cycle mid-failure | No partial state | All ops from a proposta applied in one SQLite transaction |
| Duplicate op IDs in proposta | Deduplicated before merge | `HashSet<Uuid>` seen during `mesclar` |
| Double-submit of same op | Safe no-op | UNIQUE constraint on `ops.id` in database |
| Out-of-order blob upload | Safe | Blobs are content-addressed; SHA-256 verified before acceptance |
| `foto.remover` on unknown blob | No-op with audit entry | Reducer emits op with `effective: false` annotation |
| `conteudo.atualizar` with no change | Recorded, marked non-effective | Reducer detects no-change via hash comparison |

---

## Transport

### HTTP Endpoints (REST)

All endpoints are under the `/sync/api/` prefix.

| Endpoint | Method | Auth | Body | Response | Notes |
|---|---|---|---|---|---|
| `/sync/api/manifesto` | GET | none | — | `Manifesto` | Public; allows divergence detection without auth |
| `/sync/api/ops` | GET | Bearer | — | NDJSON stream of `Operacao` | Query param: `desde_hlc=<hlc>` |
| `/sync/api/blob/{sha256}` | GET | Bearer | — | Blob bytes | `Content-Type` from stored mime |
| `/sync/api/proposta` | POST | Bearer | `Proposta` | `RelatorioMesclagem` | Main sync entry point |
| `/sync/api/proposta/{id}` | GET | Bearer | — | Proposta status + last `RelatorioMesclagem` | Polling for async processing |
| `/sync/api/proposta/{id}` | DELETE | Bearer | — | `{ "ok": true }` | Cancel an open proposta |
| `/sync/api/proposta/{id}/blob/{sha256}` | POST | Bearer | Blob bytes | `{ "ok": true }` | Upload a requested blob |
| `/sync/api/proposta/{id}/resolver` | POST | Bearer | `{ "resolucoes": [...] }` | `{ "estado_final": ..., "novo_hlc": ... }` | Resolve one or more conflicts |

**All state-mutating endpoints** wrap application in a SQLite transaction — either all ops apply or none do.

**NDJSON streaming** for `/sync/api/ops` allows efficient transfer of large op sets without loading the full list into memory.

### WebSocket (v1.1, reserved for CO-67)

`GET /ws/sync/{universe}` — duplex op stream for real-time collaboration. Ops are pushed as they are committed. The initial message is a `Manifesto` for sync bootstrapping. Detailed protocol defined in CO-67.

---

## Auth

### v1.0 (Shared Secret)

```
Authorization: Bearer {SYNC_TOKEN}
```

A rotatable shared secret configured via the `SYNC_TOKEN` environment variable. Simple and sufficient for admin-to-admin UAT→prod sync.

The `GET /sync/api/manifesto` endpoint requires no auth — it exposes only aggregate counts and hash roots, no content.

### v1.1 (Federation, reserved)

When CO-67 ships, authentication will be upgraded to:

- JWT per node, signed with the node's Ed25519 private key.
- Op signatures verified on receipt (the `Operacao.assinatura` field, currently always `null`).
- Capability tokens scoped per universe for fine-grained access control.
- Unlocked by `CO_FEATURE_FEDERATION=true` environment variable.

The v1.0 and v1.1 auth layers are additive — v1.0 clients continue to work via the shared secret.

---

## Reducer Rules

The reducer applies the op log to compute current state. Rules:

1. **HLC-topological order.** Ops are applied in HLC order, respecting the `pai` DAG. A parent op is always applied before its children.

2. **`conflito.resolver` semantics:**
   - Ops listed in `ops_superadas` are marked as overridden in the reducer index. They are excluded from state projection going forward.
   - Ops listed in `ops_efetivas` are retained and applied normally.
   - Each op may be superseded by at most one resolver (unique constraint).
   - Re-applying a resolver with the same ID is a no-op (`INSERT OR IGNORE`).

3. **Reverting a resolution:** Emit a new `conflito.resolver` op with the reverse mapping: swap `ops_efetivas` and `ops_superadas`. The DAG records the full chain. The most recent resolver in HLC order wins.

4. **`foto.adicionar` without the blob:** The op is accepted and applied. The blob fetch is deferred — the `RelatorioMesclagem.blobs_solicitados` list triggers the transfer. If the blob never arrives, the op remains pending and photos in that batch are shown as unavailable until upload completes.

5. **Schema ops commute with content ops.** A `schema.estender` op addresses `Alvo{ tipo: "schema", ... }`, which never equals any content entity's `Alvo`. Therefore schema ops never conflict with content ops and are always applied cleanly.

6. **Deleted entities are retained in the log.** A `conteudo.excluir` op does not remove earlier ops from the log. It appends a new op that causes the reducer to exclude the entity from state projections. The full history remains accessible.

---

## Test Vectors

Fixtures are in `docs/sync-protocol-v1/fixtures/`. Each is a JSON file with `input` and `expected` fields. The `core/tests/sync_fixtures.rs` test runner loads each fixture and asserts the output of `mesclar(...)` matches `expected`.

### UUID Conventions (all fixtures)

| Name | Value |
|---|---|
| `PROPOSTA_1` | `11111111-1111-1111-1111-111111111111` |
| `PROD_NODE_UUID` | `aaaaaaaa-0000-0000-0000-000000000000` |
| `UAT_NODE_UUID` | `bbbbbbbb-0000-0000-0000-000000000000` |
| `PROD_NODE_HLC_HEX` | `aaaaaaaa000000000000000000000000` |
| `UAT_NODE_HLC_HEX` | `bbbbbbbb000000000000000000000000` |
| `base_hlc` | `1000:0:00000000000000000000000000000000` |
| Prod op IDs | `00000000-0001-0000-0000-00000000000N` |
| UAT op IDs | `00000000-0002-0000-0000-00000000000N` |
| SHA256 foto-a | `aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111` |
| SHA256 foto-b | `bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222` |

Prod HLCs are `≥ 1500` (after base 1000). UAT HLCs are `≥ 2000`.

### Fixture Index

| File | Scenario | Key assertion |
|---|---|---|
| `01-clean-add.json` | UAT adds foto; prod has no post-base ops | UAT op applied; blob requested |
| `02-prod-advance.json` | Both add independent fotos (different alvos) | Both applied; prod op in novas_ops_remotas |
| `03-same-slot-prod-wins.json` | Both set destaque on same relato concurrently | Conflict detected; sugestao=prod |
| `04-same-slot-uat-override.json` | Same as 03 plus a clean resolver op | Resolver applied; destaque conflict still detected |
| `05-copia.json` | UAT deletes, prod updates same entity | Conflict detected (copia strategy appropriate) |
| `06-idempotent-retry.json` | Same UAT op ID appears twice | Deduped; applied exactly once |
| `07-recursive-resolution.json` | UAT creates r1 and r1-copy; prod also created both | Two independent conflicts |
| `08-causality-respected.json` | UAT op has prod op in its pai | No conflict (causal, not concurrent) |
| `09-schema-migration.json` | UAT extends schema; prod creates content | Different alvo tipos; both applied |
| `10-reversibility.json` | UAT's resolver has prod's resolver in its pai | Causal; applied cleanly; no conflict |

### Running the tests

```bash
cargo test -p co sync
```

Or run a specific fixture test:

```bash
cargo test -p co fixture_01_clean_add -- --nocapture
```

---

## Compatibility Notes

### Relation to file-based sync (CO-51, CO-54)

The op-log protocol is **not a replacement** for the existing CLI + file-based sync track. It is the canonical layer underneath. File-level sync is a projection.

| CO-51/CO-54 concept | CO-61 equivalent |
|---|---|
| `sync.json` file hashes | `Manifesto.ops_merkle_root` + per-entry hash derived from reducer |
| last-write-wins | `conflito.resolver { estrategia: "prod" or "uat" }` |
| local-wins | `estrategia: "uat"` |
| remote-wins | `estrategia: "prod"` (default) |
| manual (`.local/.remote/.base` sidecar files) | `estrategia: "manual"` + server projection of sidecar files |
| 3-way auto merge on non-overlapping hunks | Non-conflicting ops apply cleanly |
| CO-54 `entry_versions` table | Materialized view rebuilt from op log |
| `.conflict` marker file | Server projection: for each open `Conflito`, write `{path}.conflict` to vault |

### Mapping `co sync` commands to protocol

| CLI command | Protocol operation |
|---|---|
| `co sync pull` | `GET /sync/api/manifesto` + reconstruct files from reducer state |
| `co sync push` | Compute diff → POST `Proposta` containing derived ops |
| `co sync status` | `GET /sync/api/manifesto` + compare to local sync.json |
| `co sync watch` | Poll `/sync/api/manifesto` (WebSocket in CO-67) |

### CO-55 (SSH keys)

SSH key management is for git clone authentication. Sync protocol auth is `SYNC_TOKEN` (v1.0) or JWT+Ed25519 (v1.1). Different layers. No conflict.

### CO-58 (Desktop tray + PWA)

Both natively consume `/sync/api/*` endpoints. The tray app watches vault files, computes diffs, and emits ops. The PWA holds pending ops in IndexedDB and flushes on reconnect. WebSocket push (CO-67) upgrades polling to live push — no protocol changes required.

### CO-66 (Automerge / CRDT rich text)

Automerge ops are embedded as `args` within a `conteudo.atualizar` op. The outer op log handles ordering, causality, and coarse conflict detection. Automerge handles fine-grained character-level merging within a single field. The two layers compose without protocol changes.

### CO-68 (Git projection)

Git canonical state is written by a background reducer that reads the op log and projects each entity to a file. The file tree becomes a read model. Writes always go through ops. The git history becomes a derived audit trail.

---

## Semver

This spec is versioned independently from the CO application version.

| Version | Status | Notes |
|---|---|---|
| v1.0 | Current | Shared-secret auth, REST transport, 3-way merge, 10 op types |
| v1.1 | Planned | WebSocket duplex stream (CO-67), Ed25519 federation auth, per-universe capability tokens |
| v2.0 | Future | Full CRDT embedding, git-backed canonical state (CO-68), multi-universe atomic transactions |

**Breaking changes** between minor versions:
- Adding new fields to `Operacao.args` for existing op types is non-breaking (unknown fields are ignored by older reducers).
- Adding new op types is non-breaking (older reducers log unknown types as `effective: false`).
- Changing `Alvo` semantics or `Hlc` ordering rules is a breaking change requiring a major version bump.

**Application version impact:**
- This spec addition corresponds to a minor version bump in the CO application (feat → x.Y.0).
- The commit message is: `feat(sync): CO-61 — sync protocol v1 spec with recursive conflict resolution`.
