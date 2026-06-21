//! CO-110 — Filesystem-as-Web: protocol core for encrypted remote file editing.
//!
//! This module is the **deterministic, side-effect-free core** of the
//! Filesystem-as-Web architecture (CO-110): a browser renders a live, editable
//! mirror of a remote PC's filesystem, the Co server acts as a *verified relay*
//! only, and the bytes flowing between browser and the user's machine are
//! end-to-end encrypted so the operator cannot read or alter them.
//!
//! ```text
//! Browser  ◄── encrypted WS ──►  Co server (relay)  ◄── encrypted WS ──►  co-agent (user PC)
//!  E2E key                        sees opaque                              E2E key
//!  FS UI                          ciphertext + pairing IDs                 reads/writes allowed paths
//!     └──────────── shared X25519 ECDH key (never on server) ───────────────┘
//! ```
//!
//! ## What lives here (and what does not)
//!
//! The pieces below are the *security-critical, testable* parts that the CO-110
//! spec (`work/co/CO-110.md`, Phase 1) asks to design before any wiring, so the
//! shapes inform CO-86 (`.co` envelope) and CO-87 (layer stack):
//!
//! * [`frame`] — the versioned wire frame the relay multiplexes. The server-side
//!   [`frame::ServerView`] parses **only** routing metadata (pairing id,
//!   direction, sequence) and treats the payload as opaque ciphertext — this is
//!   the structural guarantee that the relay cannot read content.
//! * [`pairing`] — the pairing record + its state machine (Pending → Active →
//!   Expired/Revoked) and per-operation authorization (scope × state × expiry).
//! * [`relay`] — the pure routing *decision* the server is allowed to make
//!   (forward / queue / reject) given only header metadata and pairing state.
//! * [`scope`] — agent-side path scoping: every request is resolved under the
//!   configured roots, and traversal (`../`) / absolute escapes are rejected.
//! * [`session`] — the end-to-end session: ChaCha20-Poly1305 (reusing CO-86's
//!   AEAD primitive) with a counter nonce for replay protection, plus the
//!   short-authentication-string ("6-word phrase") used by the trust ceremony
//!   to detect a MITM.
//!
//! Transport (the axum WebSocket relay endpoint + the `agent_pairings` table +
//! the browser library) and the `co-agent` daemon itself are the *phased*,
//! infrastructure-bearing parts (Phase 2+). They build directly on this core but
//! are intentionally out of this crate so the protocol can be specified, tested,
//! and reviewed in isolation — and so a fast-moving server schema does not gate
//! the protocol design. See the `docs/specs/co-110-*.md` specs and
//! `docs/threat-models/co-110-filesystem-as-web.md`.

pub mod frame;
pub mod pairing;
pub mod relay;
pub mod scope;
pub mod session;

pub use frame::{Direction, FrameError, RelayFrame, ServerView};
pub use pairing::{AuthzError, Op, Pairing, PairingStatus, Scope};
pub use relay::{RejectReason, RelayDecision, decide};
pub use scope::{ScopeError, resolve_within_roots};
pub use session::{SessionError, short_auth_string};

/// Protocol version of the CO-110 relay frame and handshake. Bumped only on a
/// breaking change to the wire format; both peers and the server log (but never
/// reject on) unknown minor differences. A frame whose major version differs is
/// rejected by [`frame::RelayFrame::decode`].
pub const PROTOCOL_VERSION: u8 = 1;
