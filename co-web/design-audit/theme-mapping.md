# Theme Audit — Component × Token × Override Mapping

Generated as part of CO-40 pre-spec preparation.
Source: `co-web/static/variants/a/style.css` (lines 3005–3990, 4028–4290)

---

## Summary

The CSS has **four named palette blocks** (scholarly, scholarly-dark, relic, relic-light)
plus **eight additional theme blocks** (medieval, steampunk, cyberpunk, matrix, garden,
terminal, retro, and the default Modern).

Named palettes (Scholarly + Relic) receive the most extensive component-level overrides.
Other palettes set tokens only and fall through to default component rules.

---

## 1. CSS Custom Properties per Palette

### 1a. Base tokens (all palettes define these)

| Token | Description |
|-------|-------------|
| `--bg` | Page background |
| `--bg-hover` | Hover state background |
| `--sidebar-bg` | Sidebar background |
| `--sidebar-hover` | Sidebar item hover |
| `--sidebar-active` | Sidebar active item accent |
| `--card-bg` | Card / task card background |
| `--border` | Default border color |
| `--text-primary` | Primary text |
| `--text-secondary` | Secondary text |
| `--text-muted` | Muted / placeholder text |
| `--accent` | Primary accent color |
| `--accent-hover` | Accent hover state |
| `--accent-light` | Light accent (bg tints) |
| `--danger` | Danger / destructive |
| `--danger-hover` | Danger hover |
| `--font` | Body font stack |
| `--font-label` | Label/UI font stack |
| `--radius-sm / -md / -lg` | Border radii |
| `--shadow-sm / -md / -lg` | Shadow levels |
| `--modal-overlay` | Modal backdrop color |
| `--modal-surface` | Modal background |
| `--form-input-bg` | Input background |
| `--form-input-border` | Input border |
| `--form-input-border-focus` | Input focus border |
| `--form-input-radius` | Input corner radius |
| `--form-input-padding` | Input padding |

### 1b. Status tokens (scholarly-dark, relic, relic-light define; others may not)

| Token | Description |
|-------|-------------|
| `--status-todo` | To-do status bar color |
| `--status-in_progress` | In-progress bar color |
| `--status-in_review` | In-review bar color (scholarly-dark only) |
| `--status-done` | Done bar color |
| `--status-{status}-bg` | Status badge background |
| `--status-{status}-text` | Status badge text |

### 1c. MD3 alias tokens (scholarly + relic only)

Full `--md-surface-*`, `--md-on-surface-*`, `--md-primary-*`, `--md-outline-*`,
`--md-secondary`, `--md-tertiary`, `--md-background` tokens set for MD3 component compat.

---

## 2. Component × Override Mapping

Legend: ✓ = overrides default; – = falls through to default; (tok) = token-only, no selector override

