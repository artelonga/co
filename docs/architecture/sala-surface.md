# Sala — one surface, fractal scope

**Decision (2026-06-09, Yuri):** there is exactly **one** Sala surface. It is
parameterized by *scope*, never reimplemented per context:

| Scope | Meaning | URL shape |
|---|---|---|
| one universe | canvas anchored to a single universe | `/u/{universe}/sala[/{slug}]` |
| all universes | canvas over every universe the caller can see | `/sala` *(future)* |
| any subset | canvas over an arbitrary set of universes | `/sala?u={a},{b},…` *(future)* |

**Recursive and fractal:** a node on the canvas may be an *entry*, a
*universe*, or another *sala*. Opening a universe node descends into that
universe's own sala — the same surface with a narrower scope. Zooming out
widens the scope. Universes nest (`parent_key`), so salas nest with them.

## The landscape model (CO-410, 2026-06-11)

The surface renders as a **grid landscape**: infinite squares over
deterministic procedural terrain (value noise — same map for everyone, no
assets, no storage). Every square holds a value:

| You type on a square | It becomes |
|---|---|
| a single character | that character, rendered on the square; Enter advances right |
| `/nome` | a **pasta** — a draggable folder unit |
| longer text | a **nota** card |
| (empty) | clears the square |

Notas and pastas drag-and-drop with pointer events (mouse + touch, snap to
grid). A nota dropped on a pasta joins it; the pasta moves as one unit. New
salas seed the root pasta `/` at the origin square.

Layout JSON is v2 — `{ v, cells, notes, folders, nodes, edges, view }` — a
superset of the CO-352 shape, persisted through the same `workspace_states`
PUT. v1 graph layouts (nodes with world x/y) migrate to notas on load with
`id = entry_path`, so saved edges keep resolving. `co-graph.js` is no longer
used by sala.html (graph.html still uses it).

## Universe-as-node — descend / ascend (CO-400, 2026-06-14)

A node on the canvas may be a **universe** (`layout.universes:
[{kind:"universe", key, name, count, x, y}]`), rendered distinctly (teal
world-ring + globe glyph + entry-count badge). Add one from the entry picker's
**Universos** tab (lists the caller's visible universes via `GET
/api/v1/universes`). Activating it **descends** into that universe's own sala —
the same surface at `/u/{key}/sala`, a narrower scope (double-tap the node, or
its panel's *Descer* button).

Descend/ascend is **navigation, not embedding** (the one-surface rule). A
breadcrumb stack in `sessionStorage` (`sala_stack`) remembers each ancestor
sala's camera; the header back-link (`#sala-back`) **ascends** and restores the
originating camera instantly (`sala_restore_cam`), independent of server
persistence — so read-only viewers ascend with camera intact too. Layout JSON
round-trips universe nodes through the unchanged `workspace_states` PUT (the
server stores `layout_json` opaquely).

**Cycles are inert.** Because descend navigates rather than embeds, a sala that
holds its own universe node renders once and never recurses; activating a node
that points at the current universe is a no-op (a toast, no reload).

## Folder-sub-sala ↔ YG room (1:1) (CO-454, 2026-06-14)

A **pasta** node is *descendable* too — the same descend/ascend machinery as a
universe node (CO-400), but it recurses at the **folder** layer instead of the
universe layer. Activating a named pasta (double-tap, or its panel's *Descer*
button) descends into that folder's **sub-sala**: the same universe, a narrower
*slug*.

**Why this exists — converging with Yggdrasil `/mundo` (YG-146).** CO and the
YG content rooms must be **1:1**, but they recursed at different layers: CO
descended by *universe* (a universe node opens *another* universe's sala; a pasta
was mere visual grouping), while YG `/mundo` makes **pasta = sala** — walking
through a door enters a *child folder-room* inside **one** instance. The owner's
**Option A** (2026-06-14) closes the gap by making CO folders descendable
sub-salas (not by promoting every room to a universe — that was Option B / CO-98,
rejected). The resulting map:

| Yggdrasil `/mundo` | CO sala |
|---|---|
| instance | universe (`universe_key`) |
| room (pasta) | folder-sub-sala (slug path) |
| nota | nota |

**Identity = a deeper slug, nothing more (CO-352).** The sub-sala is "just
another slug": descending appends the pasta to the current slug path —
`default` → `default/jardim` → `default/jardim/estufa`. Parent/child is therefore
a slug **prefix**, mirroring the enter/exit nesting YG walks through doors. The
path rides the URL as **one percent-encoded segment** (`default%2Fjardim`), so
`/u/{universe}/sala/{slug}`, the state API, and the realtime WS route all match
unchanged and the server decodes the slash back into the slug. **No new table, no
migration** — `workspace_slug` is opaque TEXT and the UNIQUE
`(universe_key, workspace_slug, user_id)` keeps each depth an independent row.

**Presence is per-sub-sala (CO-353).** The realtime room key
`workspace_id = "{universe_key}/{workspace_slug}"` already accepts the `/` in the
slug, so a pasta's sub-sala has its own roster — 1:1 with a YG `/mundo` room.

**Inert cases** match CO-400: the root pasta `/` (no name) IS the current sala, so
descending it is a soft no-op; the breadcrumb back-link ascends to the parent slug
and restores its camera via `sala_restore_cam`.

**Federation contract (CO-413 ↔ YG-146 Fatia 2).** With both surfaces recursing at
the folder layer, a federated `pos{room,x,y}` is unambiguous: `room` = the pasta
path, `instance` ↔ `universe_key` — no layer mismatch to reconcile on round-trip.

## What this means for implementations

- `co-web/static/shared/sala.html` (CO-352) **is the surface.** All canvas
  rendering, state persistence (`workspace_states`), share tokens, and
  read-only/login gating live there and only there.
- The SPA's *Sala* view tab (CO-355 `modules/views/workspace.js`) is a
  **launcher**, not a canvas: it lists/creates salas (template registry,
  `_workspace.yaml`) and then navigates to the surface at
  `/u/{universe}/sala/{slug}`. It must never grow its own canvas.
- Godot/Yggdrasil render the same scope model when a universe is played as a
  2D world — the sala is the universal spatial lens, the game is one renderer
  of it.

## Anti-goals

- No per-context canvas forks (SPA canvas vs page canvas vs mobile canvas).
- No scope baked into storage: `workspace_states` rows key on
  `(universe_key, workspace_slug, user_id)` today; multi-universe scopes will
  extend the key, not duplicate the table.

## Open work

- All-universes and subset scopes (URL shapes above) — needs a CO-N task.
- ~~Universe-as-node + descend/ascend navigation~~ — done (CO-400).
- ~~Folder-as-sub-sala (pasta=sala, 1:1 with YG `/mundo` rooms)~~ — done (CO-454).
- CO-354 (suggest/review) operates on the one surface's state, scope-agnostic.
