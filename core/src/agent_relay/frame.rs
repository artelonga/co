//! CO-110 relay frame — the versioned wire unit the Co server multiplexes
//! between a paired (browser, agent) pair.
//!
//! ## Wire layout (all integers big-endian)
//!
//! ```text
//! offset  size  field
//! 0       2     magic = b"CR"  (Co Relay)
//! 2       1     version        (PROTOCOL_VERSION; major mismatch → reject)
//! 3       1     direction      (0 = browser→agent, 1 = agent→browser)
//! 4       1     pairing_id_len (N, 1..=255)
//! 5       N     pairing_id     (ASCII; the relay routing key)
//! 5+N     8     seq            (per-direction counter; the AEAD nonce source)
//! 13+N    4     ciphertext_len (M)
//! 17+N    M     ciphertext     (OPAQUE to the server — never interpreted)
//! ```
//!
//! The header (everything before `ciphertext`) is the **only** part the relay
//! reads, via [`ServerView`]. The ciphertext is an authenticated ChaCha20-Poly1305
//! envelope (see [`super::session`]); the server has no key and so cannot read
//! or forge it.

use super::PROTOCOL_VERSION;

const MAGIC: [u8; 2] = *b"CR";
/// Fixed header bytes excluding the variable-length `pairing_id`:
/// magic(2) + version(1) + direction(1) + pairing_id_len(1) + seq(8) + ciphertext_len(4).
const FIXED_HEADER: usize = 2 + 1 + 1 + 1 + 8 + 4;

/// Direction of travel through the relay. The server uses this to pick which of
/// the two paired sockets a frame is forwarded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Frame originated at the browser, destined for the agent.
    BrowserToAgent,
    /// Frame originated at the agent, destined for the browser.
    AgentToBrowser,
}

impl Direction {
    fn as_byte(self) -> u8 {
        match self {
            Direction::BrowserToAgent => 0,
            Direction::AgentToBrowser => 1,
        }
    }

    fn from_byte(b: u8) -> Result<Self, FrameError> {
        match b {
            0 => Ok(Direction::BrowserToAgent),
            1 => Ok(Direction::AgentToBrowser),
            other => Err(FrameError::BadDirection(other)),
        }
    }

    /// The direction a reply travels: the relay forwards a `BrowserToAgent`
    /// frame to the agent, whose reply comes back as `AgentToBrowser`.
    pub fn peer(self) -> Direction {
        match self {
            Direction::BrowserToAgent => Direction::AgentToBrowser,
            Direction::AgentToBrowser => Direction::BrowserToAgent,
        }
    }
}

/// Errors from encoding/decoding a [`RelayFrame`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    /// Buffer too short to contain the declared structure.
    #[error("frame truncated: need {need} bytes, have {have}")]
    Truncated { need: usize, have: usize },
    /// Leading magic bytes did not match `b"CR"`.
    #[error("bad magic: not a CO-110 relay frame")]
    BadMagic,
    /// Major protocol version differs from [`PROTOCOL_VERSION`].
    #[error("unsupported protocol version {got} (this build speaks {ours})")]
    BadVersion { got: u8, ours: u8 },
    /// Direction byte was neither 0 nor 1.
    #[error("invalid direction byte {0}")]
    BadDirection(u8),
    /// `pairing_id` was empty or not valid ASCII.
    #[error("invalid pairing id")]
    BadPairingId,
}

/// A decoded relay frame. The browser and agent construct these; the server
/// only ever needs [`ServerView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayFrame {
    /// Protocol version this frame was written with.
    pub version: u8,
    /// Direction of travel.
    pub direction: Direction,
    /// Pairing routing key (the relay multiplex key).
    pub pairing_id: String,
    /// Per-direction sequence number; also the AEAD nonce source.
    pub seq: u64,
    /// Opaque authenticated ciphertext.
    pub ciphertext: Vec<u8>,
}

impl RelayFrame {
    /// Build a frame at the current [`PROTOCOL_VERSION`].
    pub fn new(
        direction: Direction,
        pairing_id: impl Into<String>,
        seq: u64,
        ciphertext: Vec<u8>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            direction,
            pairing_id: pairing_id.into(),
            seq,
            ciphertext,
        }
    }

    /// Serialize to the wire layout documented at the module level.
    pub fn encode(&self) -> Vec<u8> {
        let id = self.pairing_id.as_bytes();
        let mut out = Vec::with_capacity(FIXED_HEADER + id.len() + self.ciphertext.len());
        out.extend_from_slice(&MAGIC);
        out.push(self.version);
        out.push(self.direction.as_byte());
        out.push(id.len() as u8);
        out.extend_from_slice(id);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&(self.ciphertext.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Parse a full frame. Rejects bad magic, a differing major version, a bad
    /// direction byte, a non-ASCII / empty pairing id, and any truncation.
    pub fn decode(buf: &[u8]) -> Result<Self, FrameError> {
        let (view, ciphertext) = ServerView::parse(buf)?;
        Ok(Self {
            version: view.version,
            direction: view.direction,
            pairing_id: view.pairing_id.to_string(),
            seq: view.seq,
            ciphertext: ciphertext.to_vec(),
        })
    }
}

/// The relay's view of a frame: header metadata plus an **opaque** borrow of the
/// ciphertext. This is deliberately the *only* parser the server uses — it
/// cannot, by construction, recover plaintext from a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerView<'a> {
    /// Protocol version of the frame.
    pub version: u8,
    /// Routing key.
    pub pairing_id: &'a str,
    /// Direction of travel (selects the peer socket).
    pub direction: Direction,
    /// Sequence number (for ordering / rate accounting only).
    pub seq: u64,
    /// Length of the opaque ciphertext, in bytes.
    pub ciphertext_len: usize,
}

