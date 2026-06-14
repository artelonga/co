# StaaS — Storage as a Service for partners (multi-tenant design)

> **Status:** design (decisions + contract, no impl) — CO-460
> **Targets:** post-CO-458 (the `StorageBackend` keystone must land first)
> **Source specs:** [CO-458](../../work/co/CO-458.md) `StorageBackend` · [CO-448](../../work/co/CO-448.md) token capabilities · [CO-456](../../work/co/CO-456.md) API envelope · [CO-453](../../work/co/CO-453.md) telemetry/usage · [CO-80](../../work/co/CO-80.md) rate-limit + quota · [CO-459](../../work/co/CO-459.md) local backup · [CO-81](../../work/co/CO-81.md) S3 storage
> **Index:** linked from [`COMPOSABILITY.md`](COMPOSABILITY.md) (the external-consumer seam map)

This doc is the **written contract** for selling **storage as a service** (StaaS)
to *parceiros* (partners). It binds together the pieces that already exist or are
landing — it adds **no code, no migration, no route**. The implementation tasks in
§8 follow this contract instead of inventing the shape ad-hoc, so the owner can
approve the architecture before spending Opus on impl.

A **parceiro** rents an isolated **namespace** of storage. They `PUT/GET/DELETE/LIST`
blobs against it through a versioned API, authenticate with least-privilege tokens,
are metered on bytes×time + operations, and are bounded by per-tier quota. Optionally
a parceiro brings their own backend (S3-compatible) behind the same trait.

---

## 0. What already exists (the substrate we productize)

StaaS is an assembly of seams that are already in the tree (or specced). Nothing
below is new mechanism — the design **consumes** these on paper:

| Piece | Real artifact | Role in StaaS |
|-------|---------------|---------------|
| Storage trait | `StorageBackend` — `co-web/src/storage/backend/mod.rs` (CO-458, planned) | the per-namespace blob backend |
| Blob metadata | `blob_refs(hash, backend, size, content_type, refcount, created_at)` — migration **v086** (CO-458) | metering source (`size`), GC refcount |
| Existing blob seam | `BlobStore` trait + `BlobBackend::for_universe(dir, key)` — `co-web/src/infra/blob.rs` | proves the **prefix-by-key** namespacing pattern (`R2BlobStore { prefix }`) |
| Token capabilities | `KNOWN_CAPABILITIES`, `resolve_scopes()`, `grants()` — `co-web/src/auth/capabilities.rs` (CO-448) | partner auth scopes |
| Capability enforcement | `Scoped<C>` extractor + `Capability` trait — `co-web/src/auth/extractors.rs` | gate handlers by scope |
| Token storage | `api_tokens.scopes` (JSON) — `co-web/src/storage/api_tokens.rs` | persist partner scopes |
| API envelope | `envelope_middleware`, `wrap_response()` — `co-web/src/server/envelope.rs` (CO-456) | partner-facing response shape |
| Usage/metering | `usage_sessions` + `telemetry_api.rs` (`/metrics/throughput`, `token-budget`) (CO-453) | usage rollup surface |
| Quota + rate-limit | `Tier`, `TierLimits`, `RateLimiter`, `check_storage_quota()` — `co-web/src/platform/rate_limit.rs` (CO-80) | enforcement layer |
| Backup | `BackupBackend` trait + `LocalFsBackend` — `co-web/src/storage/backup/` (CO-459) | per-namespace durability |
| Remote durability | `S3Backend` stub — `co-web/src/storage/backup/s3.rs` (CO-81) | off-machine replica |

---

## 1. Modelo multi-tenant — parceiro → namespace

A **parceiro** is a billing+identity entity. Each parceiro owns one or more
**namespaces**; a namespace is an **isolated key-prefix** in a `StorageBackend`.
This is exactly the pattern `R2BlobStore` already uses for universes
(`prefix: universe_key`, joined as `format!("{prefix}/{key}")` in
`co-web/src/infra/blob.rs`) — StaaS generalizes "universe" to "partner namespace".

### Naming

```
ns/{partner_slug}/{namespace_slug}/{blob_key}
```

