//! Usage — usage ingestion + launcher registry + fleet queries (CO-426).
//!
//! CO-457: `otlp` adds the OTLP metrics receiver that ingests Claude Code's
//! native `claude_code.*` telemetry into the same `usage_sessions` ledger.

pub mod otlp;
pub mod routes;
