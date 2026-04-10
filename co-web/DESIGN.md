# CO Design Conventions

Design principles and tonal conventions for the CO platform UI themes.

---

## Named Palettes

CO ships four **named palettes** (Scholarly Automaton light/dark, Relic Archive light/dark) and eight additional variants. Named palettes enforce specific aesthetic conventions; generic variants fall through to base defaults.

| Palette key | Alias | Character |
|-------------|-------|-----------|
| `scholarly` | Scholarly Automaton — Light | Parchment, warm brass, serif ledger |
| `scholarly-dark` | Scholarly Automaton — Dark | Inkwell, bronze glow, deep warm dark |
| `relic` | Relic Archive — Dark | Near-black steel, crimson accent, glass surfaces |
| `relic-light` | Relic Archive — Light | Rose dust, crimson ink, clean serif |

---

## Core Design Rule: Tonal Shift Over Hard Lines

**Named palettes (Scholarly, Relic) must NOT use hard borders for structural separation.**
Use tonal gradient shifts, box-shadow elevation, and surface layering instead.

### Applies to:
- Header: no `border-bottom` → use `box-shadow` and/or gradient background
- Sidebar: no `border-right` → use `box-shadow`
- Sidebar footer: no `border-top` → rely on tonal contrast of sidebar-bg
- Kanban columns: no gap borders → use tonal background shift (e.g., `--sidebar-bg` or one step lighter/darker)

### Permitted exceptions:
- Modal overlay header: thin internal separator within the modal surface (distinct from structural chrome)
- Priority indicator: thin left border on task cards (`3px` only, left side only — data visualization, not structural)
- Form inputs (Scholarly): bottom-border-only input style (ledger convention, not structural chrome)

### Why:
Hard borders conflict with the editorial/archival aesthetic of Scholarly and Relic. These palettes use ink-on-paper and metal-plate metaphors where surfaces are distinguished by material depth, not ruled lines.

---

## Typography

Both named palette families share a typography hierarchy:

| Role | Scholarly | Relic |
|------|-----------|-------|
| Body / headline | Newsreader (Georgia fallback) | Newsreader (Georgia fallback) |
| Labels / UI | Work Sans (system-ui fallback) | Manrope (system-ui fallback) |
| Mono | system monospace | system monospace |

### Typography rules:
- `#project-name`: Newsreader italic 20px/600 (both palettes)
- `.task-title`: Newsreader 14px/600 (both palettes)
- `.sidebar-section-header` text: Work Sans 10px, uppercase, 0.15em tracking (Scholarly only)
- `.form-group label`, `.btn`, `.view-tab`: font-label with letter-spacing 0.04em (Scholarly)
- Modal `h2`: Newsreader italic, accent color

---

## Surface Elevation Model

Surfaces are ordered from deepest to lightest:

```
sidebar-bg (deepest)
  └── bg (main background)
        └── bg-hover / kanban-column-bg
              └── card-bg (task cards)
                    └── modal-surface (floating/elevated)
```

### Dark mode specifics:
- Sidebar is noticeably deeper than main content area
- Cards lift slightly above bg via `--shadow-sm`
- Modals use `--modal-surface` + `--shadow-lg`
- Relic dark modals add `backdrop-filter: blur(20px)` for glass effect

---

## Form Input Conventions

| Palette | Style | Border | Background |
|---------|-------|--------|------------|
| Scholarly (both) | Ledger / underline | Bottom-only, 1px, text-muted → accent on focus | Transparent |
| Relic dark | Contained | 1px crimson 20% → gold (#e9c349) on focus | Semi-transparent dark |
| Relic light | Contained | 1px crimson 18% | Very light crimson tint |
| Other palettes | Box | Variant-defined | Variant-defined |

---

## Button System

| Variant | Scholarly | Relic dark | Relic light |
|---------|-----------|------------|-------------|
| Primary | Brass accent + inner highlight glow | Gradient silk: `#ffb3b5 → #e0505f`, dark text | Crimson accent, soft shadow |
| Secondary | Ghost, 15% opacity border | Ghost, very low opacity | — |

---

## Interactive States

### Sidebar items (Scholarly + Relic):
- Hover: `translateX(4px)` translate only, no background fill
- Active: accent-colored text + low-opacity accent background tint

### View tabs (Scholarly + Relic):
- Container: pill-group shape (`border-radius: 99px`)
- Active: Scholarly → brass accent fill; Relic dark → crimson tint; Relic light → accent-light fill

### Task cards (Scholarly + Relic):
- Padding: asymmetric `12px 10px 12px 14px` (extra left room for priority strip)
- Borders: left only (3px priority color) — no top, right, or bottom borders
- Hover: tonal background shift (one step warmer/cooler), `--shadow-md` lift

---

## Status Badges

| Palette | Shape | Styling |
|---------|-------|---------|
| Relic dark | Pill (`border-radius: 99px`) | Colored bg + matching text from `--status-*-bg/text` tokens |
| Scholarly dark | Rectangular | `--status-*-bg/text` tokens |
| Others | Variant default | Variant-defined |

---

## Login Screen

| Palette | Screen background | Card treatment |
|---------|-------------------|----------------|
| Scholarly light | Radial gradient parchment (#FFF9ED → #EAD9BF) | `--modal-surface`, warm border, no backdrop blur |
| Scholarly dark | Radial gradient inkwell (#1c1610 → #0d0a07) | Dark warm bg, no backdrop blur |
| Relic dark | Solid near-black (#131313 95%) | Dark glass (same surface as modal) |
| Relic light | Flat rose-dust (#F5F0F0) | White card, crimson border, no backdrop blur |

---

## Adding New Theme-Specific Overrides

1. Add overrides under the appropriate `[data-palette="..."]` selector in `style.css`
2. Use existing CSS custom property tokens — do not hard-code colors
3. Never introduce `border-top/right/bottom` on structural chrome (header, sidebar) for Scholarly/Relic
4. Follow the surface elevation model — elevate via shadow, not border
5. Test all 12 palettes after any change to shared component rules
