//! CO-148 — Phase 3 of CO-145: per-universe asset encryption envelope.
//!
//! ## Threat model (Tier 1 — what ships)
//!
//! - **Disk-only attacker** (stolen volume, leaked backup, accidental dump)
//!   cannot read blob plaintext. They'd also need the server's
//!   `CO_ASSETS_MASTER_KEY` env (or `JWT_SECRET`, used as a fallback so dev
//!   environments work without extra setup).
//! - **Server-trusted attacker** (root on the running box, or anyone holding
//!   the env) CAN read blobs. Tier 2 (user-password-derived KEK with
//!   session-scoped DEK) closes this gap and is filed as CO-148-followup.
//!
//! ## Key hierarchy (deterministic)
//!
//! ```text
//! master_key (32 bytes from CO_ASSETS_MASTER_KEY or JWT_SECRET)
//!         │ BLAKE3 keyed hash, ctx = "co-asset-dek-v1\0" || universe_key
//!         ▼
//!   per-universe DEK (32 bytes — never stored on disk; derived on demand)
//!         │ ChaCha20-Poly1305 + random 12-byte nonce + AAD = universe_key || sha256
//!         ▼
//!     blob ciphertext (size = plaintext + 16-byte Poly1305 tag)
//! ```
//!
//! Deterministic derivation means we don't store the DEK anywhere; lose the
//! master key and the universe's blobs are unrecoverable, which matches the
//! "encryption-at-rest is real" claim. Rotation: change the master key + run
//! a re-wrap sweep (deferred to CO-148 follow-up).
//!
//! ## On-disk layout
//!
//! Phase 3 keeps the same `data/universes/<aa>/<bb>/<key>/blobs/<aa>/<bb>/<sha>`
//! path. The bytes at that path are now CIPHERTEXT for newly-uploaded assets
//! (`assets.encrypted = 1`). Existing Phase 1 plaintext rows (`encrypted = 0`)
//! continue to read directly without decryption. A future migration sweep will
//! rewrite plaintext rows in place.

use blake3::Hasher;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};

const DEK_CONTEXT: &[u8] = b"co-asset-dek-v1\0";

/// 32-byte master key derived from env. `CO_ASSETS_MASTER_KEY` is the canonical
/// source; falls back to `JWT_SECRET` so dev environments don't need extra
/// setup (the security posture in dev is "no encryption-at-rest claim made").
fn master_key() -> [u8; 32] {
    // CO-434: read through the process-global SecretsProvider seam.
    let secrets = crate::infra::secrets::global();
    let raw = secrets
        .get("CO_ASSETS_MASTER_KEY")
        .or_else(|| secrets.get("JWT_SECRET"))
        .unwrap_or_else(|| "dev-secret-change-me".to_string());
    // Hash whatever we got into a clean 32-byte key — accepts any length input
    // including hex/base64-encoded values without further parsing.
    *blake3::hash(raw.as_bytes()).as_bytes()
}

/// Per-universe DEK derived deterministically from the master key and the
/// universe key. Never stored on disk.
pub fn derive_dek(universe_key: &str) -> [u8; 32] {
    let master = master_key();
    let mut hasher = Hasher::new_keyed(&master);
    hasher.update(DEK_CONTEXT);
    hasher.update(universe_key.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Encrypt a blob. Returns (ciphertext, nonce). The nonce is 12 random bytes;
/// store it alongside the row so we can decrypt later.
///
/// AAD = `universe_key || sha256` — binds the ciphertext to its identity so
/// copying a blob into a different universe (or under a different sha) fails
/// to decrypt.
pub fn encrypt_blob(
    plaintext: &[u8],
    universe_key: &str,
    sha256_hex: &str,
) -> Result<(Vec<u8>, [u8; 12]), String> {
    let dek = derive_dek(universe_key);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&dek));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let aad = aad_for(universe_key, sha256_hex);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|e| format!("encrypt: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());
    Ok((ciphertext, nonce_bytes))
}

/// Decrypt a blob. Returns the plaintext bytes.
pub fn decrypt_blob(
    ciphertext: &[u8],
    nonce: &[u8; 12],
    universe_key: &str,
    sha256_hex: &str,
) -> Result<Vec<u8>, String> {
    let dek = derive_dek(universe_key);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&dek));
    let nonce = Nonce::from_slice(nonce);
    let aad = aad_for(universe_key, sha256_hex);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|e| format!("decrypt: {e}"))
}

fn aad_for(universe_key: &str, sha256_hex: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(universe_key.len() + 1 + sha256_hex.len());
    v.extend_from_slice(universe_key.as_bytes());
    v.push(0);
    v.extend_from_slice(sha256_hex.as_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate_env() {
        // Tests share the process; force a deterministic master key.
        unsafe { std::env::set_var("CO_ASSETS_MASTER_KEY", "test-master-key") };
    }

    #[test]
    fn dek_is_deterministic_per_universe() {
        isolate_env();
        let a = derive_dek("uA");
        let b = derive_dek("uA");
        let c = derive_dek("uB");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn round_trip_encrypts_and_decrypts() {
        isolate_env();
        let plaintext = b"hello world";
        let (ct, nonce) = encrypt_blob(plaintext, "u1", "deadbeef").unwrap();
        assert_ne!(ct, plaintext);
        let pt = decrypt_blob(&ct, &nonce, "u1", "deadbeef").unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn aad_mismatch_fails() {
        isolate_env();
        let (ct, nonce) = encrypt_blob(b"bytes", "uA", "sha-1").unwrap();
        // Wrong universe
        assert!(decrypt_blob(&ct, &nonce, "uB", "sha-1").is_err());
        // Wrong sha
        assert!(decrypt_blob(&ct, &nonce, "uA", "sha-2").is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        isolate_env();
        let (mut ct, nonce) = encrypt_blob(b"bytes", "u", "sha").unwrap();
        ct[0] ^= 0x01;
        assert!(decrypt_blob(&ct, &nonce, "u", "sha").is_err());
    }
}
