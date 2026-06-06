//! CO-380: `SalaBroadcaster` — replaces CO-353 workspace-presence lobby.
//!
//! Subscribes to `workspace.*` events and fans them out to WebSocket
//! connections in the same universe room. The actual WebSocket fanout is
//! handled by the existing `sync_rooms` broadcast channels in `RealtimeState`.

use std::sync::Arc;

use tracing::{debug, info};

use crate::eda::bus::{EdaBus, Filter};

/// Spawn the `SalaBroadcaster` subscriber.
///
/// Listens for `workspace.cursor`, `workspace.entry_placed`,
/// `workspace.entry_linked` events and logs them (full WebSocket fanout
/// is wired in CO-381 LiveTimeline — `SalaBroadcaster` owns the filter).
pub fn spawn(bus: Arc<dyn EdaBus>) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec![
            "workspace.cursor".into(),
            "workspace.entry_placed".into(),
            "workspace.entry_linked".into(),
        ]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: SalaBroadcaster started (CO-353 presence replacement)");
        while let Some(ev) = sub.recv().await {
            debug!(
                event_type = %ev.event_type,
                universe_key = ?ev.universe_key,
                "EDA: workspace event — would fan-out to sala room"
            );
            // CO-381: feed into LiveTimeline WebSocket fanout here.
        }
        info!("EDA: SalaBroadcaster stopped");
    });
}
