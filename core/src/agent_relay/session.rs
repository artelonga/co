//! CO-110 end-to-end session — the crypto the **endpoints** (browser and agent)
//! run and the **server cannot**.
//!
//! Each side derives the same 32-byte shared secret from an X25519 ECDH between
//! the browser and agent public keys (the server only shuttles the public keys
//! during the handshake; it never sees the secret). From that secret each side
//! builds a [`Session`] and:
//!
//! * [`Session::seal`] — encrypts a plaintext payload into a [`RelayFrame`] with
//!   ChaCha20-Poly1305 (the same AEAD primitive as the CO-86 `.co` envelope),
//!   using a per-direction **counter nonce**.
//! * [`Session::open`] — authenticates + decrypts an incoming frame and enforces
//!   a strictly-increasing sequence number, which gives replay protection: a
//!   frame the server records and re-injects fails the monotonicity check.
//!
//! A wrong key or any tampered byte fails the Poly1305 tag — so the operator can
//! drop or reorder frames (denial of service) but can neither read nor forge
//! content.
//!
//! The [`short_auth_string`] is the social proof of the trust ceremony: both the
//! browser page and the agent terminal derive the same 6-word phrase from the
//! shared secret. If a MITM substituted its own keys, the two phrases differ and
//! the human aborts.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

use super::frame::{Direction, RelayFrame};

/// Shared-secret length (X25519 output / ChaCha20-Poly1305 key length).
pub const SECRET_LEN: usize = 32;

/// Errors from sealing / opening a session frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// AEAD authentication failed: wrong key or tampered ciphertext.
    #[error("decryption failed: wrong key or tampered frame")]
    Decrypt,
    /// The frame's sequence number was not greater than the last accepted one
    /// (a replay or reorder).
    #[error("replayed or out-of-order frame (seq {got} <= last {last})")]
    Replay { got: u64, last: u64 },
    /// The frame travelled in the direction this side also sends — i.e. it is
    /// not addressed to this endpoint.
    #[error("frame direction not addressed to this endpoint")]
    WrongDirection,
}

/// One end of an end-to-end encrypted session. `send_dir` is the direction this
/// endpoint *sends*; it opens frames travelling the opposite way.
pub struct Session {
    cipher: ChaCha20Poly1305,
    send_dir: Direction,
    send_seq: u64,
    recv_high_water: Option<u64>,
}

impl Session {
    /// Build a session from the ECDH shared secret. `send_dir` is the direction
    /// this endpoint emits: a browser uses [`Direction::BrowserToAgent`], an
    /// agent uses [`Direction::AgentToBrowser`].
    pub fn from_shared_secret(secret: &[u8; SECRET_LEN], send_dir: Direction) -> Self {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(secret));
        Session {
            cipher,
            send_dir,
            send_seq: 0,
            recv_high_water: None,
        }
    }

    /// Derive the 12-byte ChaCha20-Poly1305 nonce from the direction + sequence.
    /// Domain-separating by direction means the two endpoints never reuse a
    /// (key, nonce) pair even though both count from zero.
    fn nonce(dir: Direction, seq: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[0] = match dir {
            Direction::BrowserToAgent => 0xB0,
            Direction::AgentToBrowser => 0xA0,
        };
        n[4..12].copy_from_slice(&seq.to_be_bytes());
        n
    }

    /// Encrypt `plaintext` into the next outbound frame for `pairing_id`.
    pub fn seal(&mut self, pairing_id: &str, plaintext: &[u8]) -> RelayFrame {
        let seq = self.send_seq;
        self.send_seq += 1;
        let nonce_bytes = Self::nonce(self.send_dir, seq);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .expect("ChaCha20-Poly1305 encryption is infallible for valid inputs");
        RelayFrame::new(self.send_dir, pairing_id, seq, ciphertext)
    }

    /// Authenticate, decrypt, and replay-check an inbound frame. The frame must
    /// be travelling toward this endpoint (opposite this session's `send_dir`).
    pub fn open(&mut self, frame: &RelayFrame) -> Result<Vec<u8>, SessionError> {
        if frame.direction != self.send_dir.peer() {
            return Err(SessionError::WrongDirection);
        }
        if let Some(last) = self.recv_high_water
            && frame.seq <= last
        {
            return Err(SessionError::Replay {
                got: frame.seq,
                last,
            });
        }
        let nonce_bytes = Self::nonce(frame.direction, frame.seq);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), frame.ciphertext.as_ref())
            .map_err(|_| SessionError::Decrypt)?;
        self.recv_high_water = Some(frame.seq);
        Ok(plaintext)
    }
}

