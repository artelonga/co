# Architecture review — SPA modules + composition

**Asked:** *"review documentation and modules for composition over inheritance (single responsibility), since we have a 500 line app.js that's unclear what it does at all."*

Snapshot (2026-05-16):

```
co-web/static/variants/a/
  app.js              705   ← entry point, orchestrates everything
  modules/
    modals.js         987   ← every modal in the app
    login.js          779   ← every auth flow
    chat.js           669
    sidebar.js        515
    conversas.js      390
    invitations.js    366
    settings.js       321
    yggdrasil.js      319
    notifications.js  280
    helpers.js        251
    onboarding.js     249
    notification-settings.js 200
    api.js            222
    boot.js           174
    state.js           53
    constants.js      153
    push.js            92
    conversas-welcome.js 67
    dm.js             222
  TOTAL             7014
```

The pain isn't total size — it's that **app.js does ~12 things** with no clear seam between them. Below: the SRP violations, the proposed decomposition, and an incremental refactor plan that doesn't require a stop-the-world rewrite.

---

## 1. What `app.js` actually does today

Roughly, in order of appearance:

| Concern | Lines (approx) | Belongs where? |
|---|---|---|
| Imports + state aliases | 1–80 | unchanged |
| Universe + template banner DOM helpers (`showTemplateBanner` / `hideTemplateBanner`) | 100–125 | `modules/template-banner.js` |
| URL parsing (`readUniverseSlugFromUrl`, `readEntryPathFromUrl`, `readGameFromUrl`) | 126–145 | `modules/url.js` |
| `ensureOwnUniverse` (clone-or-redirect anon visitors) | 147–177 | `modules/anon-flow.js` |
| `openContentEditor` (legacy task editor) | 178–250 | move to `modules/views/content-editor.js`; the inline detail pane already does most of this |
| Deep-URL handlers (`maybeOpenPageFromUrl`, `maybeOpenEntryFromUrl`) | 247–290 | `modules/url.js` |
| `switchView` (kanban/table/calendar/timeline/dashboard/conteudo) | 290–340 | `modules/view-router.js` |
| `selectProject` + `refreshTasks` | 354–380 | `modules/project.js` |
| Callback wiring (`injectXxx` for every module) | 390–445 | `modules/wire-modules.js` |
| `bindStaticEvents` (header clicks, keyboard shortcuts) | 480–540 | `modules/hotkeys.js` |
| `init()` — the giant orchestrator | 555–705 | stays in app.js but slimmed |

**The orchestrator (init) is the load-bearing function.** It's where the universe-routing decision tree lives: anon vs logged-in × template vs owned × deep URL vs root, etc. Today it's ~150 lines of nested `if`s.

---

## 2. Single-responsibility violations — concrete examples

### app.js does URL parsing + auth + bootstrap + side-effects in one path

The auth check, universe detection, banner toggling, login modal, OAuth flow, and entry-resolution all live in `init()` between lines 555–700. Easy to get lost. A regression like the recent template-redirect-on-deep-URL bug (2.7.19) hides inside the nested conditionals.

### `modules/login.js` (779 lines) is auth + signup + recovery + handover

Three distinct flows ride together:

- email magic-code login (CO-188)
- password login (CO-85)
- cross-domain handover (CO-205 / CO-206)
- passwordless onboarding (CO-190)

Each has its own state machine; one giant file makes the seams invisible. Same shape as `modals.js`.

### `modules/modals.js` (987 lines) is every modal

Task modal, universe info modal, login modal, asset upload, settings, invitations… all in one file. Each modal owns its DOM, state, and lifecycle but they share nothing meaningful. The file is a directory pretending to be a module.

### Callback injection is the de-facto IoC

Notice the pattern in app.js around lines 390–445:

```js
injectBootCallbacks({ showLoading, hideLoading, render, selectProject, ... });
injectKanbanCallbacks({ openTaskModal, ensureOwnUniverse, renderKanban, showToast, renderContent });
injectCalendarCallbacks({ openTaskModal, openZoomModal, apiFetch });
// 8 more lines like this
```

Each module exports `injectXxxCallbacks(fn)` to break a circular import. It works, but the wiring isn't typed and the dependency graph isn't visible anywhere — you have to read the imports + the inject calls + the receivers to reconstruct it. **This is composition, but it's not legible.** A central `wire-modules.js` (or a tiny DI registry) would make the graph explicit.

---

## 3. Target module structure

Same code, less narrative debt:

```
co-web/static/variants/a/
  app.js                 (~120 lines: parse URL → pick boot strategy → wire modules)
  modules/
    state.js             (unchanged, the shared mutable state)
    constants.js         (unchanged)
    helpers.js           (move pure utility fns; keep small)
    api.js               (unchanged, HTTP client)

    url.js                  ← readUniverseSlugFromUrl, readEntryPathFromUrl, readGameFromUrl, deep-URL handlers
    view-router.js          ← switchView + the layoutToView map
    anon-flow.js            ← ensureOwnUniverse + template banner toggles
    project.js              ← selectProject + refreshTasks
    hotkeys.js              ← keyboard shortcuts (currently in bindStaticEvents)
    wire-modules.js         ← all the injectXxxCallbacks calls in one place

    auth/
      login-email.js        ← magic-code flow
      login-password.js     ← password flow
      handover.js           ← cross-domain JWT handover
      onboarding.js         ← passwordless onboarding

    views/
      kanban.js, table.js, calendar.js, timeline.js, dashboard.js, conteudo.js  (existing)
      content-editor.js     ← extracted from app.js's openContentEditor

    modals/
      task.js               ← from modals.js
      universe-info.js
      login.js
      assets-upload.js
      settings.js
      invitations.js
      ...

    chat.js, dm.js, conversas.js  (existing)
    notifications.js
    invitations.js
    push.js
    sidebar.js
    yggdrasil.js
    settings.js (palette + theme persistence — current 321-line file is reasonable)
```

**Two principles applied here:**

1. **One file = one user-visible thing.** A modal, a view, a flow. If you can name it in a single noun-phrase, it's a module.
2. **No god-files.** `app.js` becomes orchestrator-only. `modals.js` becomes a directory.

---

## 4. Migration plan — incremental, never a stop-the-world

The current code works; ripping it apart in one commit is asking for regression. Pull threads one at a time.

### Phase 1: extract pure helpers (zero risk)

These have no shared state with the rest of app.js. Move them, update imports, ship.

- `readUniverseSlugFromUrl`, `readEntryPathFromUrl`, `readGameFromUrl` → `modules/url.js`
- `showTemplateBanner` / `hideTemplateBanner` → `modules/template-banner.js`

### Phase 2: extract view-routing (low risk)

`switchView` + `layoutToView` + the view-tab DOM toggles all move together to `modules/view-router.js`. app.js loses ~80 lines.

### Phase 3: split `modals.js` (medium risk)

Each modal is its own file under `modules/modals/`. The current `modals.js` becomes a re-export barrel that preserves the existing import paths. Once callers migrate to deep imports, remove the barrel.

### Phase 4: split `login.js` (medium risk)

Same shape as modals. Each auth flow → its own file. `auth/index.js` is the barrel.

### Phase 5: typed wire-modules (real refactor)

A new file `modules/wire-modules.js` that takes a `{ all callbacks }` object and dispatches `injectXxx(callbacks)` to each module. The dispatch list IS the dependency graph — readable in one place.

```js
export function wireModules(callbacks) {
    injectBootCallbacks(callbacks);
    injectKanbanCallbacks(callbacks);
    injectModalsCallbacks(callbacks);
    // ...
}
```

Each `inject*` function destructures only the keys it needs — duck-typed but discoverable.

### Phase 6 (optional, later): TypeScript

Once the file structure stabilizes, the value of types compounds (operationId codegen from OpenAPI, typed state, etc.). Not urgent.

---

## 5. Documentation gaps

What's missing besides modules:

- **Module map.** No single doc says "this is what every file owns." This review is the start; a permanent `co-web/static/variants/a/README.md` would carry it forward.
- **State contract.** `modules/state.js` has the shape but no narrative about what each field means or who's allowed to mutate it. A short comment per field would help.
- **Decision log for naming.** Why `template` instead of `modelo`? Why `co::public/*` instead of moving content? These conversations live in CHANGELOG entries — promote to a `docs/decisions/*.md` directory (CO already has a placeholder reference in the infra catalog).

---

## 6. What I'd ship first

If you want to act on this:

1. **Phase 1 + Phase 2** (URL + view-router extraction) — one ~150-line commit, app.js drops by ~120 lines, no behavior change. Tests untouched.
2. **Module map README** — half-a-day docs work; one file under `co-web/static/variants/a/README.md` listing every module + its responsibility.

After those two land, the next refactors get easier because the seams are visible.

**This review itself doesn't refactor anything.** Tell me which phase to start, and I'll send a PR-shaped patch (one Phase per release).
