# Time-rendering primitive (CO-387)

CO decouples **canonical time storage** from **rendering**. Entries store
deterministic timestamps; a per-universe `_calendar.yaml` declares *calendar
lenses*; the `<co-time-grid>` component renders any `(entries, lens)` pair.
Switching lens is a pure re-render — same entries, no schema migration, no
API call. This is the IaaS principle applied to time: the bounded service
(lens spec, timestamps) renders the brain's content (entries, custom
calendars, fictional epochs).

## Canonical storage — per-lens canonical units

Unix ms (i64) covers ±292 Myr from epoch — enough for human, historical,
fictional and Pomodoro scales, but the cosmic scale (~13.8 Gyr) overflows it
by ~47×. Each lens therefore declares which canonical field drives its math:

| Lens family | Canonical field | Type | Storage |
|---|---|---|---|
| Gregorian, 4-day-week, Pomodoro, fictional-ms | `event_at_ms` (or `due_at_ms` / `scheduled_at_ms`) | `i64` ms | `entries.event_at_ms` column (universe-pool **v19**) |
| Cosmic | `cosmic_year_bp` | `f64` years-before-present | `entries.frontmatter_json.cosmic_year_bp` |
| Shandara (fictional, custom unit) | `shandara_year` | `i64` units | `entries.frontmatter_json.shandara_year` |

The CO-73 `entry_dates` table (ISO 8601, semantic-keyed) remains the durable
source of truth. Universe-pool migration v19 added the three ms columns and
backfilled them from `entry_dates`; `EntryIndex::upsert_dates` keeps them in
sync on every write. No `BIGINT`/`i128` column is needed: SQLite INTEGER is
already i64, and cosmic math is f64-native, so cosmic timestamps live in
frontmatter (most entries don't need cosmic precision).

## `_calendar.yaml` — per-universe lens config

Placed at the universe content root (same pattern as CO-355's
`_workspace.yaml`). Loader: `co-web/src/time/calendar_loader.rs`. Universes
without the file default to the built-in Gregorian lens.

```yaml
default_lens: gregorian

lenses:
  - id: gregorian
    name: Gregorian (canonical)
    epoch_ms: 0
    scale: linear
    week_length_days: 7
    weekday_names: [Mon, Tue, Wed, Thu, Fri, Sat, Sun]
    month_length: gregorian
    timezone: America/Sao_Paulo

  - id: 4-day-week
    name: 4-day week experiment
    epoch_ms: 1735689600000            # 2025-01-01 (project start)
    scale: linear
    week_length_days: 4
    weekday_names: [Um, Dois, Três, Quatro]
    display_format: "S{week}.D{day}"

  - id: cosmic
    name: Cosmic (Big Bang → now)
    canonical_field: cosmic_year_bp     # f64 years before present
    canonical_type: f64_years
    scale: log
    epoch: present
    display_unit: "billion years"
    label_periods:
      - { name: "Big Bang", at: 13.8e9 }
      - { name: "Sun forms", at: 4.6e9 }
      - { name: "Common Era", at: 2026 }

  - id: shandara
    name: Shandara epoch
    scale: linear
    custom_event_field: "shandara_year" # implies canonical_type: i64_units

  - id: pomodoro
    name: Pomodoro (work cells)
    epoch_ms: 1735689600000
    scale: linear
    cell_duration_ms: 1500000           # 25 min
    break_duration_ms: 300000           # 5 min break
    display_format: "Pom{cell}"
```

Served at `GET /api/v1/universes/{slug}/calendar` (lens metadata only — no
entry data; safe for anonymous read). See `docs/architecture/api-catalog.md`.

## Conversion math

Pure functions in `co-web/src/time/conversion.rs`, mirrored 1:1 in
`co-web/static/shared/lib/co-time.js` (keep both in sync):

- `entry_to_lens_position(raw, lens)` → `LensPosition`:
  - `Linear { week, day_of_week, hour }` — linear + i64 ms (euclidean math,
    so pre-epoch dates work)
  - `Cell { cell, in_break }` — Pomodoro-style lenses (`cell_duration_ms`)
  - `Axis { position }` — raw-unit lenses (`custom_event_field`)
  - `Log { log_position }` — log10 placement for f64 years-bp (cosmic)
  - `None` — type/scale mismatch → entry renders as orphan
- `lens_position_to_canonical(pos, lens)` — inverse, for "click a cell to
  create an event there" (emits the right field type for the active lens).

## `<co-time-grid>` component

`co-web/static/shared/lib/co-time-grid.js` — vanilla custom element:

```html
<co-time-grid universe="co" lens="4-day-week" view-mode="grid"
              range-from="2026-01-01T00:00:00Z" range-to="2026-12-31T23:59:59Z">
</co-time-grid>
```

- View modes: `grid`, `timeline`, `scatter`, `gantt`.
- Lens dropdown switches without reload; choice persists in
  `localStorage.co_calendar_lens_<universe>`.
- `universe` accepts a comma list — entries are color-coded by origin
  universe (the `/timeline` multi-universe view).
- `lens` accepts a comma list — pinned stacked lenses, one section each.
- Live updates: subscribes to the CO-380 event bus
  (`/api/v1/events?scope=<universe>`) and re-renders on `entry.*` events.
- Telemetry (CO-380/CO-46): `time.lens_switched{universe,from_lens,to_lens}`
  and `time.grid_rendered{universe,lens,entry_count}` via `window.coTrack`.
- `el.setData(entries, calendarConfig)` injects data without fetching.

## Lens-frame integration (CO-393 — one-surface rule)

The time grid is an *instance* of the CO-393 lens frame, not a parallel
system: `co-web/static/variants/i/modules/lenses/time-grid.js` registers a
`time-grid` lens via `registerLens()`; `renderContent` dispatches to it
through the registry with zero if/else edits. Named manifest views were
generalized at the registry level (`lens.namedViews` — previously a
gantt-only special case), so a manifest can declare:

```yaml
views:
  - type: time-grid
    name: 4day
    label: "4-day week"
    lens: 4-day-week
    view_mode: grid
```

## `/timeline` multi-lens stacking

`/timeline?lens=cosmic,gregorian,shandara&u=tempo,universo,humanity` pins
stacked lens sections over the same universes (cosmic log-scale renders Big
Bang at the left edge, present at the right). Without `?lens=`, the existing
SVG timeline is untouched.

## Privacy

CO-378 path redaction applies: the entries API already filters private
universes for anonymous viewers, so the grid renders only what the caller
can see. Lens metadata is universe-scoped and contains no entry data.
