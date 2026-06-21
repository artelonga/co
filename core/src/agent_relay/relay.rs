//! CO-110 relay routing decision — the *pure* policy the Co server applies to a
//! frame before forwarding it. This is the security boundary on the server side:
//! everything the relay is allowed to consider is an argument here, and the
//! frame body is **not** one of them.
//!
//! The relay never decrypts. It decides, from header metadata + pairing state +
//! caller identity + page capability + peer liveness, whether to:
//!
//! * [`RelayDecision::Forward`] the opaque bytes to the peer socket,
//! * [`RelayDecision::Queue`] them briefly until the peer connects, or
//! * [`RelayDecision::Reject`] with a precise [`RejectReason`].

use chrono::{DateTime, Utc};

use super::frame::ServerView;
use super::pairing::{Pairing, PairingStatus};

/// The relay's verdict for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayDecision {
    /// Forward the opaque ciphertext to the peer socket verbatim.
    Forward,
    /// Peer not connected yet; hold the frame (bounded queue, ~60s) and forward
    /// on connect.
    Queue,
    /// Drop the frame; do not forward.
    Reject(RejectReason),
}

/// Why the relay refused to forward a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RejectReason {
    /// No pairing with this id exists.
    #[error("unknown pairing")]
    UnknownPairing,
    /// The authenticated caller does not own this pairing.
    #[error("pairing does not belong to caller")]
    NotOwner,
    /// Pairing is still pending agent confirmation.
    #[error("pairing pending confirmation")]
    Pending,
    /// Pairing has expired.
    #[error("pairing expired")]
    Expired,
    /// Pairing was revoked.
    #[error("pairing revoked")]
    Revoked,
    /// The connecting page does not declare `mount: agent` capability.
    #[error("page lacks mount:agent capability")]
    PageNotCapable,
    /// The frame's pairing id did not match the connection's pairing id
    /// (a misrouted or spoofed frame).
    #[error("frame pairing id mismatch")]
    PairingMismatch,
}

/// Inputs the relay knows about a connection, gathered from the authenticated
/// WebSocket upgrade — never from the frame body.
#[derive(Debug, Clone, Copy)]
pub struct RelayContext<'a> {
    /// The authenticated caller's user id (from JWT / API token).
    pub caller_user_id: &'a str,
    /// The pairing id this socket connected as (from the query string).
    pub connection_pairing_id: &'a str,
    /// Whether the connecting page's manifest declares `mount: agent`.
    pub page_has_mount: bool,
    /// Whether the peer endpoint (the other half of the pairing) is connected.
    pub peer_connected: bool,
}

