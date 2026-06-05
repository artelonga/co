# CO Roadmap — Brain as a Service

This roadmap is framed by the BaaS thesis (see [`ArteLonga/docs/brain-as-a-service.md`](https://github.com/artelonga/ArteLonga/blob/main/docs/brain-as-a-service.md)).

A **brain** = human + their hardware + their notes/tasks/calendar + their content, delivered as a sovereign, renderable surface that scales horizontally at zero marginal SaaS cost.

**`yuri`** is the reference brain. **`artelonga`** is the business. **CO** is the platform — identity · warehouse · payment · sync.

## Current state (2026-06-05)

- **Version**: v2.40.0 — substrate stable + OSS integrations
- **Production**: `https://co-artelonga.fly.dev` — single Fly app, shared-cpu-1x, 512 MB
- **Content universes on prod**: co, artelonga, comunicacao, mbya, rfq, yggdrasil, template, plus yuri/retro-umarizal/yoruba/neuro/odysseus/claude-code surfaced via remote sync
- **Open work items**: ~95 (after backfill: 13 critical, 41 high)

## Phased releases to v3.0 public launch

### ✅ v2.40 — Brain substrate stable + open brain references (shipped 2026-06-05)
Universal substrate ready; first OSS brain references (odysseus, claude-code) ingested.

### 🟡 v2.41 — Brain interlink map (this week)
The axons between thoughts — cross-universe wikilinks resolve, graph view aggregates across brains.
- **CO-345** cross-universe graph view + publishable saved views (🟢 just merged)
- **CO-363** wikilink resolver (`[[key::path]]` → entry_relations.to_universe)
- **CO-350** Catalog → OpenAPI codegen + CI drift check
- **CO-361** atividades audit log + schema_versoes admin surface

### 🟡 v2.42 — Brain interaction surface (Sala) (week 2-3)
How a brain works — multi-user spatial canvas anchored to a content universe. Yggdrasil's `comunicacao` sala absorbed into CO as the **workspace** primitive.
- **CO-352** workspace primitive (`entry_type: workspace` + per-user state)
- **CO-355** workspace template registry (`_workspace.yaml` per universe)
- **CO-354** suggest/review pipeline (with login CTA)
- **CO-365** storage backend trait (replaces CO-143 AWS lock-in)
- Yggdrasil `/universos/comunicacao` 301-redirects to CO

### 🟡 v2.43 — Multi-brain collab + brain dashboard (week 4)
Realtime presence; single dashboard for the operator.
- **CO-353** WebSocket lobby + realtime presence (cursors, live placement)
- **CO-360** unified `/gestao/resumo` dashboard (collapses 6 admin routes)
- Yggdrasil game content migrated into CO `yggdrasil` universe

### 🔵 v3.0 — First brain on any device, public (week 5-6)
Public launch — anyone can register, browse template, fork a universe, install as PWA on phone.
- **CO-356** touch DnD on board (mobile compat)
- **CO-357** PWA shell (manifest, SW, install prompt, offline cache)
- **CO-358** mobile IA pass (drawer sidebar, breadcrumb collapse, board reflow)
- **CO-359** mobile E2E CI matrix (Pixel 7 / iPhone 14 / iPad Pro)
- **CO-278-B** public API rate limits + abuse protection
- Press launch, blog post, open invite

### 🟣 v3.1 — Monetization + universal KB
First-product moment — brain owners can pay; any content syncs to KB.
- **CO-366** conversion/payment wiring (Hostinger first; Pix + Stripe stubs)
- **CO-367** universal content → KB sync (generalizes CO-340 rollup pattern)

### Future (v3.2+)
- Sync + offline protocol v1 (CO-61, CO-62, CO-128)
- Security: encrypted assets (CO-145), `.co` format (CO-86), filesystem-as-web (CO-110)
- Scale: load test scaffolding (CO-101), per-tier rate limits (CO-80), embedding sidecar (CO-286)
- Native shells: Capacitor iOS/Android (CO-344)
- Universe types: manifest plugin system (CO-63, CO-70, CO-89, CO-93)

## The BaaS invariants

These four properties must hold across every release:

1. **3 separated layers** — brain content / brain surface (form) / CO platform (identity · warehouse · payment · sync)
2. **Sovereign edge** — the brain owner controls their hardware + their data; CO is consented async
3. **No third-party lock-in** — every external integration is behind a trait (storage, payment, sync target)
4. **Cache-first delivery** — local write renders immediately; sync is eventually consistent; ingest break ≠ unavailable

## Critical open items not yet in a wave

| ID | Why | Disposition |
|---|---|---|
| CO-145 encrypted assets | Sovereignty signal; post-v3.0 | v3.2 |
| CO-86 `.co` file format | Transport-optimized; post-v3.0 | v3.2 |
| CO-87 composable protocol stack | Architecture; post-v3.0 | v3.2 |
| CO-76 scalability infra | Pre-empts thousand-brain scale | v3.2 |
| CO-278-A token tiers + billing | Builds on CO-366; post-v3.1 | v3.2 |

## Conventions

- **Branch**: `feat|fix|refactor/CO-<n>-<short-desc>` (no `issue-` prefix)
- **Commits**: conventional, `Co-Authored-By: Claude <noreply@anthropic.com>`
- **Spec format**: `work/co/CO-<n>.md` with YAML frontmatter (id, title, status, priority, labels, module, dates, related)
- **Forbidden in agent PRs**: `Cargo.toml`, `co-cli/Cargo.toml`, `CHANGELOG.md` — owned by `scripts/release-commit.sh`
- **Changelog entries**: write to `CHANGELOG-PENDING/CO-<n>.md`; release script consolidates
- **Merge**: `scripts/safe-merge-pr.sh artelonga/co <pr>` (never bare `gh pr merge --delete-branch`)

## Reference repos

| Repo | Role |
|---|---|
| `artelonga/co` | the platform (this repo) |
| `artelonga/ArteLonga` | the BaaS thesis + reference surfaces (`docs/brain-as-a-service.md`) |
| `artelonga/yggdrasil` | game runtime; content moving to CO |
| `artelonga/comunicacao` | mbya + yoruba lexicon source; surfaced as CO universes |
| `artelonga/topologia` | concept plane source; surfaced as CO universe |
| `artelonga/rfq-gateway` | RFQ trade log; surfaced as CO universe |
| `pewdiepie-archdaemon/odysseus` | open-source AI workspace reference (read-only) |
| `anthropics/claude-code` | Anthropic agentic CLI reference (read-only) |
