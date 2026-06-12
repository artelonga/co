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
- Universe-as-node + descend/ascend navigation — needs a CO-N task.
- CO-354 (suggest/review) operates on the one surface's state, scope-agnostic.
