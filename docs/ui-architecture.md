# Composable universe UI — content lenses vs schema-driven forms

A revamp of the `co-web` SPA (`static/variants/a/`) so that **every universe feature is
a composable module**, the UI is **organized around the read/write split**, and a
universe's surface is **declared by its manifest** (`_universe.yaml`) rather than
hand-wired in JS. New content types and child universes compose in with **no UI code**.

## Current state (review)

| Area | Today | Verdict |
|---|---|---|
| **Content (read)** | `modules/views/`: `kanban`, `table`, `timeline`, `calendar`, `dashboard`, + a **74 KB `conteudo.js`** monolith | lens-based already, but the registry is partly hardcoded |
| Lens registry | `app.js injectManifestViewTabs` (CO-73) injects tabs **only for `manifest.views[].type === 'gantt'`** | seed of manifest-driven composability, not generalized |
| **Forms (write)** | `modules/modals.js`: bespoke hand-coded HTML with inline styles per form (`<input name=… style=…>`, `<select>`) | **no schema→form generation — the main gap** |
| Schemas | `_universe.yaml` `content_types[].schema` (e.g. `title: {type, required}`) already declare field shapes | unused by the form layer |
| Shell | `sidebar/`, `breadcrumbs`, `state/`, theme, universe switcher | solid; needs child-universe hierarchy |

**The asymmetry:** content is composable lenses; forms are one-off HTML. The schemas
that *should* drive forms already exist in the manifest but only feed (partial) lens
tabs. Closing that is the revamp.

## The model: Shell · Lenses · Form engine — all manifest-driven

```
┌──────────────────────── Universe Shell ────────────────────────┐
│  sidebar (universe tree: parent_key → children)  ·  theme  ·    │
│  breadcrumbs  ·  lens tabs (from manifest.views)                 │
│ ┌─────────────── CONTENT (read) ───────────────┐  ┌───────────┐ │
│ │  Lens registry — pick by manifest + type:     │  │  FORM      │ │
│ │  board · table · timeline · calendar · graph · │  │  engine    │ │
│ │  document · dashboard                          │  │ (write)    │ │
│ │  each lens = (entries, content_type) → render  │  │ schema →   │ │
│ └────────────────────────────────────────────────┘  │  fields    │ │
│                                                       └───────────┘ │
└──────────────────────────────────────────────────────────────────┘
                  one Storage / REST + Vault API (see universe-crud.md)
```

### 1. Content lenses (read) — a registry, fully manifest-driven

- A **lens** is `(entries, contentType, viewState) → DOM`. The existing `views/*`
  become registered lenses behind a uniform interface: `{ id, label, icon, supports(type), render() }`.
- Generalize `injectManifestViewTabs`: a universe's visible lenses come from
  `manifest.views` + which `content_types` it declares — not a hardcoded `gantt` check.
  A universe with only `note` entries shows document/table/graph; one with `task`
  shows board/timeline; etc.
- Split the `conteudo.js` monolith into `lenses/document.js` + shared rendering, per
  the 500-LoC rule (server `MODULES.md` analog for the SPA).

### 2. Form engine (write) — schema-driven, replaces bespoke modals

- One `form/engine.js`: `renderForm(schema, value?) → <form>` + `collect(form) → value`,
  where `schema` is a `content_types[].schema` from the manifest (field → `{type,
  required, enum, label}`).
- Field renderers per type (`string`, `text`, `number`, `date`, `enum`, `ref`,
  `boolean`). `ref` renders a cross-universe entry picker (the grafo).
- **Every CRUD form becomes a config, not code:** entry create/edit, universe
  create/edit, member add, `ficha-cadastro`. Adding a `content_type` to `_universe.yaml`
  yields a working create/edit form **and** a matching lens with zero JS.
- Bespoke forms in `modals.js` (branch ops, etc.) migrate onto the engine incrementally.

### 3. Universe shell — composes children

- The sidebar renders the **`parent_key` hierarchy** (CO-98): a universe and its child
  universes as a tree; switching is composition, not navigation away.
- Lenses may span universes — the graph lens uses the multi-universe API
  (`GET …/graph?universes=a,b,c`, CO-345), so a child universe's content shows in the
  parent's network (e.g. `grcsamazonia` ↔ `quilomboaraucaria` ↔ `artelonga`).

## Organizing principle: content vs form

| | Content (read) | Form (write) |
|---|---|---|
| Driven by | entries + `content_types` | `content_types[].schema` |
| Unit | **lens** (`views/` → `lenses/`) | **field renderer** + form engine |
| Registry | manifest `views` + type support | schema fields |
| Composability | new type → eligible lenses light up | new type → create/edit form generated |

The two share the manifest as the single source of truth, so a universe's *whole*
surface — what you can see and what you can edit — is declared, not coded.

## Composability contract

A universe module = `{ manifest (_universe.yaml), entries, theme }`. The shell reads the
manifest and composes: lenses (from `views` + types), forms (from `content_types`),
child universes (from `parent_key`). **No universe needs bespoke UI** — `grcsamazonia`,
`retro-umarizal`, `comunicacao` all render from their manifest. Bespoke surfaces
(retro's standalone site) remain opt-in *additions*, not the default path.

## Migration (incremental, low-risk)

1. **Lens interface** — wrap existing `views/*` in the registry; generalize tab
   injection beyond `gantt`. (No visual change.)
2. **Form engine** — build `form/engine.js`; migrate entry create/edit first, then
   universe create/edit, then the modal grab-bag.
3. **Child hierarchy** — sidebar tree + cross-universe lens spans.
4. **Split `conteudo.js`** into `lenses/`.

Each step ships behind the existing `variants/` mechanism so `a` stays stable while a
new composable variant is proven, then promoted. Tracked in **CO-393**.
