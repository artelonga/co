//! CO-380: `AtividadesPersistor` — appends EDA events to `event_log` table.
//!
//! Subscribes to all events and writes each one to `event_log` for replay.
//! The `atividades` audit table continues to be written directly by
//! `log_atividade()` (backward-compat shim).

use async_trait::async_trait;
use tracing::warn;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: appends every EDA event to the `event_log` table for replay.
pub struct AtividadesPersistor;

#[async_trait]
impl EdaSubscriber for AtividadesPersistor {
    fn name(&self) -> &'static str {
        "AtividadesPersistor"
    }

    fn filter(&self) -> Filter {
        Filter::default()
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
        let storage = ctx.storage.lock();
        let payload_str = ev.payload.to_string();
        let vis = format!("{:?}", ev.visibility);
        let created_at = ev.created_at.to_rfc3339();
        if let Err(e) = storage.insert_event_log(
            &ev.id,
            &ev.event_type,
            ev.universe_key.as_deref(),
            ev.user_id.as_deref(),
            &payload_str,
            &vis,
            &created_at,
        ) {
            warn!("EDA: AtividadesPersistor INSERT failed: {e}");
        }
    }
}