/// Decide what to do with a frame.
///
/// `pairing` is the looked-up record (or `None` if unknown). The decision is a
/// total function of metadata only — note the absence of any ciphertext
/// argument, which is the structural reason the operator cannot read content.
pub fn decide(
    view: &ServerView<'_>,
    pairing: Option<&Pairing>,
    ctx: &RelayContext<'_>,
    now: DateTime<Utc>,
) -> RelayDecision {
    // A frame must address the same pairing the socket connected as; otherwise
    // a caller could try to relay onto someone else's channel.
    if view.pairing_id != ctx.connection_pairing_id {
        return RelayDecision::Reject(RejectReason::PairingMismatch);
    }

    let Some(p) = pairing else {
        return RelayDecision::Reject(RejectReason::UnknownPairing);
    };

    if p.user_id != ctx.caller_user_id {
        return RelayDecision::Reject(RejectReason::NotOwner);
    }

    if !ctx.page_has_mount {
        return RelayDecision::Reject(RejectReason::PageNotCapable);
    }

    match p.status(now) {
        PairingStatus::Pending => return RelayDecision::Reject(RejectReason::Pending),
        PairingStatus::Expired => return RelayDecision::Reject(RejectReason::Expired),
        PairingStatus::Revoked => return RelayDecision::Reject(RejectReason::Revoked),
        PairingStatus::Active => {}
    }

    if ctx.peer_connected {
        RelayDecision::Forward
    } else {
        RelayDecision::Queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_relay::frame::{Direction, RelayFrame};
    use crate::agent_relay::pairing::Scope;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn active_pairing() -> Pairing {
        Pairing {
            id: "pair_1".into(),
            user_id: "u1".into(),
            page_url: "co.artelonga.com.br/co/template/agent-demo".into(),
            agent_pubkey: Some("x25519:agent".into()),
            browser_pubkey: "x25519:browser".into(),
            scope: Scope::ReadWrite,
            created_at: ts(1_000),
            expires_at: ts(10_000),
            revoked_at: None,
        }
    }

    fn frame_view(pairing_id: &str) -> (Vec<u8>, ()) {
        (
            RelayFrame::new(Direction::BrowserToAgent, pairing_id, 1, vec![1, 2, 3]).encode(),
            (),
        )
    }

    fn ctx<'a>(pairing_id: &'a str, peer: bool) -> RelayContext<'a> {
        RelayContext {
            caller_user_id: "u1",
            connection_pairing_id: pairing_id,
            page_has_mount: true,
            peer_connected: peer,
        }
    }

    #[test]
    fn forwards_when_active_and_peer_connected() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        let p = active_pairing();
        assert_eq!(
            decide(&view, Some(&p), &ctx("pair_1", true), ts(2_000)),
            RelayDecision::Forward
        );
    }

    #[test]
    fn queues_when_peer_absent() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        let p = active_pairing();
        assert_eq!(
            decide(&view, Some(&p), &ctx("pair_1", false), ts(2_000)),
            RelayDecision::Queue
        );
    }

    #[test]
    fn rejects_unknown_pairing() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        assert_eq!(
            decide(&view, None, &ctx("pair_1", true), ts(2_000)),
            RelayDecision::Reject(RejectReason::UnknownPairing)
        );
    }

    #[test]
    fn rejects_when_caller_not_owner() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        let p = active_pairing();
        let mut c = ctx("pair_1", true);
        c.caller_user_id = "intruder";
        assert_eq!(
            decide(&view, Some(&p), &c, ts(2_000)),
            RelayDecision::Reject(RejectReason::NotOwner)
        );
    }

    #[test]
    fn rejects_page_without_mount() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        let p = active_pairing();
        let mut c = ctx("pair_1", true);
        c.page_has_mount = false;
        assert_eq!(
            decide(&view, Some(&p), &c, ts(2_000)),
            RelayDecision::Reject(RejectReason::PageNotCapable)
        );
    }

    #[test]
    fn rejects_frame_addressing_other_pairing() {
        let (bytes, _) = frame_view("pair_OTHER");
        let (view, _) = ServerView::parse(&bytes).unwrap();
        let p = active_pairing();
        // Socket connected as pair_1 but the frame names pair_OTHER.
        assert_eq!(
            decide(&view, Some(&p), &ctx("pair_1", true), ts(2_000)),
            RelayDecision::Reject(RejectReason::PairingMismatch)
        );
    }

    #[test]
    fn rejects_expired_and_revoked_and_pending() {
        let (bytes, _) = frame_view("pair_1");
        let (view, _) = ServerView::parse(&bytes).unwrap();

        let p = active_pairing();
        assert_eq!(
            decide(&view, Some(&p), &ctx("pair_1", true), ts(50_000)),
            RelayDecision::Reject(RejectReason::Expired)
        );

        let mut pending = active_pairing();
        pending.agent_pubkey = None;
        assert_eq!(
            decide(&view, Some(&pending), &ctx("pair_1", true), ts(2_000)),
            RelayDecision::Reject(RejectReason::Pending)
        );

        let mut revoked = active_pairing();
        revoked.revoked_at = Some(ts(1_500));
        assert_eq!(
            decide(&view, Some(&revoked), &ctx("pair_1", true), ts(2_000)),
            RelayDecision::Reject(RejectReason::Revoked)
        );
    }
}
