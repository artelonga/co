---
id: 62
title: "quilombo-blog sync adapter — 3-way merge UAT↔prod (CO-61 v1 subset, photos-only)"
status: blocked
priority: critical
parent: 61
blocked_reason: mis-routed-task
recorded_at: 2026-06-21
recorded_by: co-auto (Claude)
---

# CO-62 — BLOCKED: mis-routed into the wrong repository

This task was checked out as a worktree of the **`co`** repo
(`git@github.com:artelonga/co.git`), but every acceptance criterion targets a
**SvelteKit/TypeScript** application — not Rust. It cannot be implemented or
committed here without fabricating work that does not belong in `co`.

## Evidence

CO-62's criteria reference artifacts that do not exist anywhere in `co`:

- `conteudo.ts`, `salvarFoto(...)`, `excluirFoto(...)`
- `npx tsc --noEmit`, `npm run rebuild-fotos-from-ops`
- Migration `v006` with Portuguese tables (`atores`, `operacoes`, `propostas`,
  `conflitos`, `schema_versoes`)
- Version bump `0.4.1 → 0.5.0` for **quilombo-blog**
- Fly apps `quilombo-araucaria` / `quilombo-araucaria-uat`, route `/admin/sync`,
  `exigirPermissao`

`co` is Rust (axum + rusqlite). Its web layer (`co-web`) uses English schema
(`entries`, `schema_version`) and migrations in `co-web/src/storage/migrations/`
(currently up to `v088.rs`) — an entirely different system.

## Where this task actually belongs

| Repo | State (verified 2026-06-21) | Verdict |
|---|---|---|
| `artelonga/quilombo-blog` (named by the task) | **DEPRECATED / archived read-only** since 2026-05-18 (`DEPRECATED.md`: "Do not reference"). `package.json` version is **0.4.0**, not the task's 0.4.1. | Do not implement here — archived, not deployed. |
| `artelonga/quilomboaraucaria` → `web/` (canonical successor, v0.13.0) | The sync feature **already exists**: `src/lib/server/sync/{hlc,merge,ops,index}.ts`, route `src/routes/sync/api/proposta/+server.ts`. `merge.ts` / `ops.ts` already reference `RelatorioMesclagem`, `base_hlc`, `conflito`. | The op-log + 3-way-merge core is already implemented; remaining gap (if any) is `/admin/sync` UI + the foto-specific subset. |

`quilombo-blog`'s own `DEPRECATED.md` states: *"For new work: open PRs against
`artelonga/quilomboaraucaria`."* The Fly apps named in CO-62
(`quilombo-araucaria*`) are served from `quilomboaraucaria/web/`, confirming that
is the live target.

## Conclusion

CO-62 was mis-routed by the co-auto pipeline into the `co` worktree. The correct
home is `artelonga/quilomboaraucaria` (`web/`), where the core sync protocol is
**already implemented** — so this may be largely a duplicate of completed QB-*
work rather than new development.

**No code was written and no `co` source was modified.** The deprecated and
successor repos were left untouched (only read for diagnosis).

## Recommended next action (needs a human decision)

1. **Re-route or close** CO-62 in the `co` backlog — it is not a `co` task.
2. If the photos-only sync subset is still wanted, **audit** the existing
   `quilomboaraucaria/web/src/lib/server/sync/*` against CO-62's 10 scenarios and
   open a `QB-*` task there for the gap (likely just the `/admin/sync` UI).
3. Do **not** implement against `quilombo-blog` — it is archived/read-only.
