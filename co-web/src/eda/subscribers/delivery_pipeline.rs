//! CO-398: `DeliveryPipelinePersistor` — reacts to `task.status_changed` events.
//!
//! Responsibilities:
//!  1. Persist each status transition to `task_status_log` for lead-time queries.
//!  2. When `to = "done"`, publish `deploy.triggered` so CO-395 `construir` and
//!     the UAT→prod flow can react.

use std::sync::Arc;

use parking_lot::Mutex;
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::{Event, Visibility, new_ulid};
use crate::storage::Storage;

/// Spawn the `DeliveryPipelinePersistor` as a background Tokio task.
///
/// Subscribes to `task.status_changed` events, writes each row to
/// `task_status_log`, and re-publishes `deploy.triggered` when a task is done.
pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let bus_clone = Arc::clone(&bus);
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["task.status_changed".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: DeliveryPipelinePersistor started");
        while let Some(ev) = sub.recv().await {
            let universe_key = ev.universe_key.clone().unwrap_or_default();
            let path = ev.payload["path"].as_str().unwrap_or("").to_string();
            let status_from = ev.payload["from"].as_str().map(String::from);
            let status_to = ev.payload["to"].as_str().unwrap_or("").to_string();
            let trigger = ev.payload["trigger"]
                .as_str()
                .unwrap_or("manual")
                .to_string();
            let triggered_at = ev.created_at.to_rfc3339();
            let log_id = new_ulid();

            {
                let storage = storage.lock();
                if let Err(e) = storage.conn().execute(
                    "INSERT OR IGNORE INTO task_status_log \
                     (id, universe_key, entry_path, status_from, status_to, trigger, triggered_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        log_id,
                        universe_key,
                        path,
                        status_from,
                        status_to,
                        trigger,
                        triggered_at,
                    ],
                ) {
                    warn!("EDA: DeliveryPipelinePersistor INSERT failed: {e}");
                }
            }

            // When a task is done, emit deploy.triggered for downstream hooks.
            if status_to == "done" {
                let deploy_ev = Event::new(
                    "deploy.triggered",
                    ev.universe_key.clone(),
                    ev.user_id.clone(),
                    serde_json::json!({
                        "entry_path": path,
                        "universe_key": universe_key,
                        "trigger": trigger,
                    }),
                    Visibility::System,
                );
                bus_clone.publish(deploy_ev);
            }
        }
        info!("EDA: DeliveryPipelinePersistor stopped (bus closed)");
    });
}
