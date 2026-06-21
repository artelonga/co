//! Privacy layers — confidentiality of `content` at rest and in transit.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

const NONCE_LEN: usize = 12;

/// No-op privacy layer: preserves the `CoFile` as-is. Swapping
/// [`ChaCha20Encrypted`] for this requires zero changes elsewhere.
pub struct PassthroughPlain;

impl Layer for PassthroughPlain {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        Ok(input)
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        Ok(input)
    }
}

/// ChaCha20-Poly1305 authenticated encryption of `content`.
///
/// `write` frames the output as `nonce(12) || ciphertext+tag`; `read` splits the
/// nonce back off and decrypts. A random nonce is drawn per write, so the same
/// plaintext encrypts differently each time — yet always round-trips, since the
/// nonce travels with the ciphertext.
///
/// v1 uses a single symmetric key. Multi-recipient public-key wrapping
/// (`RecipientSet`) is a follow-up.
pub struct ChaCha20Encrypted {
    cipher: ChaCha20Poly1305,
}

impl ChaCha20Encrypted {
    pub fn new(key: [u8; 32]) -> Self {
        let key = Key::from_slice(&key);
        Self {
            cipher: ChaCha20Poly1305::new(key),
        }
    }
}

impl Layer for ChaCha20Encrypted {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        if input.content.len() < NONCE_LEN {
            return Err(LayerError::Crypto("ciphertext too short".into()));
        }
        let (nonce_bytes, ct) = input.content.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plain = self
            .cipher
            .decrypt(nonce, ct)
            .map_err(|_| LayerError::Crypto("decrypt failed (bad key or tampered)".into()))?;
        input.content = plain;
        Ok(input)
    }

    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher
            .encrypt(nonce, input.content.as_slice())
            .map_err(|_| LayerError::Crypto("encrypt failed".into()))?;
        let mut framed = Vec::with_capacity(NONCE_LEN + ct.len());
        framed.extend_from_slice(&nonce_bytes);
        framed.extend_from_slice(&ct);
        input.content = framed;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofile(body: &[u8]) -> CoFile {
        CoFile {
            path: "secret.md".into(),
            content: body.to_vec(),
            sha256: "s".into(),
            mime_type: "text/markdown".into(),
            modified_ns: 7,
        }
    }

    #[test]
    fn passthrough_is_identity() {
        let p = PassthroughPlain;
        let original = cofile(b"hi");
        assert_eq!(
            p.read(p.write(original.clone()).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn chacha_round_trips() {
        let c = ChaCha20Encrypted::new([3u8; 32]);
        let original = cofile(b"top secret diary entry");
        let enc = c.write(original.clone()).unwrap();
        assert_ne!(enc.content, original.content);
        let dec = c.read(enc).unwrap();
        assert_eq!(dec.content, original.content);
        assert_eq!(dec.path, "secret.md");
    }

    #[test]
    fn chacha_wrong_key_fails_loudly() {
        let enc = ChaCha20Encrypted::new([1u8; 32])
            .write(cofile(b"data"))
            .unwrap();
        let err = ChaCha20Encrypted::new([2u8; 32]).read(enc).unwrap_err();
        assert!(matches!(err, LayerError::Crypto(_)));
    }

    #[test]
    fn chacha_nonce_is_random_but_round_trips() {
        let c = ChaCha20Encrypted::new([5u8; 32]);
        let a = c.write(cofile(b"same")).unwrap();
        let b = c.write(cofile(b"same")).unwrap();
        assert_ne!(a.content, b.content, "nonce should differ per write");
        assert_eq!(c.read(a).unwrap().content, b"same");
        assert_eq!(c.read(b).unwrap().content, b"same");
    }
}
