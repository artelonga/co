//! CO-387: time-rendering primitive.
//!
//! Decouples canonical time storage from rendering:
//! - Canonical storage: Unix ms columns on `entries` (universe-pool v19) for
//!   human-scale lenses; lens-specific frontmatter fields (`cosmic_year_bp`,
//!   `shandara_year`, …) for scales that overflow i64 ms.
//! - Per-universe `_calendar.yaml` declares the available calendar lenses
//!   ([`calendar_loader`]).
//! - Pure per-lens conversion math lives in [`conversion`] and is mirrored in
//!   `static/shared/lib/co-time.js` for the `<co-time-grid>` SPA component.

pub mod calendar_loader;
pub mod conversion;
pub mod routes;
