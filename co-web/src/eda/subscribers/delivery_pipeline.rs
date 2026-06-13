//! CO-398: `DeliveryPipelinePersistor` — reacts to `task.status_changed` events.
//!
//! Responsibilities:
//!  1. Persist each status transition to `task_status_log` for lead-time queries.
//!  2. When `to = "done"`, publish `deploy.triggered` so CO-395 `construir` and
//!     the UAT→prod flow can react.

use async_trait::async_trait;
use tracing::warn;

use crate::eda::bus::Filter;
use crate::eda::event::{Event, Visibility, new_ulid};
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: persists `task.status_changed` transitions to `task_status_log` and
/// re-publishes `deploy.triggered` when a task reaches `done`.
pub struct DeliveryPipelinePersistor;

#[async_trait]
impl EdaSubscriber for DeliveryPipelinePersistor {
    fn name(&self) -> &'static str {
        "DeliveryPipelinePersistor"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["task.status_changed".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
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
            let storage = ctx.storage.lock();
            if let Err(e) = storage.insert_task_status_log(
                &log_id,
                &universe_key,
                &path,
                status_from.as_deref(),
                &status_to,
                &trigger,
                &triggered_at,
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
            ctx.bus.publish(deploy_ev);
        }
    }
}
