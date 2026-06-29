# CO Platform — Universe Registry

Canonical inventory of all system-owned universes. Every universe listed here
has a documented purpose and an idempotent seed path (re-running `co-web` on a
fresh database reproduces the same set).

---

## System universes

| Key | Name | Visibility | Owner | Seed path | Purpose |
|-----|------|-----------|-------|-----------|---------|
| `template` | CO | public-static | system | `seed_template_universe()` + `reseed_template_content_pages()` | Read-only onboarding board; source of anonymous clones |
| `yggdrasil` | Yggdrasil | requires-login | system | `seed_yggdrasil_universe()` | Minigames hub (CO-38) |
| `dados` | Dados Rastreados | public-static | system | seeded externally | Privacy disclosure dataset |
| `tempo` | Linha do Tempo | public-static | system | `seed_timeline_universe()` manifest | Universal timeline (CO-92) |
| `humanity` | Humanidade | public-static | system | `seed_timeline_universe()` manifest | Human-history timeline (CO-92) |
| `universo` | Universo | public-static | system | `seed_timeline_universe()` manifest | Cosmological timeline (CO-92) |

## Admin-owned content universes

Seeded by `seed_admin_content_universes()` — DB rows only; content is pushed
via the Vault API or `co push` separately.

| Key | Name | Visibility | Purpose |
|-----|------|-----------|---------|
| `artelonga` | ArteLonga | public-subscribable | Portfolio and public presence |
| `rfq` | RFQ Gateway | private | Trade log and quotation system |
| `co` | Co Platform | public-subscribable | CO roadmap, releases, and decisions |

## Dev board

The CO development board (`co-dev` key) was deprecated in CO-142. It is no
longer seeded as a universe row. The dev board API routes are now mounted at
`/api/v1/admin/co-dev` (admin-only) and read from `data/co/CO-*.md`, which is
refreshed on every boot from the bundled `work/co/` snapshot (`/app/seed-co/`).

## Deprecated universes (hard-deleted on boot)

These keys are deleted from the database on every `co-web` startup via
`delete_deprecated_universes()`.

| Key | Reason for deletion |
|-----|---------------------|
| `co-dev` | CO-142: replaced by admin-only API + `co` work universe |
| `co-experience` | CO-142: concept retired; tracking moves to `co` universe epics |

## Epic ↔ sub-universe decision (CO-142 Phase C)

**Decision: epics stay as entries in the `co` universe.**

Each epic (CO-20, CO-53, CO-98 …) remains a `type: epic` entry inside the `co`
universe. Sub-universes via `parent_key` (CO-98) are reserved for genuinely
separate content trees (e.g., `tempo`/`humanity`/`universo` under `template`).

This avoids universe proliferation and keeps the CO roadmap in one navigable
board. If a future epic warrants its own universe (e.g., a game), it can be
promoted individually with an explicit migration.
