//! Scrum context.
//!
//! - CO-372: sprint calendar, DoD aggregation, ICS export (`/api/v1/scrum/*`).
//! - CO-368: per-universe Scrum artifacts — `_scrum.yaml` manifest, PBI/Sprint
//!   entries, deterministic current-sprint cadence
//!   (`/api/v1/universes/{key}/scrum/*`).

pub mod calendar;
pub mod current;
pub mod manifest;
pub mod routes;
pub mod validate;

pub use calendar::router;
pub use routes::universe_router;
