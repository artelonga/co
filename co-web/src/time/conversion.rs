//! CO-387 / CO-396: pure per-lens conversion math.
//!
//! The math itself was **hoisted** into the shared layout engine
//! (`game_core::time_layout`) so co-web and yggdrasil-core consume one
//! implementation, not two. This module re-exports it under the historical
//! `crate::time::conversion::*` path. Still mirrored in
//! `static/shared/lib/co-time.js` for the `<co-time-grid>` SPA component —
//! keep that JS mirror in sync.
//!
//! Tests for this math live with the engine, in `game-core`.

pub use game_core::time_layout::{
    CanonicalValue, LensPosition, MS_PER_DAY, MS_PER_HOUR, entry_to_lens_position,
    lens_position_to_canonical, read_canonical_value,
};
