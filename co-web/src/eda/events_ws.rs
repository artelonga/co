//! CO-380: `/api/v1/events` — real-time EDA event stream over WebSocket.
//!
//! Route: `GET /api/v1/events?scope=<universe_key>`
//!
//! Clients receive a stream of JSON-serialized `Event` objects filtered by:
//!   - `scope` query param (universe_key, or "global" for all)
//!   - Server-side visibility rules (anon → Public only; owner → UniverseOwner+)
//!
//! Federation rules (enforced before each message is sent):
//!   - Anonymous / no token  → `Public` events only
//!   - Universe member       → `Public` + `UniverseMembers` for their universe
//!   - Universe owner        → All of the above + `UniverseOwner`
//!   - System admin          → All including `System`
//!
//! The LiveTimeline broadcast channel feeds this handler; the handler applies
//! per-connection visibility gates before forwarding.
//!
//! ⚠️ **Single-machine fan-out (CO-468).** The LiveTimeline channel is an
//! in-process `tokio::broadcast`. It reaches every client connected *to this
//! machine* and no further. Today `co-artelonga` runs a single Fly machine
//! (`min_machines_running = 1`), so this delivers to everyone. If the app is
//! ever scaled horizontally (≥2 machines), Fly load-balances connections
//! across them and same-universe events will NOT cross machines — clients on
//! machine B won't see events published on machine A. Scaling out therefore
//! requires either sticky routing per universe or a cross-machine pub/sub
//! (Redis/NATS) or self-bridging. The `/api/v1/events/bridge` (CO-384) bridge
//! federates *separate deployments* (co ↔ yggdrasil), NOT machines of one app.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::debug;

use crate::eda::event::{Event, Visibility};
use crate::eda::subscribers::timeline::{TimelineFrame, TimelineSender};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Universe key to scope the stream, or `"global"` for all public events.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Optional JWT token (alternative to session cookie).
    pub token: Option<String>,
}

fn default_scope() -> String {
    "global".into()
}

// ---------------------------------------------------------------------------
// Visibility level for the connected client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ClientLevel {
    Anonymous,
    Member,
    Owner,
    Admin,
}

