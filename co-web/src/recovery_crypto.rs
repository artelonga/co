//! CO-165 — Encryption for recovery channel values.
//!
//! Follows the same key-derivation pattern as `asset_crypto.rs`:
//!   - Master key from `CO_RECOVERY_KEY` → fallback `JWT_SECRET` → hardcoded dev value.
//!   - Two 32-byte subkeys derived via BLAKE3 keyed hash:
//!       - enc_subkey: ChaCha20-Poly1305 encryption of the channel value.
//!       - lkp_subkey: BLAKE3 keyed hash for the deterministic lookup hash.
//!
//! No AAD is used (unlike asset_crypto) because channel values are small
//! opaque blobs that don't need identity binding.

use blake3::Hasher;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};

const ENC_CONTEXT: &[u8] = b"co-recovery-enc-v1\0";
const LKP_CONTEXT: &[u8] = b"co-recovery-lkp-v1\0";

/// 32-byte master key from env. `CO_RECOVERY_KEY` is canonical; falls back to
/// `JWT_SECRET` so dev environments work without extra setup.
fn master_key() -> [u8; 32] {
    let raw = std::env::var("CO_RECOVERY_KEY")
        .or_else(|_| std::env::var("JWT_SECRET"))
        .unwrap_or_else(|_| "dev-secret-change-me".to_string());
    *blake3::hash(raw.as_bytes()).as_bytes()
}

/// Encryption subkey — used by ChaCha20-Poly1305 to encrypt/decrypt channel values.
fn enc_subkey() -> [u8; 32] {
    let master = master_key();
    let mut hasher = Hasher::new_keyed(&master);
    hasher.update(ENC_CONTEXT);
    *hasher.finalize().as_bytes()
}

/// Lookup subkey — used by BLAKE3 keyed hash to produce the deterministic
/// lookup hash stored in `value_lookup_hash` (allows DB index lookups without
/// decrypting the stored ciphertext).
fn lkp_subkey() -> [u8; 32] {
    let master = master_key();
    let mut hasher = Hasher::new_keyed(&master);
    hasher.update(LKP_CONTEXT);
    *hasher.finalize().as_bytes()
}

/// Encrypt a channel value. Returns `(ciphertext, nonce)`.
/// Nonce is 12 random bytes; store alongside the row.
pub fn encrypt_channel_value(plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12]), String> {
    let key = enc_subkey();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("encrypt_channel_value: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());
    Ok((ciphertext, nonce_bytes))
}

/// Decrypt a channel value. Returns the plaintext bytes.
pub fn decrypt_channel_value(ciphertext: &[u8], nonce: &[u8; 12]) -> Result<Vec<u8>, String> {
    let key = enc_subkey();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt_channel_value: {e}"))
}

/// Compute the deterministic lookup hash for a normalized channel value.
/// Returns a 64-character hex string.
pub fn compute_lookup_hash(normalized_value: &str) -> String {
    let key = lkp_subkey();
    let mut hasher = Hasher::new_keyed(&key);
    hasher.update(normalized_value.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Normalize a channel value to a canonical form for consistent hashing.
///
/// - `email`: trim + lowercase
/// - `sms` / `whatsapp`: retain only digits, but preserve a leading `+`
pub fn normalize_channel_value(channel_type: &str, value: &str) -> String {
    match channel_type {
        "email" => value.trim().to_lowercase(),
        "sms" | "whatsapp" => {
            let value = value.trim();
            let has_plus = value.starts_with('+');
            let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
            if has_plus {
                format!("+{digits}")
            } else {
                digits
            }
        }
        _ => value.trim().to_string(),
    }
}

/// Mask a channel value for display.
///
/// - `email`: `j***@domain.com`
/// - `sms`: `****1234`
/// - `whatsapp`: `wa:****1234`
pub fn mask_channel_value(channel_type: &str, value: &str) -> String {
    match channel_type {
        "email" => {
            if let Some(at_pos) = value.find('@') {
                let local = &value[..at_pos];
                let domain = &value[at_pos..]; // includes the '@'
                if local.is_empty() {
                    return format!("***{domain}");
                }
                let first = &local[..local
                    .char_indices()
                    .next()
                    .map(|(_, c)| c.len_utf8())
                    .unwrap_or(1)];
                format!("{first}***{domain}")
            } else {
                "***".to_string()
            }
        }
        "sms" => {
            let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
            let tail = if digits.len() >= 4 {
                &digits[digits.len() - 4..]
            } else {
                &digits
            };
            format!("****{tail}")
        }
        "whatsapp" => {
            let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
            let tail = if digits.len() >= 4 {
                &digits[digits.len() - 4..]
            } else {
                &digits
            };
            format!("wa:****{tail}")
        }
        _ => "***".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate_env() {
        unsafe { std::env::set_var("CO_RECOVERY_KEY", "test-recovery-key") };
    }

    #[test]
    fn round_trip() {
        isolate_env();
        let plaintext = b"test-email@example.com";
        let (ct, nonce) = encrypt_channel_value(plaintext).unwrap();
        assert_ne!(ct, plaintext);
        let pt = decrypt_channel_value(&ct, &nonce).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn hash_deterministic() {
        isolate_env();
        let h1 = compute_lookup_hash("alice@example.com");
        let h2 = compute_lookup_hash("alice@example.com");
        let h3 = compute_lookup_hash("bob@example.com");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn normalize_email() {
        assert_eq!(
            normalize_channel_value("email", "  Alice@EXAMPLE.COM  "),
            "alice@example.com"
        );
        assert_eq!(
            normalize_channel_value("email", "user@domain.org"),
            "user@domain.org"
        );
    }

    #[test]
    fn normalize_phone() {
        assert_eq!(
            normalize_channel_value("sms", "+55 (11) 9 8765-4321"),
            "+5511987654321"
        );
        assert_eq!(
            normalize_channel_value("whatsapp", "+1-800-555-0199"),
            "+18005550199"
        );
        assert_eq!(normalize_channel_value("sms", "11987654321"), "11987654321");
    }

    #[test]
    fn mask_email() {
        assert_eq!(
            mask_channel_value("email", "john@example.com"),
            "j***@example.com"
        );
        assert_eq!(mask_channel_value("email", "a@b.com"), "a***@b.com");
    }

    #[test]
    fn mask_phone() {
        assert_eq!(mask_channel_value("sms", "+5511987654321"), "****4321");
        assert_eq!(mask_channel_value("sms", "11987654321"), "****4321");
    }

    #[test]
    fn mask_whatsapp() {
        assert_eq!(
            mask_channel_value("whatsapp", "+5511987654321"),
            "wa:****4321"
        );
    }
}
