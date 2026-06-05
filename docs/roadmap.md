# CO Roadmap — Wave-based, v3.0 mobile public release in Wave 4

Framed by the BaaS thesis (see [`ArteLonga/docs/brain-as-a-service.md`](https://github.com/artelonga/ArteLonga/blob/main/docs/brain-as-a-service.md)).

A **brain** = human + their hardware + their notes/tasks/calendar + their content, delivered as a sovereign, renderable surface that scales horizontally at zero marginal SaaS cost.

**`yuri`** is the reference brain. **`artelonga`** is the business. **CO** is the platform — identity · warehouse · payment · sync.

## Current state (2026-06-05)

- **Version**: v2.40.0 → v2.41.0 (Wave 3 in flight: CO-368 + CO-370 pending)
- **Production**: `https://co-artelonga.fly.dev` — single Fly app, shared-cpu-1x, 512 MB
- **Staging** (post-CO-373): `https://staging.co.artelonga.com.br`
- **Cadence**: bi-weekly Thursday 15:00 BRT releases (PR cutoff Wed 23:59 BRT); CO-372 cron-driven from Wave 4
- **Open work items**: ~90 (after closing CO-94/146/162/277/143)
- **Per-wave DoD**: `docs/release-checklist.md`

## Wave-aligned releases

Each wave = one git semver tag = a cohesive set of PRs tested + verified end-to-end.

| Wave | Tag | Theme | PR count |
|---|---|---|---|
| ⏪ done | v2.39.0 | Foundation (CO-211/280/291/301/337/338) | 6 |
| ⏪ done | **v2.40.0** | Substrate stable + OSS integrations | 10 |
| 🟡 active | **v2.41.0** | Brain interlink + scrum + retro sim | 7 |
| 🔵 next | **v3.0.0 — PUBLIC LAUNCH** | All substrate + Sala + mobile + staging + edge protection + privacy | **17** |
| ⚪ planned | v3.1.0 | Monetization + KB + funnel + sprint calendar | 4 |
| ⚪ planned | v3.2.0 | Security epic (encrypted assets, .co format, fs-as-web) | 4 |
| ⚪ planned | v3.3.0 | Sync + offline (op log, conflict UI) | 4 |
| ⚪ planned | v3.4.0 | Scale (job queue, cache, rate tiers, load tests) | 6 |
| ⚪ planned | v3.5.0 | Universe types (manifest plugins, git-backed) | 4 |
| ⚪ planned | v3.6.0 | Native shells (Capacitor) + advanced | 3 |

## Wave 4 — v3.0 (the big public release push)

**Theme**: Brain on any device, public.

17 PRs batched for CI sanity. Each batch fires when prior batch's PRs merge.

### Batch A — substrate gates (4 PRs, fire first)
| PR | Spec | What it ships |
|---|---|---|
| **CO-373** | Staging Fly app + DNS + per-test universe isolation | `staging.co.artelonga.com.br` live |
| **CO-365** | Storage backend trait | LocalFsBackend default; S3/R2/Fly/GCS stubs |
| **CO-278-B** | Rate limits + abuse heuristics | 60/min anon; X-RateLimit-* headers |
| **CO-360** | Unified `/gestao/resumo` | One Svelte page, 4 tabs |

### Batch B — Sala + identity (5 PRs)
| PR | Spec | What it ships |
|---|---|---|
| **CO-352** | Workspace primitive | `entry_type: workspace` + per-user state |
| **CO-354** | Suggest/review pipeline | draft → reviewed → published lifecycle |
| **CO-355** | Workspace template registry | `_workspace.yaml` per universe |
| **CO-377** | Cross-env identity (Phase 1) | yuri creds work on prod + staging |
| **CO-378** | Analytics privacy (noindex respect) | Private paths redacted in `/gestao/resumo` |

### Batch C — Realtime + mobile foundation (4 PRs)
| PR | Spec | What it ships |
|---|---|---|
| **CO-353** | WebSocket lobby + presence | Cursors broadcast < 300ms |
| **CO-356** | Touch DnD on board | Pointer-events replaces HTML5 DnD |
| **CO-357** | PWA shell | Manifest, SW, install, offline cache |
| **CO-358** | Mobile IA pass | Drawer, breadcrumb collapse, board reflow |

### Batch D — Validation + tagging (4 PRs)
| PR | Spec | What it ships |
|---|---|---|
| **CO-374** | Playwright E2E suite for staging | 6 scenario files + acceptance generator |
| **CO-375** | API contract enforcement | Probe drift gates prod release |
| **CO-376** | Pre-prod migration validation | Snapshot+migrate+smoke per migration PR |
| **CO-359** | Mobile E2E CI matrix | Pixel 7 / iPhone 14 / iPad Pro projects |

**v3.0 release ships** when Batch D closes + `docs/release-checklist.md` Wave 4 section all green + retrospective passes.

## All open todo/in_progress work mapped

| ID | Title | Wave |
|---|---|---|
| CO-368 | Scrum entry types | v2.41 (in flight) |
| CO-370 | Lead funnel + unified capture | v2.41 (in flight) |
| CO-352, 353, 354, 355, 356, 357, 358, 359, 360 | Workspace + mobile + admin | **v3.0 Wave 4** |
| CO-365, 373, 374, 375, 376, 377, 378, 278-B | Storage + staging + privacy + rate limits | **v3.0 Wave 4** |
| CO-366, 367, 371, 372 | Monetization + KB + funnel + calendar | v3.1 Wave 5 |
| CO-145 | Encrypted assets | v3.2 Wave 6 |
| CO-86, 87, 110 | `.co` format + protocol stack + fs-as-web | v3.2 Wave 6 |
| CO-61, 62, 128, 58 | Sync protocol + adapter + conflict UI + PWA offline | v3.3 Wave 7 |
| CO-76, 78, 79, 80, 101, 285, 286 | Scale infra + job queue + cache + rate tiers + load + Fly cost | v3.4 Wave 8 |
| CO-63, 70, 89, 93 | Universe types + manifest plugins + git-backed | v3.5 Wave 9 |
| CO-344, 211 v2, 264 | Native shells + advanced | v3.6 Wave 10 |
| CO-227, 228, 170, 144, 283, 284, 298 | Ongoing refactors (no wave) | Continuous |
| CO-94, 143, 146, 162, 277 | Already resolved (status flipped today) | — |

## The BaaS invariants (must hold across every wave)

1. **3 separated layers** — brain content / brain surface / CO platform
2. **Sovereign edge** — brain owner controls hardware + data; CO is consented async
3. **No third-party lock-in** — every external integration behind a trait
4. **Cache-first delivery** — local write renders immediately; sync eventual; ingest break ≠ unavailable

## The bi-weekly release cadence (cron-driven from Wave 4)

```
Monday-Wednesday: PR work
Wednesday 23:59 BRT: PR cutoff for current sprint
Thursday 06:00 BRT: main HEAD → staging auto-deploy
Thursday 06:00-12:00 BRT: Playwright full suite vs staging
Thursday 12:00 BRT: green = release candidate; red = abort
Thursday 12:00-14:30 BRT: human review window
Thursday 15:00 BRT: prod deploy via release-commit.sh + git tag
Thursday 15:05 BRT: retrospective begins
```

CO-372 calendar surfaces the cadence; CO-376 gates with migration validation; CO-374 gates with E2E suite.

## Cost addendum

- Current prod: ~$3-8/mo (shared-cpu-1x, 512 MB, 3 GB volume)
- Add staging (Wave 4): +$3/mo
- Total post-Wave 4: ~$6-11/mo
- Growth tier (50 universes): ~$8/mo prod + $3/mo staging
- Yggdrasil-scale (200 universes): ~$25-30/mo prod + $3/mo staging

## Conventions

- Branch: `feat|fix|refactor/CO-<n>-<short-desc>` (no `issue-` prefix)
- Commits: conventional, `Co-Authored-By: Claude <noreply@anthropic.com>`
- Spec format: `work/co/CO-<n>.md` with YAML frontmatter
- Forbidden in agent PRs: `Cargo.toml`, `co-cli/Cargo.toml`, `CHANGELOG.md` (release-commit owns)
- Changelog entries: `CHANGELOG-PENDING/CO-<n>.md`
- Merge: `scripts/safe-merge-pr.sh artelonga/co <pr>` (never bare `gh pr merge --delete-branch`)

## Reference repos

| Repo | Role | Surfaced as universe? |
|---|---|---|
| `artelonga/co` | the platform (this repo) | `co` |
| `artelonga/ArteLonga` | BaaS thesis + reference surfaces (`docs/brain-as-a-service.md` · `scrum/scrum.md`) | `artelonga` + child universes `yuri`, `neuro` |
| `artelonga/quilomboaraucaria` | **canonical reference architecture** (analytics, telemetry, atividades, schema_versoes, OpenAPI codegen — patterns CO adopts) | `quilomboaraucaria` |
| `artelonga/yggdrasil` | game runtime + sala (workspace absorbed into CO Wave 4) | `yggdrasil` |
| `artelonga/comunicacao` | mbya + yoruba lexicon source | `comunicacao` (parent of `mbya`, `yoruba`) |
| `artelonga/topologia` | concept plane source (CO-141) | `topologia` (parent of language family) |
| `artelonga/mbya` | Arandu — standalone Mbyá Guarani lexicon (Rust + Dioxus + SQLite) | `mbya` (promoted to first-class via CO-348) |
| `artelonga/retro-umarizal` | neighborhood memory site | `retro-umarizal` (CO-347) |
| `artelonga/rfq-gateway` | RFQ trade log | `rfq` |
| `pewdiepie-archdaemon/odysseus` | open-source AI workspace reference (read-only) | `odysseus` (CO-364) |
| `anthropics/claude-code` | Anthropic agentic CLI reference (read-only) | `claude-code` (CO-364) |

## Pre-flight gate (every release)

See `docs/release-checklist.md` for the full per-wave checklist.
