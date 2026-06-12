# Epics backlog — release-ready index

> Derived from `work/co/CO-*.md` (`grep 'type: epic'`) + child statuses, reconciled
> against [`../roadmap.md`](../roadmap.md). Last reconciled: **2026-06-12** (v3.5.0
> pending on `main`). This is the "epics backlog ready at release" artifact: every
> open epic, its child user-stories, and the Wave/version it targets.

**Statuses** are the live `status:` field of each work item. An epic can read
`todo` while most of its children are `done` (the epic is the *formal* close, not
the work) — those are flagged so they can be closed at release.

---

## Theme: Security & encryption

The Wave-6 security epic — design pinned in
[`wave6-security-epic.md`](wave6-security-epic.md) (CO-402, done). The
encryption-at-rest precondition (CO-104 backup + CO-119 restore drill) is now
**done**, so the gate that blocks encryption-at-rest is satisfied.

| Epic | Title | Status | Children (status) | Targets |
|---|---|---|---|---|
| **CO-145** | Encrypted, indexable, lazy-loaded assets | `todo` (closeable — all children done) | CO-146 ✅ CAS upload · CO-147 ✅ indexable metadata · CO-148 ✅ ChaCha20 envelope · CO-149 ✅ HTTP range/ETag · CO-150 ✅ lazy-load UI | **Wave 6** (impl largely shipped; close epic at release) |
| **CO-115** | Phase 4 — encrypted + privileged compute zone (operator-cannot-read) | `todo` | CO-130 privileged compute zone · CO-131 k-anonymity DLP at egress · CO-132 key-access audit log (all `todo`) | **Wave 6+** (long-horizon, after CO-145) |

**Adjacent design specs (not `type: epic`, but Wave-6 scoped):** CO-86 `.co`
format · CO-87 protocol stack · CO-110 filesystem-as-web · CO-148 envelope
(done). All pinned by `wave6-security-epic.md`.

---

## Theme: Scale & data-infra (the phased platform roadmap, CO-111…116)

The six "Phase N" platform epics. Phase 0–1 are mostly delivered; Phase 2–5 stay
long-horizon and only fire when a real load/collaboration demand justifies the
streaming/lake machinery.

| Epic | Title | Status | Children (status) | Targets |
|---|---|---|---|---|
| **CO-111** | Phase 0 — foundation hardening | `todo` (closeable) | CO-117 ✅ Cloudflare CDN · CO-118 WAE telemetry (`todo`) · CO-119 ✅ restore drill | Continuous / done-leaning |
| **CO-112** | Phase 1 — demoable + telemetry | `todo` | CO-120 co-agent adapter trait (`todo`) · CO-121 A/B primitives (`todo`) · CO-122 ✅ quota/tier model | Wave 8 (scale) |
| **CO-113** | Phase 2 — sustained public test | `todo` | CO-123 ClickHouse · CO-124 multi-target agents (both `todo`) | Wave 8+ |
| **CO-114** | Phase 3 — collaboration + streaming | `todo` | CO-125 Redpanda · CO-126 Iceberg-on-R2 · CO-127 Flink · CO-128 conflict UI · CO-129 ✅ jujutsu changelog | Wave 7 (sync) / 8+ (streaming) |
| **CO-116** | Phase 5 — programmable platform + multi-target deployer | `todo` | CO-133 ✅ deploy.yaml schema · CO-134 ✅ static-on-R2 deployer · CO-135 cloudflare-pages (`todo`) · CO-136 Pinot (`backlog`) | Wave 9+ |

*(CO-115 Phase 4 is listed under Security above.)*

**Related non-epic infra specs:** CO-284 (pluggable infrastructure — trait-
abstracted backends), CO-76/78/79/80/101/285/286 (scale infra: job queue, cache,
rate tiers, load tests, Fly cost) — all Wave 8.

