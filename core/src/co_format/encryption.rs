//! CO-86 — body encryption for the `.co` envelope using ChaCha20-Poly1305
//! (AEAD). Encryption is a *layer*: the plaintext body is replaced with
//! `body.encrypted{ algo, ciphertext, nonce }`, while `content_hash` (the
//! SHA-256 of the plaintext) stays in the cleartext header so a recipient can
//! detect tampering after decrypt. The whole body is encrypted or nothing
//! (field-level encryption is out of scope for v1).

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

use super::{CoError, CoFile, Encrypted, canonical_body, co_file, content_hash};

/// AEAD algorithm label recorded in [`Encrypted::algo`].
pub const ALGO: &str = "chacha20-poly1305";

/// Key length (bytes) for ChaCha20-Poly1305.
pub const KEY_LEN: usize = 32;

/// Nonce length (bytes) for ChaCha20-Poly1305.
pub const NONCE_LEN: usize = 12;

/// Generate a fresh random 32-byte symmetric key.
pub fn generate_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Encrypt the plaintext body in place. Requires a plaintext body (markdown or
/// compressed); the result is `body.encrypted{..}` and `content_type` becomes
/// `co/encrypted`. `content_hash` is set to the SHA-256 of the plaintext so a
/// recipient verifies integrity after [`decrypt_body`].
pub fn encrypt_body(co: &mut CoFile, key: &[u8; KEY_LEN]) -> Result<(), CoError> {
    let plaintext = canonical_body(co)?;
    co.content_hash = content_hash(&plaintext);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| CoError::Crypto(format!("encrypt failed: {e}")))?;

    co.body = Some(co_file::Body::Encrypted(Encrypted {
        algo: ALGO.to_string(),
        ciphertext,
        nonce: nonce_bytes.to_vec(),
        encrypted_key: Vec::new(),
        key_recipient: String::new(),
    }));
    co.content_type = "co/encrypted".to_string();
    Ok(())
}

/// Decrypt an encrypted body in place. A wrong key (or any tampered byte)
/// fails AEAD verification; a successful decrypt whose plaintext hash no
/// longer matches `content_hash` raises [`CoError::HashMismatch`]. On success
/// the body becomes plaintext `markdown` and `content_type` is `co/markdown`.
pub fn decrypt_body(co: &mut CoFile, key: &[u8; KEY_LEN]) -> Result<(), CoError> {
    let Some(co_file::Body::Encrypted(enc)) = co.body.as_ref() else {
        return Err(CoError::Invalid("body is not encrypted".into()));
    };
    if enc.nonce.len() != NONCE_LEN {
        return Err(CoError::Crypto("invalid nonce length".into()));
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(&enc.nonce);
    let plaintext = cipher
        .decrypt(nonce, enc.ciphertext.as_ref())
        .map_err(|_| CoError::Crypto("decrypt failed: wrong key or tampered ciphertext".into()))?;

    let actual = content_hash(&plaintext);
    if actual != co.content_hash {
        return Err(CoError::HashMismatch {
            expected: co.content_hash.clone(),
            actual,
        });
    }

    co.body = Some(co_file::Body::Markdown(plaintext));
    co.content_type = "co/markdown".to_string();
    Ok(())
}

/// Returns `true` if the body is an encrypted envelope.
pub fn is_encrypted(co: &CoFile) -> bool {
    matches!(co.body, Some(co_file::Body::Encrypted(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co_format::{canonical_body, from_markdown};

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let md = "---\ntitle: Secret\n---\nclassified body\n";
        let mut co = from_markdown(md);
        let plain = canonical_body(&co).unwrap();
        let key = generate_key();

        encrypt_body(&mut co, &key).unwrap();
        assert!(is_encrypted(&co));
        assert_eq!(co.content_type, "co/encrypted");

        decrypt_body(&mut co, &key).unwrap();
        assert_eq!(canonical_body(&co).unwrap(), plain);
    }

    #[test]
    fn wrong_key_fails() {
        let mut co = from_markdown("body\n");
        let key = generate_key();
        let wrong = generate_key();
        encrypt_body(&mut co, &key).unwrap();
        assert!(matches!(
            decrypt_body(&mut co, &wrong),
            Err(CoError::Crypto(_))
        ));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut co = from_markdown("body\n");
        let key = generate_key();
        encrypt_body(&mut co, &key).unwrap();
        if let Some(co_file::Body::Encrypted(enc)) = co.body.as_mut() {
            enc.ciphertext[0] ^= 0xff; // flip a byte
        }
        assert!(decrypt_body(&mut co, &key).is_err());
    }
}
