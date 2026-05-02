# CO-150 — Asset Browser Demo

## Feature Overview

CO-150 (Phase 5 of CO-145) delivers the user-facing lazy-load integration for the CO SPA.

### Asset Browser (`/co/{u}/assets`)

The asset browser shows all binary assets uploaded to a universe as a grid of thumbnails:

- **Image assets** render inline with `<img loading="lazy" decoding="async">` — thumbnails are fetched on scroll, not on page load.
- **Video and other assets** display a MIME-type icon.
- **Filter bar** at the top: filter by MIME type prefix (`image/`, `video/`, `application/pdf`) or search by filename.
- **Click any card** to open a detail modal:
  - Full-size preview (image inline, video with `preload="none"`)
  - sha256 hash (copy button)
  - Size, MIME type, creation date, refcount
  - Ready-to-paste markdown syntax (`![alt](sha256:…)` or ` ```video ` block)
  - Delete button (only shown when `refcount == 0`)

### Lazy-load in Board View

Images referenced in markdown as `![alt](sha256:abc…)` now render as:
```html
<img src="/api/v1/universes/KEY/assets/abc…" alt="alt" loading="lazy" decoding="async">
```

Initial board paint loads only text (frontmatter + 200-char excerpt via `?excerpt=true`). Image bytes are fetched as the user scrolls — satisfying the Lighthouse lazy-load images audit.

### Video Shortcode

````markdown
```video
sha256:abcdef0123456789…
```
````

Renders as `<video src="…" preload="none" controls>` — the file is never pre-buffered; playback starts on click.

### Drag-and-Drop Upload

Drag an image or video onto the CodeMirror editor (or paste from clipboard) to:
1. Upload to `POST /api/v1/universes/{key}/assets`
2. Automatically insert `![filename](sha256:…)` or a ` ```video ` block at the cursor

### API Changes

```
GET  /api/v1/universes/{u}/assets              → { assets, total }
GET  /api/v1/universes/{u}/entries/{p}?excerpt=true  → { frontmatter, excerpt }
```

### Screenshot

> **Note:** Replace this file with an actual screenshot at `docs/co-150-asset-browser.png`.
> Capture: open `/co/{slug}/assets` in a universe with several uploaded images,
> showing the thumbnail grid + one open detail modal.