/// A compact, human-pronounceable wordlist for the short authentication string.
/// 64 words → 6 bits each → a 6-word phrase carries ~36 bits. This is a
/// foundation-grade list; production should swap in the 2048-word BIP-39 or the
/// PGP biometric wordlist (see `docs/specs/co-110-crypto-review.md`).
const SAS_WORDS: [&str; 64] = [
    "able", "acid", "aged", "also", "area", "army", "atom", "aunt", "baby", "back", "ball", "band",
    "bank", "barn", "bath", "bean", "bear", "beat", "bell", "belt", "bird", "blue", "boat", "body",
    "bone", "book", "boss", "buzz", "cake", "calm", "card", "care", "cell", "city", "clay", "club",
    "coal", "coat", "code", "coin", "cold", "cool", "copy", "corn", "cost", "crew", "crop", "cube",
    "dark", "dawn", "deal", "deep", "deer", "desk", "dial", "dirt", "dish", "dock", "door", "dove",
    "drum", "dust", "duty", "east",
];

/// Derive the trust-ceremony short authentication string from the ECDH shared
/// secret. Deterministic: both endpoints feed the *same* secret and get the same
/// six words. A MITM with substituted keys yields a different secret → different
/// words → human aborts.
pub fn short_auth_string(secret: &[u8; SECRET_LEN]) -> [&'static str; 6] {
    // Domain-separate the SAS derivation from any other use of the secret.
    let mut hasher = Sha256::new();
    hasher.update(b"co-110/sas/v1");
    hasher.update(secret);
    let digest = hasher.finalize();
    let mut words = ["able"; 6];
    for (i, w) in words.iter_mut().enumerate() {
        *w = SAS_WORDS[(digest[i] & 0x3F) as usize];
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(byte: u8) -> [u8; SECRET_LEN] {
        [byte; SECRET_LEN]
    }

    fn pair() -> (Session, Session) {
        let s = secret(7);
        let browser = Session::from_shared_secret(&s, Direction::BrowserToAgent);
        let agent = Session::from_shared_secret(&s, Direction::AgentToBrowser);
        (browser, agent)
    }

    #[test]
    fn seal_open_roundtrip_browser_to_agent() {
        let (mut browser, mut agent) = pair();
        let frame = browser.seal("pair_1", b"list content/");
        let got = agent.open(&frame).unwrap();
        assert_eq!(got, b"list content/");
    }

    #[test]
    fn seal_open_roundtrip_agent_to_browser() {
        let (mut browser, mut agent) = pair();
        let frame = agent.seal("pair_1", b"{\"tree\":[]}");
        let got = browser.open(&frame).unwrap();
        assert_eq!(got, b"{\"tree\":[]}");
    }

    #[test]
    fn server_with_wrong_key_cannot_decrypt() {
        let (mut browser, _agent) = pair();
        let frame = browser.seal("pair_1", b"secret content");
        // The "operator" holds a different key.
        let mut evil = Session::from_shared_secret(&secret(99), Direction::AgentToBrowser);
        assert_eq!(evil.open(&frame), Err(SessionError::Decrypt));
    }

    #[test]
    fn tampered_ciphertext_fails_auth() {
        let (mut browser, mut agent) = pair();
        let mut frame = browser.seal("pair_1", b"important");
        frame.ciphertext[0] ^= 0xFF;
        assert_eq!(agent.open(&frame), Err(SessionError::Decrypt));
    }

    #[test]
    fn replayed_frame_rejected() {
        let (mut browser, mut agent) = pair();
        let f1 = browser.seal("pair_1", b"one");
        let f2 = browser.seal("pair_1", b"two");
        assert_eq!(agent.open(&f1).unwrap(), b"one");
        assert_eq!(agent.open(&f2).unwrap(), b"two");
        // Operator re-injects the first frame.
        assert_eq!(
            agent.open(&f1),
            Err(SessionError::Replay { got: 0, last: 1 })
        );
    }

    #[test]
    fn wrong_direction_rejected() {
        let (mut browser, _agent) = pair();
        let frame = browser.seal("pair_1", b"hi");
        // Browser tries to open its own outbound frame.
        assert_eq!(browser.open(&frame), Err(SessionError::WrongDirection));
    }

    #[test]
    fn counters_increment_per_seal() {
        let (mut browser, _agent) = pair();
        assert_eq!(browser.seal("p", b"a").seq, 0);
        assert_eq!(browser.seal("p", b"b").seq, 1);
        assert_eq!(browser.seal("p", b"c").seq, 2);
    }

    #[test]
    fn sas_matches_for_same_secret() {
        let s = secret(42);
        assert_eq!(short_auth_string(&s), short_auth_string(&s));
    }

    #[test]
    fn sas_differs_for_mitm_substituted_secret() {
        // Honest endpoints share secret(42); a MITM forces secret(43) on one side.
        assert_ne!(
            short_auth_string(&secret(42)),
            short_auth_string(&secret(43))
        );
    }

    #[test]
    fn sas_words_are_from_the_list() {
        for w in short_auth_string(&secret(1)) {
            assert!(SAS_WORDS.contains(&w));
        }
    }
}
