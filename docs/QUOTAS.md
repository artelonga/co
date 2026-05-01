# CO — Quota & Tier Model

> **Status: v1 proposal — all numeric limits are marked for revisit at Tier 2 exit.**
> Enforcement code is out of scope for this document (see CO-80 / CO-79, Phase 2).
> This spec exists so UI, API, and billing can implement enforcement deterministically
> without relitigating the numbers in three places.

---

## Tier matrix

| Tier | Entries / universe | Storage / universe | Universes | Telemetry events/day | Deployments | Price/mo |
|------|--------------------|--------------------|-----------|-----------------------|-------------|----------|
| **Anônimo** | 100 ¹ | 50 MB ¹ | 1 (auto-clone) ¹ | 1 000 ¹ | 0 | — |
| **Convidado** (autenticado) | 1 000 ¹ | 500 MB ¹ | 5 ¹ | 10 000 ¹ | 1 (static-on-R2) ¹ | — |
| **Cocriador** | 10 000 ¹ | 5 GB ¹ | 25 ¹ | 100 000 ¹ | 5 (any target) ¹ | TBD |
| **Coletivo** | 100 000 ¹ | 50 GB ¹ | unlimited | 1 000 000 ¹ | unlimited | TBD |
| **Admin** | unlimited | unlimited | unlimited | unlimited | unlimited | n/a |

¹ **v1 proposal — revisit at Tier 2 exit.**

### Tier descriptions

| Tier | Who | How acquired |
|------|-----|--------------|
| Anônimo | Unauthenticated visitor | Default; no sign-up required |
| Convidado | Authenticated user | Sign up via email or OAuth |
| Cocriador | Paying subscriber (tier 2) | Stripe checkout (CO-billing epic, TBD) |
| Coletivo | Paying subscriber (tier 3) | Stripe checkout (CO-billing epic, TBD) |
| Admin | Platform operator | `CO_SEED_ADMIN_EMAIL` env var or DB flag |

---

## What counts toward each limit

| Dimension | Counted as | Notes |
|-----------|-----------|-------|
| **Entries** | Rows in `entries` table for the universe | Includes all entry types (task, note, event, …) |
| **Storage** | Sum of `size_bytes` across `entries` + attached files for the universe | Blobs stored in universe directory; SQLite row overhead excluded |
| **Universes** | Rows in `universes` table owned by the user (`deleted_at IS NULL`) | Auto-clone counts as 1 |
| **Telemetry events/day** | Rows written to `telemetry_events` with `created_at >= today_utc` for the universe | Rolling 24-hour window, UTC midnight reset |
| **Deployments** | Rows in `deployments` table for the universe within the billing period | Resets monthly; first-ever deploy counts |

---

## Limit behaviors

| Usage level | Dimension | Behavior |
|-------------|-----------|----------|
| ≥ 80% of any limit | Entries, Storage, Universes, Deployments | Soft warn shown in UI: "Você está próximo do limite" (dismissable toast, re-shown on next session) |
| 100% of entries / storage / universes | Entries, Storage, Universes | **Hard block on writes** — reads continue unaffected. Prompt: "Crie sua conta para continuar" (Anônimo) or upgrade CTA (Convidado+) |
| 100% of telemetry events/day | Telemetry | Drop oldest events for the universe (FIFO eviction); surface count of dropped events in admin dashboard |
| 100% of deployment count | Deployments | Deploy API returns **402 Payment Required** with JSON body `{"error":"deployment_limit_reached","upgrade_url":"…"}` |

### Hard-block detail (entries / storage / universes)

- Write endpoints (`POST /api/v1/universes/:slug/entries`, `PUT`, `DELETE` do not count as write-quota consumption) return **429** with body `{"error":"quota_exceeded","dimension":"entries","limit":100,"tier":"anonymous"}`.
- Read endpoints continue with **200**.
- The `upgrade_url` field in the 429/402 body points to `/upgrade` (or Stripe checkout once billing ships).

---

## Tier transition rules

| Transition | Trigger | Side-effects |
|------------|---------|--------------|
| Anônimo → Convidado | User signs up and email is verified | Existing auto-clone universe is claimed; entry count carries over |
| Convidado → Cocriador | Stripe subscription confirmed (webhook) | Entry + storage headroom expands immediately; old entries unaffected |
| Cocriador → Coletivo | Stripe plan upgrade | Same as above |
| Any → Admin | DB flag / seed env var | No quota enforced; admin dashboard unlocked |
| Downgrade (any) | Stripe subscription cancelled / lapsed | **Reads-only until usage falls below new tier's limit**; no data deleted automatically |

---

## Out of scope (Phase 2+)

- **Enforcement code** — CO-80 (rate limiting) / CO-79 (caching), Phase 2.
- **Stripe / payments integration** — separate billing epic.
- **Per-tier feature flags** (e.g., "encryption only on Coletivo+") — Phase 4.
- **Storage billing by egress** — post-v1.

---

## Related tickets

| Ticket | Topic |
|--------|-------|
| CO-122 | This spec |
| CO-80 | Rate limiting enforcement (Phase 2) |
| CO-79 | Caching (Phase 2) |
| CO-23 | Anonymous usage gate — 100 entries (already shipped) |
| CO-112 | Platform epic (parent of CO-122) |