| Epic | Title | Status | Children | Targets |
|---|---|---|---|---|
| **CO-284** | Pluggable infrastructure architecture (trait-abstracted backends) | `todo` | no `children:` frontmatter — cross-cuts CO-365 storage trait (shipped) + the CO-111…116 adapter work | Continuous / Wave 8 |

---

## Theme: Server decomposition & code-health (CO-227…231)

The four refactor epics + their documentation epic. **All child user-stories are
`done`** — these epics are formally closeable at the v3.5.0 release.

| Epic | Title | Status | Children (all ✅ done) | Targets |
|---|---|---|---|---|
| **CO-227** | Server decomposition | `todo` (closeable) | CO-215 split `server.rs` · CO-216 break storage↔server cycle · CO-219 chat → folder · CO-221 slim AppState · CO-224 route context folders | Continuous — **close at release** |
| **CO-228** | Type safety | `todo` (closeable) | CO-217 typed handler payloads · CO-218 SPA → TypeScript | Continuous — **close at release** |
| **CO-229** | Event-driven workers | `todo` (closeable) | CO-220 in-process event bus · CO-223 shared `Worker` trait | Continuous — **close at release** |
| **CO-230** | Auth unification | `todo` (closeable) | CO-222 single typed extractor | Continuous — **close at release** |
| **CO-231** | Documentation | `todo` (closeable) | CO-225 AppState pattern + `MODULES.md` · CO-226 OpenAPI coverage | Continuous — **close at release** |

> All five report `status: todo` while every child is `done`. Recommend flipping
> the parents to `done` in the v3.5.0 release commit (housekeeping, no code).

---

## Theme: Sync & offline

No dedicated `type: epic` item; tracked as a wave of user-stories.

| Spec | Title | Status | Targets |
|---|---|---|---|
| CO-61 | Sync protocol v1 — op-log, content-addressed | `todo` | **Wave 7** (sync + offline) |
| CO-62 | Sync adapter | `todo` | Wave 7 |
| CO-128 | Apple-style 4-way conflict UI | `todo` (child of CO-114) | Wave 7 |
| CO-58 | PWA offline cache | `todo` | Wave 7 |

> The sync wave consumes the op-log; the CO-399 scope key must stay op-log
> friendly (Wave-7 sync consumes it) — see roadmap.

---

## Theme: Platform experiences & public API

| Epic | Title | Status | Children (status) | Targets |
|---|---|---|---|---|
| **CO-414** | Experiences-as-features — three user journeys (Miguel, Yuri-source, Yggdrasil) | `in_progress` | CO-415 GitHub login (`todo`) · CO-416 auto pt-br/en translation (`todo`) · CO-417 ✅ source:github adapter · CO-418 ✅ render-review-publish traceback · CO-419 ✅ yuri/nlp SensorySpeech · CO-420 ✅ Yggdrasil UX→epics | **v3.4.0 partial (417/418/419/420 shipped); 415/416 → v3.5.0+** |
| **CO-278** | Public API surface — versioned, documented, rate-limited | `todo` | no `children:`; depends on CO-273/275 telemetry endpoints | Continuous (rate-limit slice shipped as CO-397 in v3.0.0) |

---

## Closeable-at-release summary (housekeeping)

Epics whose work is delivered but whose `status:` still reads `todo` — flip to
`done` in the v3.5.0 release commit:

- **CO-227, CO-228, CO-229, CO-230, CO-231** — every child `done`.
- **CO-145** — every child (CO-146…150) `done`; encryption-at-rest shipped.
- **CO-111** — CO-117 + CO-119 done; only CO-118 (WAE telemetry) remains, so
  re-scope rather than close.

Still genuinely open (carry forward): **CO-112/113/114/116** (phased scale/
streaming, long-horizon), **CO-115** (privileged compute zone, Wave 6+),
**CO-284** (pluggable infra, continuous), **CO-414** (two journeys remain),
**CO-278** (public API surface).
