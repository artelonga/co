//! CO-380: `AtividadesPersistor` — appends EDA events to `event_log` table.
//!
//! Subscribes to all events and writes each one to `event_log` for replay.
//! The `atividades` audit table continues to be written directly by
//! `log_atividade()` (backward-compat shim).

use std::sync::Arc;

use parking_lot::Mutex;
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::storage::Storage;

/// Spawn the `AtividadesPersistor` subscriber as a background Tokio task.
///
/// Drains the EDA bus and writes each event into `event_log`.
/// When the bus is dropped the task exits cleanly.
pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let mut sub = bus.subscribe(Filter::default());

    tokio::spawn(async move {
        info!("EDA: AtividadesPersistor started");
        while let Some(ev) = sub.recv().await {
            let storage = storage.lock();
            let payload_str = ev.payload.to_string();
            let vis = format!("{:?}", ev.visibility);
            let created_at = ev.created_at.to_rfc3339();
            if let Err(e) = storage.conn().execute(
                "INSERT OR IGNORE INTO event_log \
                 (id, event_type, universe_key, user_id, payload_json, visibility, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    ev.id,
                    ev.event_type,
                    ev.universe_key,
                    ev.user_id,
                    payload_str,
                    vis,
                    created_at,
                ],
            ) {
                warn!("EDA: AtividadesPersistor INSERT failed: {e}");
            }
        }
        info!("EDA: AtividadesPersistor stopped (bus closed)");
    });
}