impl ClientLevel {
    fn can_see(&self, ev: &Event, scope: &str) -> bool {
        // Always hide System-visibility events from non-admin clients.
        match ev.visibility {
            Visibility::System => matches!(self, ClientLevel::Admin),
            Visibility::UserOnly => false, // never pushed to stream
            Visibility::UniverseOwner => {
                matches!(self, ClientLevel::Owner | ClientLevel::Admin)
                    && ev.universe_key.as_deref() == Some(scope)
            }
            Visibility::UniverseMembers => {
                matches!(
                    self,
                    ClientLevel::Member | ClientLevel::Owner | ClientLevel::Admin
                ) && ev.universe_key.as_deref() == Some(scope)
            }
            Visibility::Public => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /api/v1/events — upgrade to WebSocket.
///
/// Auth is accepted via:
///   - `?token=<jwt>` query param
///   - `session` cookie (sent automatically by same-origin browser pages)
///
/// The scope drives both universe filtering and the ownership check for
/// `Owner`-level elevation.
pub async fn events_ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<EventsQuery>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let scope = params.scope.clone();
    let client_level = derive_client_level(&state, &params, &headers);
    let tx: TimelineSender = state.core.timeline_tx.clone();

    ws.on_upgrade(move |socket| handle_socket(socket, tx.subscribe(), scope, client_level))
}

/// Derive the visibility level for this WebSocket connection.
///
/// Priority: `?token=` param → `session` cookie → anonymous.
/// If the JWT is valid and `tier == "admin"` → Admin.
/// If the JWT is valid and the user owns the scoped universe → Owner.
/// If the JWT is valid (any tier) → Member.
fn derive_client_level(state: &AppState, params: &EventsQuery, headers: &HeaderMap) -> ClientLevel {
    let token = params
        .token
        .as_deref()
        .map(|s| s.to_string())
        .or_else(|| crate::auth::extract_session_cookie(headers));

    let Some(token) = token else {
        return ClientLevel::Anonymous;
    };

    let secret = crate::auth::jwt_secret();
    let Ok(claims) = crate::auth::decode_claims(&token, &secret) else {
        return ClientLevel::Anonymous;
    };

    if claims.tier == "admin" {
        return ClientLevel::Admin;
    }

    let scope = &params.scope;
    if scope != "global" {
        let is_owner = state
            .core
            .storage
            .lock()
            .get_universe(scope)
            .map(|u| u.owner_id == claims.sub)
            .unwrap_or(false);
        if is_owner {
            return ClientLevel::Owner;
        }
    }

    ClientLevel::Member
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<Arc<TimelineFrame>>,
    scope: String,
    level: ClientLevel,
) {
    debug!(%scope, ?level, "EDA: WebSocket client connected to /api/v1/events");

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ignore incoming messages
                    Some(Err(_)) => break,
                }
            }
            ev = rx.recv() => {
                match ev {
                    Ok(frame) => {
                        if !level.can_see(&frame.event, &scope) {
                            continue;
                        }
                        // CO-468: reuse the JSON serialized once at the fan-out
                        // point; this is a cheap byte copy, not a re-serialize.
                        if socket
                            .send(Message::Text(frame.json.as_ref().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("EDA: WebSocket client lagged, skipped {n} event(s)");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    debug!("EDA: WebSocket client disconnected from /api/v1/events");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eda::event::Event;
    use chrono::Utc;

    fn ev(vis: Visibility, universe_key: Option<&str>) -> Arc<Event> {
        Arc::new(Event {
            id: "test-id".into(),
            event_type: "entry.created".into(),
            universe_key: universe_key.map(Into::into),
            user_id: None,
            payload: serde_json::json!({}),
            visibility: vis,
            created_at: Utc::now(),
        })
    }

    #[test]
    fn anon_sees_only_public() {
        let anon = ClientLevel::Anonymous;
        assert!(anon.can_see(&ev(Visibility::Public, Some("u1")), "u1"));
        assert!(!anon.can_see(&ev(Visibility::UniverseMembers, Some("u1")), "u1"));
        assert!(!anon.can_see(&ev(Visibility::UniverseOwner, Some("u1")), "u1"));
        assert!(!anon.can_see(&ev(Visibility::System, None), "global"));
    }

    #[test]
    fn member_sees_public_and_universe_members() {
        let member = ClientLevel::Member;
        assert!(member.can_see(&ev(Visibility::Public, None), "global"));
        assert!(member.can_see(&ev(Visibility::UniverseMembers, Some("u1")), "u1"));
        assert!(!member.can_see(&ev(Visibility::UniverseOwner, Some("u1")), "u1"));
        assert!(!member.can_see(&ev(Visibility::System, None), "global"));
    }

    #[test]
    fn member_cannot_see_other_universe_members_events() {
        let member = ClientLevel::Member;
        // scoped to "u1" but event is from "u2"
        assert!(!member.can_see(&ev(Visibility::UniverseMembers, Some("u2")), "u1"));
    }

    #[test]
    fn owner_sees_universe_owner_level() {
        let owner = ClientLevel::Owner;
        assert!(owner.can_see(&ev(Visibility::UniverseOwner, Some("u1")), "u1"));
        assert!(!owner.can_see(&ev(Visibility::System, None), "u1"));
    }

    #[test]
    fn admin_sees_system_events() {
        let admin = ClientLevel::Admin;
        assert!(admin.can_see(&ev(Visibility::System, None), "global"));
    }

    #[test]
    fn user_only_is_never_pushed() {
        for level in [
            ClientLevel::Anonymous,
            ClientLevel::Member,
            ClientLevel::Owner,
            ClientLevel::Admin,
        ] {
            assert!(!level.can_see(&ev(Visibility::UserOnly, None), "global"));
        }
    }
}
