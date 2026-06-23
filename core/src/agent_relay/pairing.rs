//! CO-110 pairing record + state machine.
//!
//! A *pairing* is a `(page, browser, agent)` triple established once via the
//! trust ceremony (see `docs/specs/co-110-pairing-handshake.md`). It carries the
//! two public keys (so the relay can shuttle them during the handshake without
//! ever learning the ECDH shared secret), a scope, and an expiry. The server
//! stores this metadata — never content.
//!
//! The lifecycle is deliberately small and total:
//!
//! ```text
//!            create                 agent confirms
//!   (none) ─────────► Pending ───────────────────► Active
//!                        │                            │
//!                        │ expires_at passed          │ expires_at passed → Expired
//!                        ▼                            │ revoke()           → Revoked
//!                     Expired                         ▼
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a pairing is allowed to do on the agent's filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// List + read only.
    Read,
    /// Write/delete/move only (rare; e.g. a drop box).
    Write,
    /// Full read + write.
    ReadWrite,
}

impl Scope {
    /// Parse the scope string used in the agent config and `mount:` frontmatter.
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "read" => Some(Scope::Read),
            "write" => Some(Scope::Write),
            "read-write" | "read_write" | "readwrite" => Some(Scope::ReadWrite),
            _ => None,
        }
    }

    /// Whether this scope permits the given filesystem operation.
    pub fn allows(self, op: Op) -> bool {
        match self {
            Scope::Read => op.is_read(),
            Scope::Write => op.is_write(),
            Scope::ReadWrite => true,
        }
    }
}

/// A filesystem operation requested over the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `list <path>` — directory listing.
    List,
    /// `read <path>` — file bytes.
    Read,
    /// `watch <path>` — stream of change events (read-shaped).
    Watch,
    /// `write <path> <bytes>`.
    Write,
    /// `delete <path>`.
    Delete,
    /// `move <from> <to>`.
    Move,
}

impl Op {
    /// Read-shaped operations (no mutation of the host filesystem).
    pub fn is_read(self) -> bool {
        matches!(self, Op::List | Op::Read | Op::Watch)
    }

    /// Write-shaped operations (mutate the host filesystem).
    pub fn is_write(self) -> bool {
        matches!(self, Op::Write | Op::Delete | Op::Move)
    }
}

/// The derived status of a pairing at a given instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingStatus {
    /// Created, awaiting the agent's confirmation (no `agent_pubkey` yet).
    Pending,
    /// Confirmed and within its validity window.
    Active,
    /// Past `expires_at`.
    Expired,
    /// Explicitly revoked (by the agent or via the dashboard).
    Revoked,
}

/// Why an operation was denied on an otherwise-known pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthzError {
    /// Pairing not yet confirmed by the agent.
    #[error("pairing is pending agent confirmation")]
    Pending,
    /// Pairing has expired.
    #[error("pairing has expired")]
    Expired,
    /// Pairing was revoked.
    #[error("pairing was revoked")]
    Revoked,
    /// The pairing's scope does not permit this operation.
    #[error("operation not permitted by pairing scope")]
    OutOfScope,
}

