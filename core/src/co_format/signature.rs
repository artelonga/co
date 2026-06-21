//! CO-86 — Ed25519 detached signatures for the `.co` envelope. Signing is a
//! *layer*: the signature covers the cleartext `content_hash` (SHA-256 of the
//! plaintext body), so it composes with both plaintext and encrypted bodies.
//!
//! Verification is two-step: (1) when the plaintext body is available,
//! re-derive its hash and confirm it matches `content_hash` (catches a mutated
//! body); (2) verify the Ed25519 signature over `content_hash` (catches a
//! forged or mutated signature). Mutating *any* covered byte fails one of them.

use ed25519_dalek::{Signature as DalekSig, Signer, SigningKey, Verifier, VerifyingKey};

use super::{CoError, CoFile, Signature, canonical_body, content_hash};

/// Signature algorithm label recorded in [`Signature::algo`].
pub const ALGO: &str = "ed25519";

/// Generate a fresh Ed25519 keypair.
pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let signing = SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying = signing.verifying_key();
    (signing, verifying)
}

/// Hex-encode a verifying key — the stable `signer_pubkey_id`.
pub fn pubkey_id(vk: &VerifyingKey) -> String {
    let mut s = String::with_capacity(64);
    for b in vk.to_bytes() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Sign the envelope in place. Refreshes `content_hash` from the plaintext body
/// when available, then attaches an Ed25519 signature over that hash.
pub fn sign(co: &mut CoFile, signing_key: &SigningKey) -> Result<(), CoError> {
    if let Ok(body) = canonical_body(co) {
        co.content_hash = content_hash(&body);
    }
    if co.content_hash.is_empty() {
        return Err(CoError::Invalid("cannot sign: empty content_hash".into()));
    }
    let sig = signing_key.sign(co.content_hash.as_bytes());
    co.signature = Some(Signature {
        algo: ALGO.to_string(),
        signature: sig.to_bytes().to_vec(),
        signer_pubkey_id: pubkey_id(&signing_key.verifying_key()),
    });
    Ok(())
}

/// Verify the envelope's signature under `verifying_key`. Returns `Ok(false)`
/// when the body no longer hashes to `content_hash`, when there is no
/// signature, or when the Ed25519 check fails.
pub fn verify(co: &CoFile, verifying_key: &VerifyingKey) -> Result<bool, CoError> {
    // (1) Body integrity, when the plaintext is reachable.
    if let Ok(body) = canonical_body(co)
        && content_hash(&body) != co.content_hash
    {
        return Ok(false);
    }
    // (2) Signature over content_hash.
    let Some(sig) = co.signature.as_ref() else {
        return Ok(false);
    };
    let bytes: [u8; 64] = match sig.signature.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return Ok(false),
    };
    let dalek_sig = DalekSig::from_bytes(&bytes);
    Ok(verifying_key
        .verify(co.content_hash.as_bytes(), &dalek_sig)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co_format::{co_file, from_markdown};

    #[test]
    fn sign_and_verify() {
        let mut co = from_markdown("---\ntitle: T\n---\nsigned body\n");
        let (sk, vk) = generate_keypair();
        sign(&mut co, &sk).unwrap();
        assert!(verify(&co, &vk).unwrap());
    }

    #[test]
    fn wrong_pubkey_fails() {
        let mut co = from_markdown("body\n");
        let (sk, _) = generate_keypair();
        let (_, other_vk) = generate_keypair();
        sign(&mut co, &sk).unwrap();
        assert!(!verify(&co, &other_vk).unwrap());
    }

    #[test]
    fn mutated_body_fails() {
        let mut co = from_markdown("original body\n");
        let (sk, vk) = generate_keypair();
        sign(&mut co, &sk).unwrap();
        // Mutate the body without re-signing.
        co.body = Some(co_file::Body::Markdown(b"tampered body\n".to_vec()));
        assert!(!verify(&co, &vk).unwrap());
    }

    #[test]
    fn mutated_signature_fails() {
        let mut co = from_markdown("body\n");
        let (sk, vk) = generate_keypair();
        sign(&mut co, &sk).unwrap();
        if let Some(sig) = co.signature.as_mut() {
            sig.signature[0] ^= 0xff;
        }
        assert!(!verify(&co, &vk).unwrap());
    }
}
