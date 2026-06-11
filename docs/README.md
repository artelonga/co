# Co — Documentation index

Documentation for the **Co platform** (graph-based content management — CLI, web server, kanban board). Reflects the **1.21.2** release as of 2026-04-30.

## Read first

| Document | When to read it |
|----------|----------------|
| [`WELCOME.md`](./WELCOME.md) | You just arrived. The CO philosophy, abstractions, history timeline, and a first CRUD example. |
| [`CLI.md`](./CLI.md) | Command reference for the `co` binary — auth, push, construir, updates, serve. |
| [`OPERATIONS.md`](./OPERATIONS.md) | You're deploying, smoke-testing, recovering, or rotating secrets. |
| [`feedback-checklist.md`](./feedback-checklist.md) | You're a public-test user and want to know what to try + how to report. |
| [`BREAKING-CHANGES.md`](./BREAKING-CHANGES.md) | You're upgrading a deployment across a minor version. |
| [`../CHANGELOG.md`](../CHANGELOG.md) | You want to know what landed and when. Newest at top. |
| [`../CLAUDE.md`](../CLAUDE.md) | You're a Claude agent (or ADHD developer using Claude) — project conventions live here. |
| [`../work/co/SPRINT-V1-LAUNCH.md`](../work/co/SPRINT-V1-LAUNCH.md) | You want to know what's next on the road to v1.0. |
| [`../work/co/ROADMAP-V1-LAUNCH.md`](../work/co/ROADMAP-V1-LAUNCH.md) | You want the long-arc roadmap (Tiers 0 → 5). |
| [`QUOTAS.md`](./QUOTAS.md) | You want to understand the quota/tier model (entry limits, storage, deployments, tier behaviors). |

## Pending docs (planned in CO-100)

| Document | Status | Ticket |
|----------|--------|--------|
| `ARCHITECTURE.md` — component map, data flows, theme/access model | not started | [CO-100](../work/co/CO-100.md) |
| `ONBOARDING.md` — 5-min "set up Co locally" guide | not started | [CO-100](../work/co/CO-100.md) |
| `CONTRIBUTING.md` (top-level) | not started | [CO-100](../work/co/CO-100.md) |

CO-100 is queued in Wave 3 of the sprint plan.

## Diagrams

`docs/diagrams/` holds Mermaid sources for system diagrams. Embed via the in-tree Mermaid renderer (CO-83 / CO-107). No external rendering dependency.

## Conventions

- **Language:** PT-BR primary, EN translations welcome (PR-friendly).
- **Format:** plain Markdown — no Sphinx/MkDocs build step. The Co platform itself can render docs as universe content.
- **Versioning:** doc deltas roll into the same patch version as the code change they describe.

## See also

- Open work items: `../work/co/CO-*.md` (each is a self-contained spec with co-auto invocation prompt).
- Project metadata: `../work/co/project.yaml`.
