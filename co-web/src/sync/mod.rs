//! CO-385: Cross-device sync conflict resolution.
//!
//! Provides:
//! - `conflict_detector`: pure `detect_conflicts()` function + `Conflict`, `ConflictKind`, `EntryRevision` types
//! - `conflict_resolver`: 7 action handlers mapped to CRUD operations
//! - `routes`: REST endpoints (`GET /api/v1/me/sync/conflicts`, `POST /api/v1/sync/conflicts/{id}/resolve`)

pub mod conflict_detector;
pub mod conflict_resolver;
pub mod routes;
pub mod strategy;

pub use conflict_detector::{Conflict, ConflictKind, EntryRevision, detect_conflicts};
pub use conflict_resolver::{Action, ResolutionResult, apply_action};
pub use routes::{persist_conflict, router};
pub use strategy::{ConflictStrategy, resolve as resolve_with_strategy};