| Component | scholarly | scholarly-dark | relic | relic-light | other themes |
|-----------|-----------|----------------|-------|-------------|--------------|
| **Page background** | gradient vignette | gradient vignette | gradient vignette | – | (tok) |
| **Header bg** | gradient, no border | gradient+border | gradient+blur, no border | gradient, no border | (tok) |
| **Header shadow** | warm 10% | brass 55% | deep black 50% | rose 7% | (tok) |
| **Sidebar bg** | no border-right, warm shadow | deep bg, no border | near-black, no border | no border-right, rose shadow | (tok) |
| **Sidebar logo** | – | brass separator | crimson separator | – | – |
| **Sidebar footer** | no border-top | – | – | no border-top | – |
| **Sidebar items hover** | translateX(4px), no bg | translateX(4px), no bg | translateX(4px), no bg | translateX(4px), no bg | – |
| **Sidebar items active** | – | brass text + tint | crimson text + tint | – | (tok) |
| **Sidebar section header** | Work Sans, uppercase 0.15em | muted text | – | – | – |
| **Kanban column bg** | #F9F3E7 | #1c1510 | #1c1a1a | #EDE5E5 | (tok) |
| **Task card padding** | asymmetric, no borders | asymmetric, no borders | asymmetric, no borders | asymmetric, no borders | default |
| **Task card hover** | #F9F3E7 + shadow | #251b12 + shadow | #201f1f | #EDE5E5 | – |
| **Task card borders** | left-only (priority) | left-only (priority) | left-only (priority) | left-only (priority) | default |
| **Task title font** | Newsreader 14px/600 | Newsreader 14px/600 | Newsreader 14px/600 | Newsreader 14px/600 | – |
| **#project-name** | Newsreader italic 20px | Newsreader italic 20px | Newsreader italic 20px | Newsreader italic 20px | – |
| **View tabs container** | pill group | pill group | pill group | pill group | – |
| **View tab active** | brass fill, pill | brass fill, pill | crimson rgba tint, pink text | accent-light fill | – |
| **View tab font** | font-label + tracking | font-label + tracking | font-label | font-label | – |
| **Status badges** | – | – | pill-shaped, status-bg/text | – | – |
| **Button (primary)** | brass accent + inner glow | brass accent + inner glow | gradient silk blood-silk | crimson + shadow | – |
| **Button (secondary)** | ghost, 15% border | ghost, 15% border | ghost, very low border | – | – |
| **Button font** | font-label + tracking | font-label + tracking | font-label | font-label | – |
| **Search input** | ledger (bottom border only) | ledger (bottom border only) | – | – | – |
| **Form group inputs** | transparent, bottom-border-only | transparent, bottom-border-only | semi-trans dark bg, crimson border, gold focus | rose-tint bg, crimson border | (tok) |
| **Form group labels** | font-label + tracking | font-label + tracking | font-label | font-label | – |
| **Modal bg** | warm shadow, warm border | warm shadow | glass (rgba + blur), crimson border+glow | crimson shadow+border | – |
| **Modal header** | gradient bg, no border-bottom | – | thin dim separator | – | – |
| **Modal h2** | Newsreader italic, accent | Newsreader italic, accent | Newsreader italic, #ffb3b5 | Newsreader italic, #af2b3e | – |
| **Login screen bg** | parchment radial gradient | inkwell radial gradient | near-black solid | rose-dust flat | – |
| **Login card bg** | --modal-surface, warm border | – | dark glass | white, crimson border | – |
| **Login title** | Newsreader italic, accent | Newsreader italic, accent | – | Newsreader italic, accent | – |
| **Login logo** | brass gradient | – | – | crimson gradient | – |
| **Login inputs** | transparent, bottom-border | transparent, bottom-border | – | rose-tint bg, crimson border | – |
| **Sidebar user name** | text-primary | text-primary | text-primary | text-primary | – |
| **.font-label class** | Work Sans | Work Sans | Manrope | Manrope | Work Sans |

---

## 3. Elements with NO theme-specific override (fall through to defaults)

These elements use only the CSS custom property tokens — any theme change must go through
token overrides, not selector overrides.

- `.loading-spinner` / `.spinner`
- `.toast` (success/error)
- `.archive-toggle`
- `.activity-panel` and children
- `.dashboard` / `.dashboard-card` / `.dashboard-task-*`
- `.bulk-bar` / `.bulk-status-menu`
- `.comments-section` and children
- `.cookie-banner`
- `.app-footer`
- `.content-viewer-body` (markdown prose)
- `.template-banner` (uses accent directly)
- `.slug-preview`

---

## 4. Open gaps (items without per-variant tokens)

| Gap | Notes |
|-----|-------|
| `scholarly` light has no explicit status tokens | Falls through; `--status-*` not set in light block. Could cause contrast issues on light bg. |
| `relic-light` has no explicit status tokens | Same gap — `--status-*` not set. |
| `scholarly` light `--priority-*` not set | Other palettes set these; scholarly only sets them in dark variant. |
| `relic-light` `--priority-*` not set | Same gap. |
| Template banner uses hard-coded gradient | `linear-gradient(135deg, var(--accent) 0%, #7c3aed 100%)` — second stop is not a token. |
| `.btn-warning` uses hard-coded `#f59e0b` | Not token-driven. |
| `.content-viewer-body code` uses `rgba(0,0,0,.06)` | Not token-driven — will look odd on very dark themes. |

---

## 5. Spec-readiness checklist (for when spec arrives)

When the UI spec is received:
- [ ] Parse spec element-by-element against the mapping table above
- [ ] Identify which component rows need new overrides vs. token adjustments
- [ ] Check if spec introduces new component classes not in the table
- [ ] Confirm: all Scholarly/Relic additions follow tonal-shift rule (see DESIGN.md)
- [ ] Implement changes under existing `data-palette` selector blocks
- [ ] Screenshot-diff all 12 themes before/after (Playwright)
- [ ] Verify no regressions in non-named palettes
