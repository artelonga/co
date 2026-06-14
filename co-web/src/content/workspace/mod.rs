//! Workspace — "Sala" state persistence (CO-352) + template registry (CO-355)
//! + realtime lobby/WebSocket layer (CO-353).

pub mod lobby;
pub mod routes;
pub mod template_routes;
pub mod ws;

#[cfg(test)]
mod ws_tests;
