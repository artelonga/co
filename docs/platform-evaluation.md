# Platform Evaluation — Vercel · Cloudflare · Fly.io · AWS Fargate

> Comparative analysis across four scenarios (video-social, service-catalog, dropshipping-logistics, CO itself) using a repeatable evaluation method. Pricing snapshots taken **2026-04-30** from official documentation; verify before contractual decisions.

---

## 0. Methodology — repeatable evaluation process

Run this whenever you re-evaluate a platform (suggested cadence: every 6 months, or on a 2× pricing change).

### 0.1 Score each platform on seven dimensions (1–5)

| Dimension | What to measure | How to test |
|-----------|-----------------|-------------|
| **Compute fit** | Does the runtime model match the workload (long-lived process? request-scoped? actor-per-tenant?) | Read the platform's "limits" page; map each workload op to a runtime primitive. |
| **Storage fit** | Object storage, relational, KV, queues, durable streams — does the catalogue cover the data shapes you need? | Inventory data shapes; check first-party offerings vs. bring-your-own. |
| **Egress economics** | $/GB out at the bandwidth volume you'll actually push | Estimate monthly GB egress at p50 and p95; multiply by the platform's price. |
| **Operational floor** | Cost of the smallest viable always-on production deployment | Reproduce the "hello world prod" SKU and read the bill. |
| **Scaling ceiling** | Headroom before you hit a hard limit (RPS, body size, exec time, concurrent connections) | Read documented limits; cross-reference with status-page incident history. |
| **Lock-in / exit cost** | Engineering days to migrate off | Identify proprietary primitives in your design (Durable Objects, Workflows, RDS schema, etc.) and estimate rewrites. |
| **Developer experience** | Local dev parity, deploy time, observability | Ship a non-trivial sample app end-to-end; time it. |

### 0.2 Form a falsifiable hypothesis per scenario

Pattern: **"For workload _W_ at scale _S_, platform _P_ delivers metric _M_ at cost _C_, beating the next candidate by _Δ_."** The hypothesis must be testable with public pricing + a benchmark; if it can't be falsified, sharpen it.

### 0.3 Run a small empirical probe

For each finalist, deploy a representative slice (one endpoint, 1k seeded rows, 10MB+ asset) and capture:

- Cold-start latency (1st req), warm latency (p50/p95/p99 over 1k req)
- Egress cost at expected fan-out (use platform's billing console, not estimates)
- Time to deploy, time to roll back
- Time to local-dev a regression

The probe is the source of truth — vendor marketing isn't.

### 0.4 Decide on cost _at your traffic_, not headline price

Bandwidth, request count, and storage class shape the bill far more than the per-vCPU rate. Build the unit economics before the recommendation.

---

## 1. Platform profiles (one-paragraph each)

**Vercel** — Serverless compute (Functions / Edge Functions / Fluid Compute) tightly coupled to Next.js, layered on AWS + Cloudflare. Optimized for JAMstack and ISR. No WebSocket-server support in Functions ([Vercel/Limits §WebSockets](https://vercel.com/docs/limits)). Pro plan: 1 TB included Fast Data Transfer, max function duration 300s, 1M invocations included; on-demand from there.

**Cloudflare** — Workers (V8 isolates, ~5ms cold start), Durable Objects (single-leader actors with optional SQLite backend, GA), R2 (S3-compatible object storage with **$0 egress**), D1 (SQLite at the edge), Stream (end-to-end video pipeline), Queues, Pages, Workers Analytics Engine. Compute model is request-scoped + actor-scoped, not long-lived process. ([CF/R2 pricing](https://developers.cloudflare.com/r2/pricing/), [CF/DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/))

**Fly.io** — Firecracker microVMs from a Docker image, deployed regionally (35+ regions). Long-lived processes, autostart/autostop, attached NVMe Volumes ($0.15/GB-mo), Tigris S3-compat object store, LiteFS for SQLite replication, managed Postgres. shared-cpu-1x at **$2.02/mo** always-on, performance-1x at $32.19/mo. Egress $0.02/GB NA+EU, $0.04/GB APAC+SA, $0.12/GB Africa+India. ([Fly/pricing](https://fly.io/docs/about/pricing/))

**AWS Fargate** — Serverless containers on ECS/EKS. Linux/x86: $0.0405/vCPU-hour + $0.00444/GB-RAM-hour. **Graviton (ARM) ~20% cheaper.** Spot up to 70% off. Pairs with the full AWS suite (S3, RDS, DynamoDB, MediaConvert, Kinesis, CloudFront, KMS, Step Functions). Highest ceiling, highest operational complexity. Egress at standard AWS rates ($0.09/GB to internet for first 10TB, decreasing tiers). ([AWS/Fargate pricing](https://aws.amazon.com/fargate/pricing/))

---

## 2. Scenario A — Video social media (TikTok-shape)

**Workload profile.** Users CRUD short videos (10s–3min) plus posts/comments/likes. Heavy upload, transcoding, global delivery, recommendation feed. Assume 100k MAU, 5M video views/mo, 50TB egress/mo, 5TB storage growth/mo.

**Hypothesis (falsifiable).** _For a video-CRUD social workload at 50TB/mo egress, **Cloudflare (Stream + R2 + Workers + Durable Objects) delivers a sub-$2k/mo blended bill at <150ms TTFB globally**, beating the next-best (Fly+Tigris) by ≥3× on egress alone and beating Fargate+S3+CloudFront by ≥5×._

### 2.1 Platform-by-platform rationale

| Platform | Verdict | Why |
|----------|---------|-----|
| **Vercel** | ✗ Disqualified | Functions don't host WebSockets; max body ~4.5MB Hobby / configurable but not video-friendly; no first-party video pipeline; bandwidth at $0.15+/GB after 1TB Pro included. ([Limits §WebSockets, §Static File uploads](https://vercel.com/docs/limits)) |
| **Cloudflare** | ✓ **Best fit** | R2 storage $0.015/GB-mo + **$0 egress** is the structural advantage. Stream handles ingest → HLS/DASH transcode → adaptive delivery in one product. Durable Objects model the per-video comment thread / like counter with strong consistency. Workers Analytics Engine captures playback events. ([R2 pricing](https://developers.cloudflare.com/r2/pricing/)) |
| **Fly.io** | △ Possible, costly | You'd run FFmpeg in performance-cpu Machines and store on Tigris. Egress at $0.02/GB × 50TB = **$1,000/mo just in bandwidth**, plus compute. Workable but you build the pipeline yourself. |
| **AWS Fargate** | △ Mature, expensive | S3 + MediaConvert + CloudFront is the canonical stack. CloudFront egress ~$0.085/GB at first tier × 50TB = **~$4,250/mo egress** (plus origin pulls, plus MediaConvert minutes). Most flexible, least cheap. |

### 2.2 Empirical probe (recommended)

1. Upload a 50MB MP4, record end-to-end transcode time per platform.
2. Stream it 1,000 times from 5 geographies; measure p50/p95 TTFB.
3. Pull the actual bill for the 1,000-stream test. Multiply to your projected scale.

**Expected result confirming hypothesis:** R2/Stream egress line item is $0; comparable AWS bill shows 4-figure egress at the same fan-out.

### 2.3 Citations
- R2 zero egress: https://developers.cloudflare.com/r2/pricing/
- Cloudflare Stream: https://developers.cloudflare.com/stream/
- AWS data-transfer pricing: https://aws.amazon.com/ec2/pricing/on-demand/#Data_Transfer

---

## 3. Scenario B — Service catalog (mostly static, moderate)

**Workload profile.** Marketing site + service catalogue with detail pages. ~100k pageviews/mo, infrequent CRUD by ops team, near-zero write contention, all data fits in a static site + small CMS API.

**Hypothesis.** _For a static-leaning catalogue at 100k pageviews/mo, **Cloudflare Pages + Workers (or Vercel Hobby/Pro) delivers <$25/mo total** with sub-100ms global TTFB. Fly and Fargate are 5-10× over-provisioned for this shape._

### 3.1 Platform-by-platform rationale

| Platform | Verdict | Why |
|----------|---------|-----|
| **Vercel** | ✓ Best DX | Next.js / SvelteKit / static + ISR + a few API routes is exactly the product. Pro $20/seat-mo + 1TB included transfer covers this with headroom. Image optimization + Web Analytics first-party. |
| **Cloudflare Pages** | ✓ Best cost ceiling | Equivalent DX, no per-seat tax, Workers free tier (100k requests/day) typically covers an internal catalogue API. R2 for any media. |
| **Fly.io** | ✗ Wrong shape | Long-lived VM for a static site burns money (~$2/mo minimum, but with no benefit over edge). Reasonable only if you already run Rust/Go services and want one platform. |
| **AWS Fargate** | ✗ Over-engineered | A container cluster for a brochure site is operational malpractice. S3 + CloudFront is the AWS-native answer; Fargate isn't. |

### 3.2 Empirical probe

1. Deploy a 50-page Astro/Next/SvelteKit static site with one `/api/contact` route.
2. Time first deploy, redeploy, rollback.
3. Run Lighthouse from 3 geographies. Compare TTFB.
4. Read the bill at the end of month 1.

### 3.3 Citations
- Vercel Pro included transfer: https://vercel.com/docs/limits §Included usage
- Cloudflare Pages free tier: https://developers.cloudflare.com/pages/platform/limits/

---

## 4. Scenario C — Dropshipping logistics (Uber-shape state)

**Workload profile.** Each order is a state machine (placed → backordered → fulfilled → shipped → tracked → delivered). Conflicting writes from supplier APIs, courier webhooks, customer cancellations. Real-time status push to customer; inventory contention; geo-tracked deliveries. Strong-consistency requirement on order state; eventual on inventory aggregates.

**Hypothesis.** _Per-order **single-writer actors** are the right primitive for this workload. **Cloudflare Durable Objects (one-DO-per-order) makes ordering and conflict-resolution trivial**, beating shared-database approaches (Fly+Postgres, Fargate+RDS) on tail latency at write contention while costing ≈ $0.20/M ops + $12.50/M GB-s of activity ([CF/DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/))._

### 4.1 Platform-by-platform rationale

| Platform | Verdict | Why |
|----------|---------|-----|
| **Vercel** | ✗ Disqualified | Stateless serverless — no native actor model, no WebSocket server, no built-in queue with durable consumers. You'd bolt on Upstash + a third-party realtime layer; you've now reinvented the stack on a more expensive base. |
| **Cloudflare** | ✓ **Best fit** | Durable Objects = one logical actor per order, single-writer per object, strong consistency, optional SQLite backend per object (5 GB-mo included, $0.20/GB-mo after). Queues for supplier-webhook fan-in. Workers for HTTP+webhooks. R2 for invoices/labels. |
| **Fly.io** | ✓ Strong alternative | Postgres + advisory locks (or `SELECT … FOR UPDATE`) gives correctness; Phoenix/SvelteKit WebSockets handle realtime push. Operationally you own connection pooling, replica failover, sharding when one DB stops scaling. |
| **AWS Fargate** | ✓ Mature | DynamoDB single-table + conditional writes + Step Functions for the state machine + EventBridge + ECS for HTTP. Maximum ceiling. Highest engineering tax. Justified at 7-figure GMV. |

### 4.2 Concurrency model comparison (the part that decides correctness)

| Approach | Conflict resolution | Failure mode |
|----------|---------------------|--------------|
| **Single-writer actor** (Durable Object) | Linearizable per actor; conflicts impossible by construction within one order | Object hot-spot if many writes target same order |
| **OCC on RDBMS** (Fly+Postgres / RDS) | Optimistic version columns, retry on conflict | Lock convoy under contention; migration pain when sharding |
| **DynamoDB conditional writes** | Conditional `UpdateItem` on version attr | Throttling at hot partitions; 400KB item limit |

### 4.3 Empirical probe

1. Simulate 1,000 concurrent updates against the same `order_id` (50/50 supplier-webhook + customer-cancel).
2. Measure: % linearizable, p99 commit latency, $ cost.
3. Repeat at 10× scale; observe whether tail latency stays bounded.

### 4.4 Citations
- DO consistency model: https://developers.cloudflare.com/durable-objects/concepts/what-are-durable-objects/
- DO SQLite: https://developers.cloudflare.com/durable-objects/best-practices/access-durable-objects-storage/
- DynamoDB conditional writes: https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/Expressions.ConditionExpressions.html

---

## 5. Scenario D — CO itself (recommendation)

**Workload profile (synthesized from C0-* tickets and CLAUDE.md).**

- Task management + content storage, **multi-tenant by universe** (one universe = one logical workspace; today implemented as SQLite + tree of `.md` files).
- Catalogue/portfolio surfaces (high-read-fan-out per universe — like Scenario B).
- Real-time CRUD with conflict resolution: **Apple file-dialog UX** (`Ignore / Replace / Keep both / Apply to all`), **Jujutsu-style commit/changelog** rendering.
- Throughput: thousands of concurrent CRUD ops across universes from conflicting sources (web + desktop + future mobile).
- E2E encryption: data encrypted at rest + in transit, **decryptable only with user token** (browser cache lifetime; desktop one-week tokens — to be tightened later).
- Telemetry, real-time analytics, A/B testing — both centralized and per-universe parallelizable.
- Stack today: Rust (Axum core + co-cli), SvelteKit web frontend, Fly.io for the API + LiteFS-replicated SQLite per universe.

**Hypothesis.** _CO's compute belongs on **Fly.io** (Rust binary, regional Machines, LiteFS) because the existing investment is sound and the runtime model fits long-lived `axum` servers. The **storage and edge layer** should move to **Cloudflare** (R2 for content blobs, Workers Analytics Engine for telemetry, Pages for the static frontend, Cloudflare in front as CDN/cache). The **conflict-resolution engine** is an in-process concern (jujutsu-like merge inside `co-core`), not a hosting concern — pick the runtime that runs Rust well, pick the storage that doesn't tax egress._

### 5.1 Why hybrid (Fly compute + Cloudflare storage/edge) beats each pure option

| Pure option | Why it loses |
|-------------|--------------|
| **Pure Vercel** | No Rust runtime path, no WebSocket server, function timeouts kill long sync sessions. ([Limits](https://vercel.com/docs/limits)) |
| **Pure Cloudflare** | Rust runs on Workers only via Wasm; loses native `rusqlite`, FFmpeg, native crypto; constrains the entire `co-core` design. Acceptable for greenfield, prohibitive for a 1.21.x codebase. |
| **Pure Fly** | Egress at $0.02/GB NA+EU is fine until the catalogue/portfolio side blows up; no first-party analytics/A-B; you'd build telemetry pipelines yourself. |
| **Pure Fargate** | Adds AWS operational tax (IAM, CDK/Terraform, VPC, KMS, CloudWatch) without any feature CO actually needs that Fly doesn't already provide. |

### 5.2 Hybrid topology (recommended)

```
                         ┌─────────────────────────────┐
                         │   Cloudflare CDN + Pages    │ ← SvelteKit frontend (already
                         │     (static, $0 egress)     │   buildable; deploy from CI)
                         └────────┬────────────────────┘
                                  │ /api/*  (fetch)
                                  ▼
        ┌─────────────────────────────────────────────────────┐
        │   Fly.io regional Machines  ── co-web (Rust/Axum)   │
        │   ─ LiteFS-replicated SQLite (per universe)         │
        │   ─ Volumes for hot path ($0.15/GB-mo)              │
        │   ─ Conflict resolver = jujutsu-like merge (in-proc)│
        └────────┬─────────────────────┬──────────────────────┘
                 │                     │
                 ▼                     ▼
   ┌─────────────────────┐   ┌────────────────────────────┐
   │ Cloudflare R2       │   │ Cloudflare Workers         │
   │ encrypted blobs     │   │ Analytics Engine           │
   │ (client-side AEAD   │   │ (real-time telemetry,      │
   │  before upload)     │   │  A/B exposure events)      │
   │ $0.015/GB · $0 egr. │   │ time-series, queryable     │
   └─────────────────────┘   └────────────────────────────┘
```

### 5.3 Mapping requirements → topology decisions

| CO requirement | Where it lives | Why |
|----------------|----------------|-----|
| Per-universe SQLite + Rust core | **Fly Machines + LiteFS** | Already shipping; Rust-native; LiteFS gives single-leader replication ≈ DO-with-SQLite shape but on a process you control. |
| Conflict resolution (Apple-dialog UX, jujutsu changelog) | **Inside `co-core`** (Rust) | Hosting-neutral. Implement as a merge engine over the universe's commit DAG; UI surfaces the 4 choices + "apply to all" via WebSocket from `co-web`. |
| Content blobs (markdown, images, video) | **R2** with **client-side encryption** | $0 egress is decisive at portfolio/catalogue scale. Server never sees plaintext; satisfies "decrypt only with user token". |
| Telemetry, real-time analytics, A/B | **Workers Analytics Engine** | Time-series ingestion is the right shape; cheap; queryable from Workers. Centralized + per-universe via tags. |
| Static frontend | **Cloudflare Pages** (or stay on Fly serving static) | Pages is free at this scale; closer to user; orthogonal to API failure domain. |
| Anonymous/UAT/admin auth | **Stay on Fly** (current `co-web`) | All session/cookie/JWT logic is in Rust today; no win moving it. |
| Background jobs (transcode, indexing) | **Fly machines (autostart)** or **Cloudflare Queues + Workers** | Start with Fly autostart; move to CF Queues if/when fan-out demands it. |

### 5.4 Encryption design (the load-bearing detail)

> "Private and safe at rest and in transit, decrypt only with user token."

Three viable schemes; pick exactly one and document it:

1. **Per-universe symmetric key, wrapped by user token.**
   Each universe has a content key `K_u`. `K_u` is wrapped under a key derived from the user's session token (Argon2id KDF). Browser holds the unwrapped key in `sessionStorage` for the cache lifetime; desktop holds it under OS keyring with a 7-day TTL. Server stores only wrapped `K_u`. Content blobs uploaded to R2 are encrypted under `K_u` with AES-256-GCM (per-blob nonce). **This is the recommended scheme** — simplest, supports multi-device, supports key rotation by re-wrapping.

2. **Per-blob keys, hierarchical KMS.** More flexible, more complex. Defer until needed.

3. **Server-side encryption only.** Rejected: server can decrypt, violates the requirement.

Implications for the design:
- Server never sees plaintext content → **server-side full-text search is impossible** without compromising. Either accept (search runs on the client over decrypted local cache) or use deterministic encryption / encrypted indexes (much harder; out of scope for v1).
- A/B exposure events and telemetry payloads must be **scrubbed of plaintext content** before reaching Workers Analytics Engine. Send IDs, durations, types — not body text.

### 5.5 Migration / rollout plan

1. **Phase 0 (1–2 weeks).** Add R2 bucket; introduce client-side AEAD on uploads behind a feature flag. Server treats R2 objects as opaque; SQLite stores only metadata + R2 key. Verify integrity via end-to-end round-trip test in CI.
2. **Phase 1.** Migrate static frontend from Fly-served to Cloudflare Pages; put Cloudflare in front of `co-artelonga.fly.dev` as CDN/cache.
3. **Phase 2.** Wire Workers Analytics Engine for `/api/*` exposure events. Keep existing Fly logs for raw runtime logs.
4. **Phase 3.** Backfill existing content into R2 (one universe at a time; reversible).
5. **Phase 4 (only if needed).** Evaluate moving per-universe coordination from LiteFS to Durable Objects. Trigger: LiteFS write throughput becomes a bottleneck or operational cost of replica management exceeds DO pricing.

### 5.6 Rejected alternatives (write these down so future-you doesn't re-litigate)

- **"Move CO to pure Cloudflare Workers."** Rust→Wasm loses native crates; rewriting `co-core` is ~quarters of work for marginal latency gain.
- **"Move CO to Fargate."** Adds AWS operational surface (IAM, CDK, VPC, etc.) to solve problems Fly already solves. Justified only if you hit Fly multi-region capacity limits or need AWS-only services (e.g., Bedrock private endpoints).
- **"Stay 100% on Fly, including content blobs."** Tigris egress is $0.02/GB NA+EU. At 1TB/mo egress (catalogue load), that's $20/mo extra vs $0 on R2. Trivial today; matters at 100TB.

---

## 6. Synthesis matrix

|  | Vercel | Cloudflare | Fly.io | Fargate |
|--|--------|------------|--------|---------|
| Video social (Scenario A) | ✗ | **★★★** | ★★ | ★★ |
| Service catalog (Scenario B) | ★★★ | **★★★** | ★ | ✗ |
| Dropshipping logistics (Scenario C) | ✗ | **★★★** | ★★ | ★★ |
| **CO (Scenario D)** | ✗ | **★★★ (storage/edge)** | **★★★ (compute)** | ✗ |

Recommendation summary:

- **Default to Cloudflare** for any new greenfield workload that touches video, real-time state, or high-egress content delivery.
- **Default to Vercel** for static/marketing/JAMstack with a Next/SvelteKit team and modest scale.
- **Default to Fly.io** when the workload is a long-lived process in Rust/Go/Elixir with regional latency requirements.
- **Default to Fargate** only when you're already deep in the AWS ecosystem and need a specific AWS-only service.
- **For CO: hybrid Fly (compute) + Cloudflare (storage/edge/analytics).**

---

# Part II — Data layer (Iceberg-anchored)

The "where do you run code" decision (Part I) is orthogonal to the "where does data live and how does it move" decision. Iceberg is the spine: pick engines and stream sources by how cleanly they read/write Iceberg tables through a shared catalog.

## 8. Taxonomy — what each piece does

| Layer | Purpose | Latency band | Examples |
|-------|---------|--------------|----------|
| **Cache / KV** | Hot reads, sessions, rate limits, pub-sub fan-out | µs–ms | Redis, Valkey, KeyDB, Cloudflare KV/Cache |
| **Event log** | Durable, ordered, replayable stream of facts | ms–s | Kafka, Redpanda, Pulsar |
| **Stream compute** | Stateful per-event processing, sessionization, joins | ms–s | Flink, Kafka Streams, Spark Structured Streaming |
| **Batch compute** | Wide-scope aggregations, backfills, ML feature gen | min–hr | Spark, Trino (interactive), DuckDB (small) |
| **Real-time OLAP** | Sub-second queries on freshly-ingested data | <1s | Apache Pinot, ClickHouse, Apache Druid, StarRocks |
| **Table format** | The contract the layers above agree on | — | **Apache Iceberg**, Delta, Hudi |
| **Catalog** | Names tables, tracks snapshots, governs writes | — | REST (Polaris, Lakekeeper, Tabular-style), Glue, Nessie, Hive, JDBC |

The Iceberg constraint **filters the catalog and engine choices**, but is mostly irrelevant to the cache layer. Redis stays Redis.

---

## 9. Cache / KV — Redis vs Valkey

**Hypothesis.** _Use **Valkey** (the BSD-3 Linux Foundation fork of Redis 7.2.4) for any new deployment. Redis Inc.'s 2024 license change to SSPL/RSALv2 makes Redis a vendor-managed product, not OSS infrastructure. Valkey is API-compatible, drop-in, and now has Linux-Foundation governance with AWS / Google / Oracle / Alibaba contributing._

| Concern | Redis (post-2024) | Valkey |
|---------|-------------------|--------|
| License | SSPL / RSALv2 | BSD-3 (LF Valkey project) |
| API compatibility | Native | 100% drop-in for Redis 7.2.4 baseline; diverges over time |
| Managed offerings | Redis Cloud, AWS ElastiCache (still on permissive Redis), Upstash | AWS ElastiCache for Valkey (cheaper than Redis SKU), Google Memorystore for Valkey |
| Iceberg relevance | None — caches sit in front of stores | None |

**Where it fits CO.** Session rate limiting, hot universe metadata, WebSocket presence/fan-out across multiple Fly machines. Single small Valkey instance is enough for the foreseeable future.

**Citations.**
- Redis license change: https://redis.io/blog/redis-adopts-dual-source-available-licensing/
- Valkey project: https://valkey.io/

---

## 10. Event log — Redpanda vs Kafka (with Tableflow / Iceberg Topics)

**Hypothesis.** _For any new event log where Iceberg is a destination, **Redpanda Iceberg Topics or Confluent Tableflow** eliminate the Kafka-Connect-Iceberg-sink hop. Pick **Redpanda** for operational simplicity and cost ($0 JVM, single binary, no ZK), pick **Confluent + Tableflow** if you're already standardized on Confluent Cloud and need broader catalog coverage (Glue + Polaris + Unity + OneLake)._

### 10.1 Architecture comparison

| Property | Apache Kafka | Redpanda | Confluent Cloud (Kafka + Tableflow) |
|----------|--------------|----------|-------------------------------------|
| Runtime | JVM, KRaft (no ZK in 4.0+) | C++/Seastar, single binary | Managed Kafka |
| Iceberg path | Kafka Connect + Iceberg Sink (community Apache Iceberg connector or Confluent's) | **Native Iceberg Topics** — topic data lands directly as Iceberg Parquet | **Tableflow GA** — topic → Iceberg/Delta with zero ETL |
| Iceberg file format | Sink-dependent (Parquet/ORC/Avro) | Parquet only | Parquet (Iceberg) / Parquet (Delta) |
| Catalog support | Whatever your sink supports | REST + object-storage catalog (incl. Glue via REST) | Glue, Snowflake Open Catalog (Polaris), Unity, OneLake |
| Schema evolution | Sink-dependent | Per Iceberg spec; auto-syncs from Schema Registry | Schema Registry-driven |
| Operational tax | Highest (run brokers, sinks, registry) | Lowest (one binary; no JVM, no ZK) | None (fully managed; pay for it) |
| License/source | Apache 2.0 | BSL with conversion to Apache 2.0 after 4 years (Redpanda Community); Enterprise add-ons proprietary | Proprietary managed service over OSS Kafka |

### 10.2 Known limitations to design around

**Redpanda Iceberg Topics** (per [docs.redpanda.com](https://docs.redpanda.com/current/manage/iceberg/about-iceberg-topics/)):
- Cannot **append** to a pre-existing non-Redpanda-created Iceberg table — Redpanda owns the table lifecycle.
- **No backfill** of existing topic data when you turn the integration on. Switch it on at topic creation, or re-key.
- **Parquet only**.
- JSON schemas supported from 25.2+.
- **CPU overhead is non-trivial** during translation; plan for cluster headroom.

**Confluent Tableflow** (per [confluent.io/product/tableflow](https://www.confluent.io/product/tableflow/)):
- GA, broader catalog coverage, "few clicks" UX. Tradeoff: Confluent Cloud pricing.

**Plain Kafka + Iceberg Sink Connector**:
- More moving parts; you own delivery semantics (the Apache Iceberg Kafka Connect connector targets exactly-once via two-phase commit but requires careful config).

### 10.3 When each wins

- **Self-host Redpanda** if you want one process, native Iceberg, cheapest TCO, and you're OK being on a single vendor's BSL terms.
- **Confluent Cloud + Tableflow** if you're already on Confluent or you need to land into multiple lakehouse catalogs (Snowflake + Databricks + Glue) without bespoke sinks.
- **Plain Kafka (MSK / Strimzi / self-host) + Iceberg Sink** if you must stay 100% Apache-licensed and accept the operational tax.

---

## 11. Stream / batch compute — Flink vs Spark

**Hypothesis.** _**Flink** wins for stateful sub-second stream processing (sessionization, online feature computation, real-time joins). **Spark** wins for batch ETL and ML training over Iceberg snapshots. They are complements, not alternatives — at non-trivial scale you run both._

### 11.1 Head-to-head

| Concern | Apache Flink | Apache Spark |
|---------|--------------|--------------|
| Primary model | True streaming (event-at-a-time) | Micro-batch (Structured Streaming) + first-class batch |
| Stateful streaming | Best-in-class — RocksDB state backend, exactly-once via checkpoints, savepoints for upgrades | Workable but coarser; checkpoint granularity is the micro-batch |
| Iceberg integration | First-class via `flink-iceberg` (read + write, streaming + batch reads, writes via `IcebergSink`) | First-class via `iceberg-spark-runtime` (read + write, time-travel, MERGE INTO, CALL procedures for maintenance) |
| Latency | 10–100ms achievable | Seconds (micro-batch interval) |
| Throughput at low latency | Higher | Lower at the same latency target |
| ML / SQL ergonomics | SQL good, ML weak | Excellent for both (Spark MLlib, ANSI SQL) |
| Operational complexity | High (JobManager + TaskManagers + state backend tuning) | High but more familiar tooling |

### 11.2 Picking one (when you must)

- **Workload = "real-time fraud / personalization / session-stitching":** Flink.
- **Workload = "nightly aggregations + backfills + train models":** Spark.
- **Workload = "land events into Iceberg every 5 min, then BI dashboards":** Either Spark Structured Streaming or Flink — Flink if the 5 min becomes "30 seconds" later.

### 11.3 Citations
- Iceberg engine support: https://iceberg.apache.org/docs/latest/ ("Spark, Trino, PrestoDB, Flink, Hive and Impala")
- `flink-iceberg`: https://iceberg.apache.org/docs/latest/flink/
- `iceberg-spark-runtime`: https://iceberg.apache.org/docs/latest/spark-getting-started/

---

## 12. Real-time OLAP — Apache Pinot (and the Druid / ClickHouse alternative)

**Hypothesis.** _When the requirement is **sub-second user-facing analytics on freshly-ingested data** (A/B test panels, live feed dashboards, per-user analytics), **Pinot** is the canonical choice; ClickHouse is the strong cost-leader for similar shapes; Druid is the third option. All three can read Iceberg in 2025-2026 — Pinot via the iceberg connector, ClickHouse via its `Iceberg` table function, Druid via its Iceberg ingestion task._

### 12.1 Pinot vs ClickHouse vs Druid (capsule)

| Property | Apache Pinot | ClickHouse | Apache Druid |
|----------|--------------|------------|--------------|
| Sweet spot | High-QPS user-facing aggregations on segmented data (used at LinkedIn, Uber, Stripe) | Wide-table analytics, log/observability, ad-hoc SQL | Time-series + slice-and-dice with rollups |
| Real-time ingest | Kafka / Pulsar / Kinesis native; segment commit ~1s | Kafka engine + ReplicatedMergeTree; seconds | Kafka indexing service; seconds |
| Iceberg support | Yes (read/external table) — exact connector path varies by version; check current Pinot docs | Yes — `iceberg(...)` table function reads Iceberg directly | Yes via Iceberg ingestion tasks |
| Catalog support | REST, Glue (via REST), Hive, Hadoop | REST, Glue | REST, Glue, Hive |
| OLTP-ish point lookups | Excellent (star-tree indexes) | OK | OK |
| Operational footprint | Heavy (Controller / Broker / Server / Minion) | Lighter (single binary, replicated) | Heavy (Coordinator / Overlord / Broker / Historicals) |

**Note on Pinot Iceberg specifics:** the canonical Pinot doc URL has moved across versions; the Iceberg integration exists but the exact path/connector should be verified against the current Pinot release at https://docs.pinot.apache.org/sitemap.md — flagged here because a stale doc reference will mislead.

### 12.2 When each wins

- **Pinot:** consumer-facing analytics with thousands of QPS, low-latency p99 over segmented dimensions (e.g., "show this user their last-30-day stats").
- **ClickHouse:** lower-ops self-host, rich SQL, observability/log analytics, cost-sensitive.
- **Druid:** entrenched at a shop already running it; less compelling for greenfield in 2026.

### 12.3 Citations
- Pinot docs: https://docs.pinot.apache.org/ (sitemap: https://docs.pinot.apache.org/sitemap.md)
- ClickHouse Iceberg: https://clickhouse.com/docs/en/engines/table-engines/integrations/iceberg
- Druid Iceberg: https://druid.apache.org/docs/latest/development/extensions-contrib/iceberg

---

## 13. Iceberg + catalog — pick the contract first

**Hypothesis.** _For multi-engine setups (Spark **and** Flink **and** Pinot **and** Trino reading the same tables), the **REST catalog spec** is the only choice that gives clean interop across all of them. Pick a REST catalog implementation; treat it as the load-bearing decision._

### 13.1 Catalog options

| Catalog | What it is | When it wins |
|---------|------------|--------------|
| **REST catalog** (the spec) | Iceberg's official catalog protocol over HTTP | Default for any new multi-engine deployment |
| **Apache Polaris** | REST-spec implementation, donated by Snowflake (2024); supports Spark, Flink, Trino, Dremio, Doris, StarRocks ([polaris.apache.org](https://polaris.apache.org/)) | Vendor-neutral REST catalog; pairs with Confluent Tableflow's "Snowflake Open Catalog" path |
| **Lakekeeper** | Independent open-source REST catalog | Strong governance/RBAC story, Rust implementation |
| **AWS Glue Catalog** | AWS-managed, not REST natively (REST adapter exists) | Already on AWS; want to query from Athena / EMR / Redshift / Snowflake |
| **Project Nessie** | Git-like branching for tables (Dremio-led) | You want branch/merge semantics on data — relevant to CO's jujutsu-flavored vision |
| **Unity Catalog** | Databricks-led, now open-sourced (UC OSS) | Already on Databricks |
| **Hive Metastore / JDBC / Hadoop catalog** | Legacy options | Migration scenarios only |

Per [iceberg.apache.org/docs/latest/](https://iceberg.apache.org/docs/latest/), the documented catalog implementations are: REST, AWS Glue, AWS DynamoDB, Hadoop, Hive, JDBC, Nessie, Java Custom.

### 13.2 The interesting one for CO: **Nessie**

> "Git-like branching for data" — this is structurally aligned with CO's jujutsu-style changelog UX. Nessie lets you branch a table, make changes on a branch, merge with conflict detection, time-travel across the lineage. If CO eventually wants per-universe analytical branches that mirror its own commit DAG, Nessie is the catalog whose mental model already matches.

Tradeoff: Nessie adoption is smaller than Polaris/Glue. Pinot/Flink/Spark all read it, but you'll occasionally hit "supported in theory, rough in practice" edges.

---

## 14. Recommended stacks per scenario (revisited)

### 14.1 Scenario A — Video social

```
App ─→ Redpanda (Iceberg Topics) ─→ Iceberg (REST/Polaris)
                                      ├─→ Pinot   (live feed analytics, recsys features)
                                      ├─→ Flink   (sessionization, real-time CTR)
                                      └─→ Spark   (nightly ML training, backfills)
Valkey in front for sessions/presence/rate-limit.
```

Hypothesis: Redpanda's native Iceberg path eliminates a Kafka-Connect cluster and a sink team.

### 14.2 Scenario B — Service catalog

Don't build this stack. A small Postgres + Cloudflare cache is enough. **Reject this section's tooling for Scenario B**; it's overkill and will dominate the cost.

### 14.3 Scenario C — Dropshipping logistics

```
Order/courier webhooks ─→ Redpanda ─→ Iceberg (Polaris/Glue)
                          │              ├─→ Flink (real-time SLA breach detection, ETA)
                          │              └─→ Spark (settlement, finance, supplier scorecards)
                          └─→ Cloudflare Durable Objects (live order state — Part I, §4)
                                  │
                                  └─→ events back to Redpanda (closed loop)
Pinot for the "where's my order" dashboards (sub-second lookup at thousands of QPS).
Valkey for inventory hot path and rate limits.
```

Hypothesis: Durable Objects own the **live state machine** (writeable truth), the event log + Iceberg own **historical truth** for analytics. Don't try to make the analytical store the source of truth for live ops.

### 14.4 Scenario D — CO

**Phased adoption is the recommendation; do not stand all of this up on day one.**

```
Phase A (now → near-term): Cloudflare Workers Analytics Engine for telemetry/A-B.
                            Single Valkey instance in front of co-web for sessions/cache.
                            No event log; no lakehouse.

Phase B (when WAE limits bite — high-cardinality joins, sessionization):
   co-web ─→ Redpanda (small cluster on Fly Machines, or Redpanda Cloud)
              └─→ Iceberg on R2 (REST catalog: start with **Lakekeeper** or **Polaris**,
                                  evaluate **Nessie** if/when jujutsu-on-data clicks)
                    ├─→ DuckDB / ClickHouse for ad-hoc analytics on universe events
                    └─→ Spark on Fly Machines (ephemeral) for monthly rollups

Phase C (only if user-facing analytics dashboards become a product surface):
   add Pinot for sub-second per-universe analytics queries.
   add Flink for stream-to-feature pipelines (recommendations, anomaly detection).
```

### 14.5 Why this phasing for CO

- Workers Analytics Engine is **free / very cheap** at CO's current scale and covers the basic A/B and telemetry needs. Don't pre-build a lakehouse you can't fill.
- Iceberg-on-R2 is the right long-term destination because **R2 has zero egress** and Iceberg + a REST catalog + Parquet are portable across any future engine choice. You're not locked in.
- **Nessie is worth a deeper look for CO specifically** — its branch/merge model on data echoes the jujutsu-shaped UX you want for content. If that resonates, prototype Nessie before committing to Polaris.
- Redpanda over Kafka because: one binary, fits a Fly Machine, and the Iceberg Topics feature collapses two components into one. The known limitation ("can't append to non-Redpanda-created Iceberg tables") is acceptable when Redpanda owns the topic→table lifecycle from day one.
- Defer Pinot until you have a concrete user-facing analytics surface that demands sub-second QPS. ClickHouse Cloud or self-hosted ClickHouse on Fly is a cheaper interim for ad-hoc analytics.

### 14.6 Encryption + lakehouse — the awkward seam

Section 5.4 of Part I committed to **client-side AEAD before content reaches R2**. That blocks server-side queryability of content. For analytics, you have two clean options:

1. **Two data classes.** Plaintext **events** (page views, A/B exposures, clicks, durations, IDs) flow to Redpanda → Iceberg unencrypted but anonymized. **Content** (markdown, video) stays AEAD-encrypted on R2, never enters the lakehouse. This is the recommended split.
2. **Encrypted analytics columns.** Use deterministic or order-preserving encryption on indexed columns. Massively complicates the design and weakens the encryption guarantee. **Reject for v1.**

Document the boundary explicitly in any CO design doc that touches analytics: events vs content, and never ingest content text into Iceberg.

---

## 15. Updated synthesis matrix (data layer)

|  | Redis/Valkey | Redpanda | Kafka (+Connect) | Confluent+Tableflow | Spark | Flink | Pinot | ClickHouse | Iceberg+REST |
|--|---|---|---|---|---|---|---|---|---|
| Video social | ★★★ | ★★★ | ★★ | ★★★ | ★★ | ★★★ | ★★★ | ★★ | ★★★ |
| Service catalog | ★★ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| Dropshipping logistics | ★★★ | ★★★ | ★★ | ★★★ | ★★ | ★★★ | ★★★ | ★★ | ★★★ |
| **CO (phased)** | ★★★ Phase A | ★★★ Phase B | ★ | ★★ | ★★ Phase B | ★★★ Phase C | ★★ Phase C | ★★★ Phase B | ★★★ Phase B |

---

# Part III — CO is the only surface

The reframe (2026-04-30): **CO is not an app that runs on a platform; CO is the control plane through which users manage content, forms, rules, and code across many self-contained deployments.** Each user universe documents its own deployment(s); CO orchestrates them, collects telemetry centrally, encrypts everything per-universe, and scales with users.

This shifts the entire question of "which platform" — CO has **two planes**, and they answer different questions.

## 17. The two planes

```
┌────────────────────────────────────────────────────────────────────┐
│ CONTROL PLANE — CO itself                                          │
│  • Auth, universe registry, deployment orchestrator                │
│  • Central log/event ingest, analytics, A/B service                │
│  • Backup + restore, encryption key custody (wrapped only)         │
│  • Web UI (the "only surface" the user touches)                    │
│  → Runs on: Fly (Rust/Axum core) + Cloudflare (CDN/Pages/R2/WAE)   │
│    — the Part I §5 recommendation, unchanged.                      │
└──────────────────────┬─────────────────────────────────────────────┘
                       │ deploys, observes, encrypts
                       ▼
┌────────────────────────────────────────────────────────────────────┐
│ DATA PLANE — user deployments (per universe, possibly many)        │
│  Each deployment is described in the universe itself:              │
│   - target: vercel | cloudflare | fly | fargate | on-prem | none   │
│   - runtime: static | edge | container | function                  │
│   - bindings: storage refs, secrets refs, domain                   │
│   - scaling rules                                                  │
│  CO's deployer reads the manifest, calls the platform API,         │
│  snapshots the artifact to encrypted R2, wires telemetry back.     │
└────────────────────────────────────────────────────────────────────┘
```

The platforms compared in Part I (Vercel / Cloudflare / Fly / Fargate) are no longer "where CO runs" — they're **deployment targets the user selects per universe**. CO must speak all of them.

## 18. The universe deployment manifest

Each universe carries a deployment manifest as part of its self-contained spec. Sketch (subject to schema design under a future CO-* ticket):

```yaml
# private/CO/deploy.yaml (per universe, per environment)
deploy:
  target: cloudflare-pages       # or vercel | fly | fargate | static-r2
  domain: meu-portfolio.co.app   # or custom CNAME
  runtime:
    kind: static                 # or edge-fn | container | serverless-fn
    build: { command: "co build", output: "dist/" }
  bindings:
    storage: { type: r2, bucket: u-meu-portfolio, encrypted: true }
    secrets: [ STRIPE_KEY, RESEND_KEY ]
  scaling:
    min: 0
    max: 100
  telemetry:
    sink: co-central             # ships logs/events to CO's ingest
    sampling: 1.0
  backup:
    schedule: daily
    retention: 30d
```

Why this matters:
- **Self-contained:** the universe is portable — content + code + deployment + rules in one tree.
- **CO is platform-pluggable:** add a new target (e.g., Render, Railway) by writing a deployer adapter, not by rearchitecting.
- **Reversible:** change `target: vercel` → `target: cloudflare-pages` and CO redeploys; the universe data hasn't moved.

## 19. Where each Part I/II component slots in (final mapping)

| Concern | Component | Plane |
|---------|-----------|-------|
| CO core API (auth, universes, content, deployer) | **Fly.io Machines + Rust/Axum + LiteFS SQLite** | Control |
| CO web UI ("the only surface") | **Cloudflare Pages** | Control |
| Edge cache / CDN for both planes | **Cloudflare** in front | Control |
| Per-universe encrypted blob storage | **R2** with **per-universe AEAD key wrapped by user token** | Both (control owns wrapping; data deployments read via signed URLs) |
| User deployment targets | **Vercel / Cloudflare Workers/Pages / Fly / Fargate / static-on-R2** | Data |
| Centralized log / event ingest | **Redpanda** (single small cluster, Iceberg Topics enabled) | Control |
| Lakehouse for analytics | **Iceberg on R2** with **REST catalog** (start with Lakekeeper or Polaris; evaluate Nessie for jujutsu-shaped lineage) | Control |
| Real-time analytics (cheap path) | **Cloudflare Workers Analytics Engine** | Control (Phase A) |
| Stream processing (when WAE isn't enough) | **Flink** on Fly Machines (small) | Control (Phase B+) |
| Sub-second user-facing analytics | **Pinot** or **ClickHouse** | Control (Phase C only) |
| Hot-path cache, rate-limit, presence | **Valkey** (single instance) | Control |

## 20. Centralized logging + real-time analytics flow

```
[user deployment on Vercel/CF/Fly/Fargate]
        │ (OTEL HTTPS push, signed with per-universe HMAC)
        ▼
[CO ingest endpoint on Fly]  ← rejects payloads carrying plaintext content
        │
        ▼
[Redpanda topic per universe class]   ← Iceberg Topics on, Parquet, R2
        │
        ├──► [Iceberg on R2]  ←  central, partitioned by universe_id
        │       ├── Phase A: query via DuckDB / ClickHouse for ad-hoc
        │       ├── Phase B: Flink for sessionization, real-time joins
        │       └── Phase C: Pinot for in-product analytics surfaces
        │
        └──► [Cloudflare Workers Analytics Engine]  ← cheap real-time tile
                (parallel write while WAE is sufficient; retire later)
```

**Privacy boundary at ingest** (load-bearing): the ingest endpoint validates that no payload field exceeds a content-text size threshold and rejects payloads matching content-shaped patterns. Telemetry carries IDs, types, durations, counts, and dimensions — never markdown, never user prose, never image bytes. Documented and enforced; not a convention.

## 21. Storage optimization per file format

CO's blob layer encrypts everything per-universe, but it should normalize formats *before* encryption to reduce cloud bill and improve restore time.

| Format class | Pre-encryption transform | Rationale |
|--------------|--------------------------|-----------|
| Markdown / text | zstd level 19, dictionary trained on universe corpus | 4–10× compression, fast decompress; AEAD over compressed bytes |
| Images | Convert source → AVIF + WebP fallback, strip EXIF | 30–50% smaller than JPEG/PNG at same quality; privacy-preserving |
| Video | HLS-segment via Stream (or FFmpeg fallback), each segment AEAD | Streamable + cacheable; per-segment keys allow partial sharing |
| Code / artifacts | tar + zstd, treat as immutable, content-addressed | Deduplication across universes; AEAD over tarball |
| Iceberg metadata + analytics events | **Not encrypted** (server queries them) | Privacy preserved by stripping plaintext content at the ingest boundary (§20) |

R2 versioning on top of this gives **time-travel restore for free**; the Iceberg snapshot ID gives a consistent point-in-time across both content blobs and analytics state.

## 22. Backup + privacy guarantees (the user-facing promise)

Three guarantees, written so they can be tested:

1. **At rest:** every content blob in R2 is AEAD-ciphertext under the universe key `K_u`; `K_u` itself is stored only wrapped under the user-token-derived KEK. **Test:** dump a random R2 object; assert it does not decrypt with any server-held key.
2. **In transit:** TLS everywhere; per-universe HMAC on telemetry payloads to prove origin. **Test:** strip TLS, replay a payload — server rejects.
3. **At restore:** restoring from snapshot `S` gives bit-identical universe state to time `t(S)`, including content + metadata + deployment manifest. **Test:** snapshot, mutate, restore, diff — must be empty.

Privacy is structural, not policy: the server **cannot** decrypt user content even if compelled. The user-token is the gate.

## 23. Multi-tenancy that scales with users

| Concern | Mechanism |
|---------|-----------|
| Isolation | Universe = unit of authz, billing, quota, encryption key, telemetry partition |
| Hot read path | Cloudflare cache keyed by universe + content hash (cache-safe per browser §5.4) |
| Per-universe deployment | Each user deployment is its own Fly app / CF Worker / Vercel project — no noisy-neighbor in compute |
| Control plane scaling | Fly Machines autoscale on the CO core; LiteFS replicas in regions; R2 is effectively unbounded |
| Cost attribution | Each universe's R2 prefix + Redpanda partition + telemetry tag → bill at the universe level |
| Quotas | Anonymous: 100 entries (already enforced); paid tiers raise the cap; storage caps per universe |

## 24. Phasing for CO (final)

Restated against this control-plane/data-plane model:

| Phase | Scope | Trigger to advance |
|-------|-------|-------------------|
| **A — Control-plane only** (current) | Single-tenant deployment of CO on Fly + CF; users CRUD content, no user-driven deploys yet | Demand for "publish to my own domain" |
| **B — Manifest-driven deploys** | Add deployer adapters: static-on-R2, Cloudflare Pages, Fly app. Per-universe `deploy.yaml`. Snapshots to R2. | Multiple universes deploying concurrently; need observability |
| **C — Centralized telemetry** | Stand up Redpanda (small) + Iceberg-on-R2 with REST catalog. OTEL ingest; ad-hoc DuckDB/ClickHouse queries. WAE in parallel. | Ad-hoc queries become bottleneck; want real-time joins / sessionization |
| **D — Stream + OLAP** | Add Flink for stream processing. Add Pinot (or ClickHouse) for user-facing analytics surfaces. | Real-time A/B test panels are a product surface |
| **E — Multi-target deployers** | Add Vercel + Fargate deployer adapters. Marketplace of templates per target. | Users demand specific platforms |

**Don't skip phases.** Each phase pays for itself before the next is justified; building Phase D before Phase B is the canonical way to over-engineer this.

## 25. What this reframe changes vs. Part I §5

| Decision in Part I §5 | Status after reframe |
|------------------------|---------------------|
| CO compute on Fly | **Unchanged** — control plane stays on Fly |
| Content blobs on R2 with client-side AEAD | **Unchanged** — applies per-universe; central key custody |
| Cloudflare Pages frontend | **Unchanged** — this *is* "the only surface" |
| WAE for telemetry | **Phase A only** — graduates to Redpanda + Iceberg in Phase C |
| Conflict resolution inside `co-core` | **Unchanged** — and now it's load-bearing because each universe is a self-contained merge target |
| "CO is a Rust app on Fly" | **Reframed** — CO is a control plane that *uses* Fly; the user's universe may target any platform |

The hybrid recommendation stands. What's added: an explicit **deployer abstraction**, a **manifest schema** as a first-class artifact in every universe, and a **central telemetry spine** that arrives in phases.

---

## 26. Re-evaluate every 6 months — checklist

- [ ] Refetch each platform's pricing page; diff against this doc's snapshot.
- [ ] Refetch each platform's limits/quotas page.
- [ ] Re-run the empirical probe for the workload that's grown most.
- [ ] Update the synthesis matrix and the CO topology section if any cell changed by more than one star.
- [ ] Open a doc PR with the diff. Don't silently update.

Snapshot date of this evaluation: **2026-04-30** (CO 1.21.2).