impl<'a> ServerView<'a> {
    /// Parse the header and return the metadata view together with a borrowed
    /// slice of the opaque ciphertext. The ciphertext slice is returned only so
    /// the relay can forward it byte-for-byte — it is never interpreted.
    pub fn parse(buf: &'a [u8]) -> Result<(ServerView<'a>, &'a [u8]), FrameError> {
        if buf.len() < 5 {
            return Err(FrameError::Truncated {
                need: 5,
                have: buf.len(),
            });
        }
        if buf[0..2] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        let version = buf[2];
        if version != PROTOCOL_VERSION {
            return Err(FrameError::BadVersion {
                got: version,
                ours: PROTOCOL_VERSION,
            });
        }
        let direction = Direction::from_byte(buf[3])?;
        let id_len = buf[4] as usize;
        if id_len == 0 {
            return Err(FrameError::BadPairingId);
        }
        let header_len = FIXED_HEADER + id_len;
        if buf.len() < header_len {
            return Err(FrameError::Truncated {
                need: header_len,
                have: buf.len(),
            });
        }
        let id_bytes = &buf[5..5 + id_len];
        let pairing_id = std::str::from_utf8(id_bytes).map_err(|_| FrameError::BadPairingId)?;
        if !pairing_id.is_ascii() {
            return Err(FrameError::BadPairingId);
        }

        let mut off = 5 + id_len;
        let mut seq_bytes = [0u8; 8];
        seq_bytes.copy_from_slice(&buf[off..off + 8]);
        let seq = u64::from_be_bytes(seq_bytes);
        off += 8;

        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&buf[off..off + 4]);
        let ciphertext_len = u32::from_be_bytes(len_bytes) as usize;
        off += 4;

        let end = off + ciphertext_len;
        if buf.len() < end {
            return Err(FrameError::Truncated {
                need: end,
                have: buf.len(),
            });
        }
        let ciphertext = &buf[off..end];

        Ok((
            ServerView {
                version,
                pairing_id,
                direction,
                seq,
                ciphertext_len,
            },
            ciphertext,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RelayFrame {
        RelayFrame::new(
            Direction::BrowserToAgent,
            "pair_abc123",
            42,
            vec![0xDE, 0xAD, 0xBE, 0xEF],
        )
    }

    #[test]
    fn roundtrip_encode_decode() {
        let f = sample();
        let bytes = f.encode();
        let back = RelayFrame::decode(&bytes).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn server_view_extracts_only_metadata() {
        // The plaintext "secret content" is sealed into ciphertext; the relay's
        // ServerView must surface routing metadata but never the plaintext.
        let plaintext = b"secret content the operator must not read";
        // Stand in for an AEAD output: the server sees these opaque bytes.
        let ciphertext = plaintext.to_vec();
        let f = RelayFrame::new(Direction::AgentToBrowser, "pair_xyz", 7, ciphertext.clone());
        let bytes = f.encode();

        let (view, opaque) = ServerView::parse(&bytes).unwrap();
        assert_eq!(view.pairing_id, "pair_xyz");
        assert_eq!(view.direction, Direction::AgentToBrowser);
        assert_eq!(view.seq, 7);
        assert_eq!(view.ciphertext_len, ciphertext.len());
        // The relay forwards the bytes verbatim; it has no key to interpret them.
        assert_eq!(opaque, ciphertext.as_slice());
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = sample().encode();
        bytes[0] = b'X';
        assert_eq!(RelayFrame::decode(&bytes), Err(FrameError::BadMagic));
    }

    #[test]
    fn version_mismatch_rejected() {
        let mut bytes = sample().encode();
        bytes[2] = 99;
        assert_eq!(
            RelayFrame::decode(&bytes),
            Err(FrameError::BadVersion {
                got: 99,
                ours: PROTOCOL_VERSION
            })
        );
    }

    #[test]
    fn bad_direction_rejected() {
        let mut bytes = sample().encode();
        bytes[3] = 5;
        assert_eq!(RelayFrame::decode(&bytes), Err(FrameError::BadDirection(5)));
    }

    #[test]
    fn empty_pairing_id_rejected() {
        // Hand-build a frame with id_len = 0.
        let mut bytes = vec![b'C', b'R', PROTOCOL_VERSION, 0u8, 0u8];
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(RelayFrame::decode(&bytes), Err(FrameError::BadPairingId));
    }

    #[test]
    fn truncated_header_rejected() {
        let bytes = sample().encode();
        let err = RelayFrame::decode(&bytes[..3]).unwrap_err();
        assert!(matches!(err, FrameError::Truncated { .. }));
    }

    #[test]
    fn truncated_ciphertext_rejected() {
        let bytes = sample().encode();
        // Drop the last ciphertext byte.
        let err = RelayFrame::decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(err, FrameError::Truncated { .. }));
    }

    #[test]
    fn direction_peer_flips() {
        assert_eq!(Direction::BrowserToAgent.peer(), Direction::AgentToBrowser);
        assert_eq!(Direction::AgentToBrowser.peer(), Direction::BrowserToAgent);
    }

    #[test]
    fn empty_ciphertext_is_valid() {
        let f = RelayFrame::new(Direction::BrowserToAgent, "p", 0, vec![]);
        let back = RelayFrame::decode(&f.encode()).unwrap();
        assert_eq!(back.ciphertext.len(), 0);
    }
}
