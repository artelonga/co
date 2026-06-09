# Universe public site — board (app) vs static site (construir)

One universe's markdown content, rendered two ways:

| Surface | URL | Audience | Tech |
|---|---|---|---|
| **Board** (interactive: kanban/table/graph, editing, conflicts) | `co.artelonga.com.br/<slug>` | logged-in / subscribed (gated by `visibility` + access) | CO SPA (co-web) |
| **Public site** (read-only rendered wiki/garden) | `<slug>.artelonga.com.br` | anyone (public) | **static site** |

Same content (`<repo>/content/*.md`), two presentations. The board is the CO app;
the public site is a **static build** — no CO chrome, no login, no editing.

## Current state (review)

- **Subdomains today serve the SPA board**, not a static site (`server/subdomain_routing.rs`
  resolves `<slug>.artelonga.com.br` → injects the universe key → serves the app).
- **co-web has no server-side markdown→HTML render** — rendering is client-side
  (`static/shared/markdown.js`). There is no SSR/reader mode.
- **Static public sites are bespoke per-universe apps**: `retro-umarizal` ships its own
  `deploy/fly.toml` + `tools/retro-server.mjs` (hand-authored HTML on Fly app
  `artelonga-retro` at `retroumarizal.artelonga.com.br`); `quilombo-blog` is a separate
  SvelteKit app. Not generalized, not generated from the universe's markdown.
- **The canonical generator already exists**: `redearte` is a **Quartz** instance
  (`jackyzha0/quartz`) — markdown → static digital-garden site with native
  `[[wikilink]]`, backlink, and graph rendering. This is the `construir` (build) tool the
  glossary promises but no `co build` command wires up yet.

So: the board exists; the static-site renderer (Quartz/redearte) exists; the **pipeline
connecting a universe's content to a deployed static site does not**.

## Proposed: `construir` — universe markdown → Quartz static site

```
   <repo>/content/*.md                     (markdown — the single source)
        │                                   │
        │ co launch / sync                  │ construir (Quartz, redearte template)
        ▼                                   ▼
   CO board (co-web)                    static digital-garden site
   co.artelonga.com.br/<slug>           Fly app artelonga-<slug>
   gated, interactive                   <slug>.artelonga.com.br, public, read-only
```

1. **Build** — `co construir <universe>` (a.k.a. `co build`) feeds the universe's
   `content/` markdown through the **redearte Quartz template** → static HTML
   (`public/`). Quartz renders the wiki we built natively: the map-of-content, the
   `[[wikilinks]]` as navigation, backlinks, and the graph — as a static garden.
2. **Deploy** — serve `public/` from a Fly static app `artelonga-<slug>` (the
   `retro-umarizal/deploy/` pattern: 256 MB `shared-cpu-1x`, `gru`, cert for
   `<slug>.artelonga.com.br`). No volume (pure static).
3. **Route** — point `<slug>.artelonga.com.br` DNS at the static app, and **drop that
   slug from co-web's subdomain SPA routing** so the subdomain is the site, not the
   board. The board stays at `co.artelonga.com.br/<slug>` (gated). (Exactly retro's
   split: `retroumarizal.artelonga.com.br` = site, `co…/retro-umarizal` = board.)

Privacy carries over: only published/public entries are built into the static site; the
`_source/` PII originals are never in `content/`, so never in the build.

## First target: grcsamazonia

- `grcsamazonia.artelonga.com.br` → Quartz static garden of the founding docs + operacional
  + cultural (the interlinked wiki).
- `co.artelonga.com.br/grcsamazonia` → board, for the diretoria / subscribers.

## Applying to the existing universes

| Universe | Board (co.artelonga.com.br/…) | Public static site |
|---|---|---|
| `artelonga` | ✅ app | agency site (today bespoke) → could move to construir |
| `quilomboaraucaria` | ✅ app | `quilombo-blog` (SvelteKit) — keep, or converge on construir |
| `comunicacao` | ✅ app | Quartz garden (lexicon) — strong fit |
| `yggdrasil` | ✅ app | notes garden — fit |
| `retro-umarizal` | ✅ app | `artelonga-retro` (bespoke HTML) — the precedent construir generalizes |

construir gives every universe a public garden from its markdown **without a bespoke app**.

## Alternative considered (lighter, not chosen)

A server-side **reader mode** in co-web (SSR clean HTML at the subdomain, no board chrome)
avoids a separate deploy but needs a new SSR render path in co-web and isn't truly
static. Quartz/redearte is preferred: it's the existing tool, produces genuinely static
output, and already does garden rendering. Tracked in **CO-395**.