/// A pairing record. Mirrors the `agent_pairings` table the relay will persist
/// (Phase 2), but is storage-agnostic here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    /// Stable id (`pair_<nanoid>`), the relay routing key.
    pub id: String,
    /// Owning Co user.
    pub user_id: String,
    /// Page URL this pairing is bound to (must declare `mount: agent`).
    pub page_url: String,
    /// X25519 public key of the agent. `None` until the agent confirms.
    pub agent_pubkey: Option<String>,
    /// X25519 public key of the browser session.
    pub browser_pubkey: String,
    /// Granted scope.
    pub scope: Scope,
    /// When the pairing was created (Pending begins).
    pub created_at: DateTime<Utc>,
    /// When the pairing stops being valid.
    pub expires_at: DateTime<Utc>,
    /// Set when revoked; `None` while live.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Pairing {
    /// Derive the status at `now`. Precedence: revoked → expired → pending →
    /// active. (A revoked-then-also-expired pairing reads as Revoked, which is
    /// the more informative reason.)
    pub fn status(&self, now: DateTime<Utc>) -> PairingStatus {
        if self.revoked_at.is_some() {
            return PairingStatus::Revoked;
        }
        if now >= self.expires_at {
            return PairingStatus::Expired;
        }
        if self.agent_pubkey.is_none() {
            return PairingStatus::Pending;
        }
        PairingStatus::Active
    }

    /// Whether the pairing is live (Active) at `now`.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.status(now) == PairingStatus::Active
    }

    /// Authorize a filesystem operation: the pairing must be Active *and* its
    /// scope must permit the operation. Returns the precise denial reason.
    pub fn authorize(&self, op: Op, now: DateTime<Utc>) -> Result<(), AuthzError> {
        match self.status(now) {
            PairingStatus::Pending => return Err(AuthzError::Pending),
            PairingStatus::Expired => return Err(AuthzError::Expired),
            PairingStatus::Revoked => return Err(AuthzError::Revoked),
            PairingStatus::Active => {}
        }
        if !self.scope.allows(op) {
            return Err(AuthzError::OutOfScope);
        }
        Ok(())
    }

    /// Mark the pairing revoked at `now`. Idempotent: re-revoking keeps the
    /// first timestamp.
    pub fn revoke(&mut self, now: DateTime<Utc>) {
        self.revoked_at.get_or_insert(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn pending() -> Pairing {
        Pairing {
            id: "pair_1".into(),
            user_id: "u1".into(),
            page_url: "co.artelonga.com.br/co/template/agent-demo".into(),
            agent_pubkey: None,
            browser_pubkey: "x25519:browser".into(),
            scope: Scope::ReadWrite,
            created_at: ts(1_000),
            expires_at: ts(10_000),
            revoked_at: None,
        }
    }

    fn active() -> Pairing {
        let mut p = pending();
        p.agent_pubkey = Some("x25519:agent".into());
        p
    }

    #[test]
    fn scope_parse() {
        assert_eq!(Scope::parse("read"), Some(Scope::Read));
        assert_eq!(Scope::parse("write"), Some(Scope::Write));
        assert_eq!(Scope::parse("read-write"), Some(Scope::ReadWrite));
        assert_eq!(Scope::parse("nonsense"), None);
    }

    #[test]
    fn scope_allows_matrix() {
        assert!(Scope::Read.allows(Op::List));
        assert!(Scope::Read.allows(Op::Read));
        assert!(Scope::Read.allows(Op::Watch));
        assert!(!Scope::Read.allows(Op::Write));
        assert!(!Scope::Read.allows(Op::Delete));

        assert!(Scope::Write.allows(Op::Write));
        assert!(Scope::Write.allows(Op::Move));
        assert!(!Scope::Write.allows(Op::Read));

        for op in [
            Op::List,
            Op::Read,
            Op::Watch,
            Op::Write,
            Op::Delete,
            Op::Move,
        ] {
            assert!(Scope::ReadWrite.allows(op));
        }
    }

    #[test]
    fn status_pending_before_confirm() {
        let p = pending();
        assert_eq!(p.status(ts(2_000)), PairingStatus::Pending);
        assert!(!p.is_active(ts(2_000)));
    }

    #[test]
    fn status_active_after_confirm_within_window() {
        let p = active();
        assert_eq!(p.status(ts(2_000)), PairingStatus::Active);
        assert!(p.is_active(ts(2_000)));
    }

    #[test]
    fn status_expired_at_and_after_boundary() {
        let p = active();
        // Boundary is inclusive: at expires_at the pairing is already Expired.
        assert_eq!(p.status(ts(10_000)), PairingStatus::Expired);
        assert_eq!(p.status(ts(99_999)), PairingStatus::Expired);
    }

    #[test]
    fn status_revoked_takes_precedence() {
        let mut p = active();
        p.revoke(ts(3_000));
        assert_eq!(p.status(ts(3_500)), PairingStatus::Revoked);
        // Even past expiry, revoked is reported.
        assert_eq!(p.status(ts(50_000)), PairingStatus::Revoked);
    }

    #[test]
    fn revoke_is_idempotent() {
        let mut p = active();
        p.revoke(ts(3_000));
        p.revoke(ts(4_000));
        assert_eq!(p.revoked_at, Some(ts(3_000)));
    }

    #[test]
    fn authorize_active_in_scope_ok() {
        let p = active();
        assert_eq!(p.authorize(Op::Read, ts(2_000)), Ok(()));
        assert_eq!(p.authorize(Op::Write, ts(2_000)), Ok(()));
    }

    #[test]
    fn authorize_out_of_scope_denied() {
        let mut p = active();
        p.scope = Scope::Read;
        assert_eq!(
            p.authorize(Op::Write, ts(2_000)),
            Err(AuthzError::OutOfScope)
        );
    }

    #[test]
    fn authorize_reports_lifecycle_reasons() {
        assert_eq!(
            pending().authorize(Op::Read, ts(2_000)),
            Err(AuthzError::Pending)
        );

        let p = active();
        assert_eq!(p.authorize(Op::Read, ts(20_000)), Err(AuthzError::Expired));

        let mut r = active();
        r.revoke(ts(2_000));
        assert_eq!(r.authorize(Op::Read, ts(2_500)), Err(AuthzError::Revoked));
    }

    #[test]
    fn pairing_serde_roundtrips() {
        let p = active();
        let json = serde_json::to_string(&p).unwrap();
        let back: Pairing = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
