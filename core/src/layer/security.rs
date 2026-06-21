//! Security layers — integrity + provenance of `content`.
//!
//! Per the composition rules, security sits *above* privacy: on write we sign
//! the ciphertext (the bytes produced by the privacy layer below), so a verifier
//! can detect tampering without seeing plaintext.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

const SIG_LEN: usize = 64;

/// No-op security layer: for local writes where signing is unnecessary.
pub struct UnverifiedTrusted;

impl Layer for UnverifiedTrusted {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        Ok(input)
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        Ok(input)
    }
}

/// Ed25519 detached-signature layer.
///
/// `write` computes a signature over `content` and frames the output as
/// `signature(64) || content`; `read` strips the signature, verifies it against
/// the carried `content`, and fails loudly if verification fails.
///
/// Construct with [`Ed25519Signed::new`] (a 32-byte secret seed) for a layer
/// that can both sign and verify, or [`Ed25519Signed::verifier`] (a public key)
/// for a read-only verifier that refuses to sign.
pub struct Ed25519Signed {
    signing: Option<SigningKey>,
    verifying: VerifyingKey,
}

impl Ed25519Signed {
    /// Build from a 32-byte secret seed; can sign and verify.
    pub fn new(secret_seed: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&secret_seed);
        let verifying = signing.verifying_key();
        Self {
            signing: Some(signing),
            verifying,
        }
    }

    /// Build a verify-only layer from a public key. `write` will error.
    pub fn verifier(public_key: [u8; 32]) -> Result<Self, LayerError> {
        let verifying = VerifyingKey::from_bytes(&public_key)
            .map_err(|e| LayerError::Crypto(format!("bad verifying key: {e}")))?;
        Ok(Self {
            signing: None,
            verifying,
        })
    }

    /// The public key bytes, for distributing to verifiers.
    pub fn public_key(&self) -> [u8; 32] {
        self.verifying.to_bytes()
    }
}

impl Layer for Ed25519Signed {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        if input.content.len() < SIG_LEN {
            return Err(LayerError::Crypto("signed payload too short".into()));
        }
        let (sig_bytes, payload) = input.content.split_at(SIG_LEN);
        let sig = Signature::from_bytes(
            sig_bytes
                .try_into()
                .map_err(|_| LayerError::Crypto("bad signature length".into()))?,
        );
        self.verifying
            .verify(payload, &sig)
            .map_err(|_| LayerError::Crypto("signature verification failed".into()))?;
        input.content = payload.to_vec();
        Ok(input)
    }

    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        let signing = self
            .signing
            .as_ref()
            .ok_or_else(|| LayerError::Crypto("verify-only layer cannot sign".into()))?;
        let sig: Signature = signing.sign(&input.content);
        let mut framed = Vec::with_capacity(SIG_LEN + input.content.len());
        framed.extend_from_slice(&sig.to_bytes());
        framed.extend_from_slice(&input.content);
        input.content = framed;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofile(body: &[u8]) -> CoFile {
        CoFile {
            path: "doc.md".into(),
            content: body.to_vec(),
            sha256: "s".into(),
            mime_type: "text/markdown".into(),
            modified_ns: 9,
        }
    }

    #[test]
    fn unverified_is_identity() {
        let u = UnverifiedTrusted;
        let original = cofile(b"x");
        assert_eq!(
            u.read(u.write(original.clone()).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn ed25519_sign_then_verify_round_trips() {
        let s = Ed25519Signed::new([11u8; 32]);
        let original = cofile(b"authentic content");
        let signed = s.write(original.clone()).unwrap();
        assert_eq!(signed.content.len(), SIG_LEN + original.content.len());
        let back = s.read(signed).unwrap();
        assert_eq!(back.content, original.content);
    }

    #[test]
    fn verifier_only_layer_verifies_but_cannot_sign() {
        let signer = Ed25519Signed::new([22u8; 32]);
        let signed = signer.write(cofile(b"payload")).unwrap();
        let verifier = Ed25519Signed::verifier(signer.public_key()).unwrap();
        assert_eq!(verifier.read(signed).unwrap().content, b"payload");
        assert!(verifier.write(cofile(b"nope")).is_err());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let s = Ed25519Signed::new([33u8; 32]);
        let mut signed = s.write(cofile(b"original")).unwrap();
        *signed.content.last_mut().unwrap() ^= 0xff; // flip a byte
        assert!(s.read(signed).is_err());
    }

    #[test]
    fn wrong_key_fails_verification() {
        let signed = Ed25519Signed::new([1u8; 32])
            .write(cofile(b"data"))
            .unwrap();
        assert!(Ed25519Signed::new([2u8; 32]).read(signed).is_err());
    }
}
