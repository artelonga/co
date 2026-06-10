//! CO-388: Security audit pipeline (Project Glasswing-aligned).
//!
//! Integrates as step 11 of CO-382's deterministic CI route.
//! Findings flow through the EDA bus, land as PBIs in the sprint backlog,
//! surface in `/agora` live timeline, and gate the release via DoD verification.
//!
//! ## Backend selection
//!
//! Controlled by `CO_SECURITY_BACKEND`:
//! - `"claude"`   — ClaudeSecurityBackend (full; requires CO_SECURITY_API_KEY)
//! - `"disabled"` — NoOpBackend (dev/test only)
//! - unset        — LocalGrepBackend (default; always available)
//!
//! ## Event flow
//!
//! ```text
//! scan_diff / scan_full
//!   → Finding detected
//!   → publish "security.finding_detected" (Visibility::System)
//!       → FindingsPersistor  → security_findings table
//!       → PBIBacklogger      → work/co/security/SEC-<id>.md (if severity >= Medium)
//!       → ReleaseBlocker     → log error + "security.release_blocked" (if severity >= High)
//!       → AtividadesPersistor → event_log (all events)
//! ```

pub mod audit;
pub mod routes;
pub mod subscribers;

pub use audit::{Finding, Severity};