- `partner_slug`, `namespace_slug`: `^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$` (DNS-safe, lowercased).
- `blob_key`: opaque to the partner. Content-addressed blobs derive the key from
  the `sha256` (per CO-458 `put` contract: "key derivatable from sha256"), so the
  physical layout under a namespace mirrors CO-81's fanout
  (`{prefix}/blobs/sha256/<aa>/<bb>/<full>`).

### Isolation

- **Logical**: every `StorageBackend` call from a partner handler is rewritten to
  prepend the namespace prefix. A partner can never address a key outside their
  prefix — the prefix is injected server-side from the authenticated token, never
  taken from the request path. (`list(prefix)` is likewise pinned to the namespace.)
- **Physical**: with the LocalFs backend, a namespace is a subtree; with a partner
  S3 backend (§6), it is a bucket+prefix the partner controls.

### Mapping to `blob_refs` (CO-458)

`blob_refs` is **content-addressed and global** (dedup across the deployment), so it
is *not* itself partitioned by namespace. Tenancy is recorded in a **join table**
proposed by CO-461 (§8), not by duplicating blob bytes:

```
namespaces(id, partner_id, slug, backend, created_at, deleted_at)
blob_namespace(namespace_id, hash, key, created_at)   -- which ns references which blob_ref.hash
```

- `blob_refs.refcount` already exists for GC; `blob_namespace` rows are the
  per-tenant references that drive that count. A blob shared by two namespaces is
  stored once (`refcount = 2`), but **metered to each** (§4) — billing is per
  *reference-bytes*, not per *physical-bytes*, so dedup never under-bills a partner.
- Deleting a namespace soft-deletes its `blob_namespace` rows and decrements
  `refcount`; physical GC of `refcount = 0` blobs reuses the CO-81 30-day sweep.

---

## 2. API de parceiro (forma — envelope CO-456)

Versioned under `/api/v1`, scoped to the partner's namespace, wrapped by the
existing `envelope_middleware` (`co-web/src/server/envelope.rs`). **No routes are
created by this doc** — they are specced for CO-462.

| Method | Path | Scope (§3) | Notes |
|--------|------|-----------|-------|
| `PUT` | `/api/v1/staas/{ns}/blobs/{key}` | `storage:write` | body = bytes; `Content-Type` stored; returns `BlobRef`. Idempotent (content-addressed). |
| `GET` | `/api/v1/staas/{ns}/blobs/{key}` | `storage:read` | streams bytes; not envelope-wrapped (binary — see below). |
| `HEAD` | `/api/v1/staas/{ns}/blobs/{key}` | `storage:read` | `BlobMeta` (size, content_type, created_at). |
| `DELETE` | `/api/v1/staas/{ns}/blobs/{key}` | `storage:write` | decrements `refcount`; 204. |
| `GET` | `/api/v1/staas/{ns}/blobs` | `storage:read` | **list**, paginated. |
| `GET` | `/api/v1/staas/{ns}/usage` | `storage:read` | per-namespace metering (§4). |

- **Envelope**: JSON responses (`LIST`, `HEAD`, `usage`, errors) are wrapped only
  when the client opts in (`X-API-Envelope: 1` or `Accept: application/vnd.co.v1+json`),
  matching `wants_envelope()`. Binary `GET` bodies are **never** wrapped (the
  middleware already skips non-JSON). Version headers (`X-API-Version`,
  `X-Co-Server-Version`) are emitted on all `/api/v1/*` as today.
- **Pagination**: `?page=&page_size=` (default 100, max 1000). The list body carries
  `{ items, page, page_size, total }`; `meta()` in the envelope lifts `page/
  page_size/total` into `meta` automatically — no new mechanism.
- **Errors**: standard `{ code, message, field?, hint? }` objects from the envelope's
  error mapping. Quota/rate breaches → `429` (§5); cross-namespace access → `403`;
  oversize body → `413`.
