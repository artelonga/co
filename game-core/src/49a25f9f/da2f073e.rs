use crate::engine::error::{GameError, Result};
use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, XChaCha20Poly1305, XNonce,
};
use obfstr::obfstr;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Hardcoded application salt (compile-time obfuscated)
fn app_salt() -> [u8; 32] {
    // obfstr! returns a temporary — copy bytes before it's dropped
    let mut salt = [0u8; 32];
    let src: &[u8] = &[
        0x67, 0x50, 0x38, 0x24, 0x6d, 0x4b, 0x32, 0x21, 0x76, 0x52, 0x35, 0x78, 0x4e, 0x39, 0x40,
        0x77, 0x4c, 0x34, 0x23, 0x74, 0x51, 0x37, 0x26, 0x6a, 0x46, 0x30, 0x5e, 0x68, 0x59, 0x36,
        0x2a, 0x7a,
    ]; // "gP8$mK2!vR5xN9@wL4#tQ7&jF0^hY6*z"
    salt.copy_from_slice(src);
    salt
}

/// Derive a machine-specific fingerprint
fn machine_fingerprint(db_path: &std::path::Path) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Username
    if let Ok(user) = std::env::var(obfstr!("USER")) {
        hasher.update(user.as_bytes());
    } else if let Ok(user) = std::env::var(obfstr!("USERNAME")) {
        hasher.update(user.as_bytes());
    }

    // Hostname
    if let Ok(host) = hostname::get() {
        let host_str = host.to_string_lossy();
        hasher.update(host_str.as_bytes());
    }

    // App salt
    hasher.update(app_salt());

    // DB path
    if let Some(path_str) = db_path.to_str() {
        hasher.update(path_str.as_bytes());
    }

    let result = hasher.finalize();
    let mut fingerprint = [0u8; 32];
    fingerprint.copy_from_slice(&result);
    fingerprint
}

/// Cached derived key for the process lifetime
static DERIVED_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Derive the master encryption key via Argon2id.
/// Result is cached for the process lifetime.
pub fn derive_key(db_path: &std::path::Path) -> Result<[u8; 32]> {
    if let Some(key) = DERIVED_KEY.get() {
        return Ok(*key);
    }

    let fingerprint = machine_fingerprint(db_path);
    let salt = app_salt();
    let argon_salt = match salt.get(..16) {
        Some(s) => s,
        None => return Err(GameError::Storage("Salt derivation failed".into())),
    };

    let mut key = [0u8; 32];
    let argon = Argon2::default();
    match argon.hash_password_into(&fingerprint, argon_salt, &mut key) {
        Ok(()) => {}
        Err(_) => return Err(GameError::Storage("Key derivation failed".into())),
    }

    let _ = DERIVED_KEY.set(key);
    Ok(key)
}

/// Encrypt plaintext bytes. Returns nonce || ciphertext.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = match cipher.encrypt(&nonce, plaintext) {
        Ok(ct) => ct,
        Err(_) => return Err(GameError::Storage("Encryption failed".into())),
    };

    // Prepend nonce (24 bytes) to ciphertext
    let mut output = Vec::with_capacity(24 + ciphertext.len());
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt nonce || ciphertext. Returns plaintext bytes.
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 24 {
        return Err(GameError::Storage("Data too short to decrypt".into()));
    }

    let (nonce_bytes, ciphertext) = data.split_at(24);
    let nonce = XNonce::from_slice(nonce_bytes);
    let cipher = XChaCha20Poly1305::new(key.into());

    match cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => Ok(plaintext),
        Err(_) => Err(GameError::Storage("Decryption failed".into())),
    }
}
