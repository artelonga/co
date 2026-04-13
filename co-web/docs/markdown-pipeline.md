# CO Markdown Rendering Pipeline

> **CO-39** — Unified markdown pipeline for the CO web app.

## Contract

All markdown text in CO flows through **one renderer**: `marked` (GFM) + `DOMPurify` (XSS sanitization). The same source produces identical HTML in every surface.

```
Markdown source (UTF-8)
  │
  ├─ Cards (kanban, conteudo)  → extractFirstParagraph() → plain text, no markup
  │
  ├─ Card body preview         → renderMarkdown() → sanitized HTML + md-fade CSS
  │
  ├─ Content viewer            → renderMarkdown() + resolveWikilinks() + highlightCode()
  │
  ├─ CodeMirror editor         → live preview pane (right pane of split view)
  │
  └─ Obsidian plugin           → Obsidian's native renderer (source preserved as-is)
```

## Files

| File | Purpose |
|------|---------|
| `co-web/static/shared/markdown.js` | Main module — exposes `window.CoMarkdown` |
| `co-web/editor/src/editor.js` | Source: `renderMarkdown`, utility exports bundled into `window.CoEditor` |
| `co-web/static/shared/editor.bundle.js` | Built bundle — exports `renderMarkdown`, utilities via `window.CoEditor` |

## API

### `window.CoMarkdown`

All functions are available synchronously. `renderMarkdown` delegates to `window.CoEditor.renderMarkdown` (marked + DOMPurify) when the editor bundle is loaded; falls back to a lightweight paragraph renderer otherwise.

```js
// Sanitized HTML from markdown source
CoMarkdown.renderMarkdown(text, opts?)

// Plain text of first content line (markdown syntax stripped)
// Use this for kanban card previews — never shows raw ** or \n
CoMarkdown.extractFirstParagraph(text)

// YAML frontmatter + body separation
// Returns { frontmatter: Record<string,string>, body: string }
CoMarkdown.extractFrontmatter(text)

// Text statistics (body only, frontmatter excluded)
CoMarkdown.wordCount(text)     // integer
CoMarkdown.readingTime(text)   // minutes (integer, min 1)
CoMarkdown.headingCount(text)  // number of # headings

// Post-processing (call after setting innerHTML)
CoMarkdown.resolveWikilinks(html, universeSlug)  // [[wikilinks]] → <a>
CoMarkdown.highlightCode(container)              // prismjs, CDN, lazy-loaded
CoMarkdown.enableImageZoom(container)            // lazy-load + click-to-zoom
```

### `window.CoEditor` (from editor bundle)

The editor bundle exposes the same markdown utilities. These are canonical implementations; `markdown.js` delegates to them when available.

```js
CoEditor.renderMarkdown(src)          // marked + DOMPurify (full GFM)
CoEditor.extractFrontmatter(text)
CoEditor.extractFirstParagraph(text)
CoEditor.wordCount(text)
CoEditor.readingTime(text)
CoEditor.headingCount(text)
CoEditor.initEditor(container, opts)  // CodeMirror 6 editor
```

## Loading Order

```html
<script src="/shared/i18n.js?v=1"></script>
<script src="/shared/markdown.js?v=1"></script>  <!-- window.CoMarkdown available immediately -->
<script src="/app.js"></script>
<!-- editor.bundle.js loaded lazily on first editor open -->
```

`markdown.js` loads synchronously and provides fallback implementations. When `editor.bundle.js` is loaded (lazily, on first editor use), `window.CoEditor` is populated and `CoMarkdown.*` automatically delegates to the full implementations.

## Rendering Surfaces

### Kanban Cards

`renderTaskCard()` calls `CoMarkdown.extractFirstParagraph(task.description)` for the description preview. Result is plain text (no HTML), escaped with `esc()` before insertion.

**Before:** Raw markdown leaked into cards (`**bold**`, `## heading`, `\n` escapes visible).  
**After:** Clean first-paragraph text, stripped of all markdown syntax.

### Content Cards (Conteudo View)

`renderConteudo()` awaits `loadEditorBundle()` then calls `CoMarkdown.renderMarkdown(e.body)` for each entry body. The HTML is inserted with the classes `md-body md-fade`:

- `md-body` — markdown typography styles (paragraphs, lists, code, links)
- `md-fade` — limits to ~6 lines with a gradient fade-out

Code blocks in cards: monospace font, horizontal scroll (`overflow-x: auto`). No syntax highlighting in cards (deferred to the viewer).

### Content Viewer

`openContentViewer(entry)` renders the full entry body:

1. `renderMarkdown(body)` — full GFM (headings, tables, lists, code, images)
2. `resolveWikilinks(html, slug)` — `[[Title]]` → `<a href="/co/slug/entries/Title">`
3. Tables wrapped in `.co-table-wrap` for responsive horizontal scroll on mobile
4. `enableImageZoom(container)` — `loading="lazy"` + click-to-zoom overlay
5. `highlightCode(container)` — PrismJS from CDN, lazy-loaded on first use

### CodeMirror Editor

The editor (`editor/src/editor.js`) shows a split view: left pane (CodeMirror) + right pane (live preview). The Preview toolbar button toggles the right pane. Auto-save drafts to localStorage every 5 seconds using keys:

- Task modal: `co_draft_task_{id}` or `co_draft_new_task`
- Content editor: `co_draft_task_{id}`
- Page editor: `co_draft_page_{encodedPath}`

Drafts are cleared on successful save.

### Obsidian Plugin (CO-34)

The Obsidian plugin preserves markdown source exactly — it does not render. Obsidian uses its own native renderer. The CO server stores markdown as-is; the plugin syncs source files.

## Cross-Platform

| Platform | Renderer | Notes |
|----------|----------|-------|
| Web browser | `marked` + DOMPurify | canonical |
| Capacitor (CO-36) | same — pure browser JS | no Node required |
| Electron (CO-35) | same — pure browser JS | no Node required |
| Obsidian plugin | Obsidian native | source preserved, not rendered |

## Security

All rendered HTML passes through DOMPurify with the allowlist defined in `editor/src/editor.js`:

```js
ALLOWED_TAGS: ['p','br','strong','em','s','del','code','pre','blockquote',
  'h1'–'h6','ul','ol','li','table','thead','tbody','tr','th','td',
  'a','img','hr','input','span','div']
ALLOWED_ATTR: ['href','src','alt','title','type','checked','disabled',
  'class','id','loading','data-zoom']
```

`extractFirstParagraph` returns plain text only — no HTML tags — always safe with `esc()`.

## Bundle Size

| Dependency | How loaded | Size |
|------------|-----------|------|
| `marked` | bundled in `editor.bundle.js` (lazy) | ~50KB gz |
| `DOMPurify` | bundled in `editor.bundle.js` (lazy) | ~7KB gz |
| `markdown.js` | synchronous, no deps | ~4KB |
| PrismJS | CDN, lazy (viewer only) | ~7KB gz + languages |

`markdown.js` itself is < 5KB. The editor bundle is lazy-loaded; it does not block initial page render. PrismJS is loaded only when the viewer opens a page with code blocks.

Total synchronous overhead added by CO-39: **< 5KB** (just `markdown.js`).