- **Size limits**: per-object cap by tier (§5), default 1 MiB inline → larger
  rejected unless the tier allows large objects (mirrors CO-81's <1 MB inline rule).

---

## 3. Auth & least-privilege (CO-448)

Partner tokens are **API tokens** (`co-web/src/storage/api_tokens.rs`,
`api_tokens.scopes` JSON) carrying capabilities resolved by
`resolve_scopes()` and checked by `grants()`.

### New capabilities (proposed, CO-463)

Add to `KNOWN_CAPABILITIES` in `co-web/src/auth/capabilities.rs`:

```
storage:read     -- GET/HEAD/LIST within the bound namespace
storage:write    -- PUT/DELETE within the bound namespace
```

…and a `staas` bundle (`storage:read, storage:write`) alongside the existing
`read`/`write`/`admin`/`agent` bundles. Enforcement reuses the `Scoped<C>` extractor:
two new `Capability` impls (`StorageRead`, `StorageWrite`) with
`REQUIRED = "storage:read" | "storage:write"` and `ADMIN_SURFACE = false` (partner
tokens are non-escalating; `requires_admin_to_grant()` stays false).

### Namespace binding

A capability says *what* (read/write); the **namespace** says *where*. The token
must also be bound to a namespace so `storage:write` on partner A cannot touch
partner B. Two layers:

1. **Scope is necessary** (`grants("storage:write")`), and
2. **Binding is sufficient**: the token row carries its `namespace_id`(s); the
   handler injects that prefix and rejects any key resolving outside it (`403`).

This is the same "scope + owner-tier" split the `Scoped<C>` check already does for
universes (JWT = full authority; API token = `BTreeSet` membership + owner scope) —
StaaS adds the namespace dimension to the binding, not a new auth engine.

### Issue / revoke

- **Issue**: `create_api_token_with_scopes()` with `["staas"]` + namespace binding.
  Admin-only mint (a partner self-serve mint is CO-466, out of v1 scope).
- **Revoke**: existing token revocation (delete/expire the `api_tokens` row) —
  immediate, no new mechanism. Revoking the last token for a namespace freezes it;
  data stays until the namespace is deleted (§1).

---

## 4. Metering & billing hooks (CO-453)

**Definition of usage** (per namespace, per billing window):

```
storage_usage = Σ_over_time( bytes_resident × Δt )        -- "byte-hours"
operations    = count(PUT) + count(GET) + count(DELETE) + count(LIST)
egress_bytes  = Σ bytes streamed out by GET                -- optional, tiered
```

- `bytes_resident` is read from `blob_refs.size` summed over the namespace's
  `blob_namespace` rows (reference-bytes, not physical — see §1). A periodic sampler
  (the existing backup/worker cadence in `co-web/src/storage/backup/worker.rs` is a
  natural host) snapshots `Σ size` per namespace into a `usage_sessions`-style rollup.
- **Source**: the telemetry rollup tables behind CO-453 (`usage_sessions`,
  surfaced by `co-web/src/platform/telemetry_api.rs`). StaaS adds a
  `storage_usage` rollup keyed by `namespace_id` rather than `model` — same shape as
  `UsageTotals`/`UsageGroup`, queried like `/metrics/throughput`.
- **Per-partner `/usage`**: `GET /api/v1/staas/{ns}/usage` returns
  `{ namespace, window_seconds, bytes_resident, byte_hours, operations{put,get,delete,list}, egress_bytes }`,
  gated by `storage:read`. The admin/aggregate view reuses `Scoped<TelemetryRead>`
  on the telemetry router.
- **Billing hooks (measurement only — no payment gateway here)**: at window close,
  emit a `Event { event_type: "staas.usage.window_closed", payload: <the usage row> }`
  on the existing `EdaBus`. A downstream `EdaSubscriber` (out of scope) maps it to an
  invoice. The contract is: **CO emits metered events; billing is someone else's
  subscriber.**

---

## 5. Quota & rate-limit (CO-80)

Enforcement reuses `co-web/src/platform/rate_limit.rs` — `RateLimiter`,
`rate_limit_middleware`, `check_storage_quota()`, and the `Tier`/`TierLimits` model.

### Partner tiers (extend `TierLimits`)

`TierLimits` today holds `reads_per_min`, `writes_per_min`, `storage_entries`,
`max_universes`. StaaS adds **byte-based** quota fields (CO-465):

| Tier | reads/min | writes/min | stored bytes | max object | namespaces |
|------|-----------|-----------|--------------|-----------|-----------|
| `partner_free` | 600 | 60 | 1 GiB | 1 MiB | 1 |
| `partner_pro` | 6 000 | 600 | 100 GiB | 64 MiB | 10 |
| `partner_byob` | 6 000 | 600 | — (partner backend, §6) | 256 MiB | 25 |

- **Bytes**: a new `check_storage_bytes_quota(namespace, incoming_size)` mirrors the
  existing `check_storage_quota()` (which counts *entries*); over-cap `PUT` → `413`
  if single-object, `429`/`403` if cumulative-cap.
- **Rate**: the existing `TokenBucket` per-tier limiter applies per **partner token**
  (the limiter key becomes the token/namespace, not the IP) — drop-in, no new algo.
  `AbuseTracker` (CO-397) still backstops 401/404 storms.
- Quota state is read from the §4 rollup (`Σ blob_refs.size` for the namespace), so
  enforcement and metering share one number.

---

## 6. Backends de parceiro (bring-your-own)

A parceiro may **bring their own backend** — an S3-compatible bucket they own —
as an alternative `StorageBackend` impl (CO-458), selected per namespace. This is
the same swap the seam map already documents ("Per-universe storage … swap the
backend behind the trait", `COMPOSABILITY.md` row).

```
StorageBackend (trait, CO-458)
 ├── LocalFsBackend         -- CO's own disk (default; partner_free/pro)
 ├── CoManagedS3Backend     -- CO's S3/R2 (CO-81)  (partner_pro at scale)
 └── PartnerS3Backend       -- partner's own bucket+creds (partner_byob)
```

- **Config per partner**: a `namespaces.backend` column (§1) selects the impl;
  partner S3 credentials live behind `SecretsProvider`
  (`co-web/src/infra/secrets.rs`), **never** in `blob_refs` or the DB in plaintext.
- **Trust boundary**: with `partner_byob`, **bytes leave CO's storage**. CO still
  meters operations and the `blob_refs` metadata it brokers, but cannot guarantee
  durability of partner-controlled bytes (so backup §7 is the partner's
  responsibility for BYOB). Auth, namespacing, and quota (op/rate) still apply at
  the CO API boundary; only the *bytes* sit in the partner bucket. Document this
  explicitly in the partner agreement.
- A BYOB backend that fails (creds revoked, bucket gone) surfaces as `502` through
  the envelope's error mapping; CO does not retry into a foreign bucket beyond the
  `StorageBackend` impl's own policy.

---

## 7. Durabilidade & backup por namespace (CO-459 / CO-81)

- **CO-managed namespaces** (LocalFs / CoManagedS3) are covered by the existing
  snapshot mechanism: `BackupBackend` (`co-web/src/storage/backup/mod.rs`),
  `LocalFsBackend` local snapshots, and the CO-81 `S3Backend` off-machine replica.
  The snapshot already walks `universes/` + blobs; StaaS namespaces are an
  additional subtree the `build_snapshot()` walk includes. Per-namespace restore =
  extract that subtree from a snapshot tarball.
- **Retention**: reuse `CO_BACKUP_RETENTION_DAYS` / `CO_BACKUP_INTERVAL_HOURS`.
  Premium tiers may pin a longer retention (a `namespaces.retention_days` override).
- **BYOB namespaces** (§6): durability is the **partner's** responsibility — CO
  backs up only the metadata (`blob_refs`, `namespaces`, `blob_namespace`), which is
  enough to re-attach a recovered partner bucket. This boundary must be stated in the
  tier description and the partner agreement.
- Backup runs **per deployment**, not per request, so it does not enter the partner
  API hot path (no lock held across the snapshot — respects the
  "never panic/block under `Mutex<Storage>`" rule).

---

## 8. Roadmap de impl (decomposição CO-N)

Sequenced after CO-458 lands (`StorageBackend` + `blob_refs` migration v086). Each
row lists its **dependency** and **what it migrates** (DB). Numbers are proposals to
be minted; confirm `max+1` migration version at claim time against `origin/main` +
open PRs (per the migration-claim protocol in the dev guide).

| Task | Title | Depends on | Migrates (DB) | Delivers |
|------|-------|-----------|---------------|----------|
| **CO-461** | Namespaces schema + tenancy join | CO-458 | `namespaces`, `blob_namespace` tables (new migration) | partner/namespace model; prefix injection helper |
| **CO-462** | Partner storage API (`/api/v1/staas/{ns}/blobs…`) | CO-461, CO-456 | none (routes only) | `PUT/GET/HEAD/DELETE/LIST` + envelope + pagination + size limits |
| **CO-463** | Storage capabilities + namespace binding | CO-462, CO-448 | `api_tokens` binding (add `namespace_id` link, migration) | `storage:read/write`, `staas` bundle, `Scoped<StorageRead/Write>`, issue/revoke |
| **CO-464** | Metering rollup + `/usage` | CO-461, CO-453 | `staas_usage` rollup table (migration) | byte-hours+ops sampler, per-ns `/usage`, `staas.usage.window_closed` event |
| **CO-465** | Quota & rate-limit per tier | CO-461, CO-80 | `TierLimits` byte fields (migration if persisted) | `partner_*` tiers, `check_storage_bytes_quota()`, per-token bucket |
| **CO-466** | Partner backend (BYOB S3) | CO-461, CO-458, CO-81 | `namespaces.backend` + creds ref (migration) | `PartnerS3Backend` impl, per-ns config, trust-boundary docs |
| **CO-467** | Per-namespace backup/restore | CO-461, CO-459, CO-81 | none (extends snapshot walk) | namespace-scoped snapshot include + restore; retention override |

**Critical path**: CO-458 → CO-461 → CO-462 → CO-463 (a usable, authed, single-tier
StaaS). Metering (464), quota (465), BYOB (466), backup (467) layer on independently
after CO-461.

---

## Flow diagram — parceiro → token → namespace → backend → metering

```mermaid
flowchart LR
    P[Parceiro] -->|"API token (storage:write,<br/>bound to ns/acme/prod)"| MW

    subgraph CO[co-web /api/v1]
      MW["envelope_middleware<br/>+ rate_limit_middleware<br/>(CO-456 / CO-80)"]
      AUTH["Scoped&lt;StorageWrite&gt;<br/>grants('storage:write')<br/>+ namespace binding (CO-448/463)"]
      H["staas handler<br/>PUT /staas/{ns}/blobs/{key}"]
      Q{"check_storage_bytes_quota<br/>(CO-80/465)"}
      MW --> AUTH --> H --> Q
    end

    Q -->|over cap| ERR["413 / 429<br/>envelope error"]
    Q -->|ok| NSP["prefix inject:<br/>ns/acme/prod/{key}"]

    NSP --> SB{"StorageBackend (CO-458)"}
    SB -->|"LocalFs / CoManaged S3"| LFS[("CO storage<br/>blob_refs.size, refcount")]
    SB -->|"partner_byob"| PS3[("Partner S3 bucket<br/>(trust boundary)")]

    LFS --> MET["metering sampler (CO-453/464)<br/>Σ blob_refs.size × Δt + ops"]
    PS3 -->|ops only| MET
    MET --> USAGE["GET /staas/{ns}/usage"]
    MET -->|"window close"| BUS["EdaBus: staas.usage.window_closed<br/>→ billing subscriber"]

    LFS -.->|snapshot (CO-459/81)| BK[("Backup: LocalFsBackend / S3Backend")]
```

---

## Não no escopo (restated)

- No route/handler implementation — **design only** (routes are specced for CO-462+).
- No real payment gateway — only the **measurement** points and the
  `staas.usage.window_closed` hook (§4).
- No change to `StorageBackend` — CO-458 defines it; here we only consume it on paper.
