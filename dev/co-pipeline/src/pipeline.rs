//! CO-88 — the encode → transport → decode → compare cycle for one file.
//!
//! Reuses the real CO building blocks so the bytes that travel are the bytes
//! production uses:
//! - the `CoFile` protobuf envelope (CO-86, `co::proto::sync::CoFile`),
//! - zstd-3 compression (same level as the sync codec),
//! - ChaCha20-Poly1305 with a per-universe deterministic DEK (mirrors the asset
//!   crypto envelope, CO-148),
//! - Ed25519 detached signatures (CO-87 signing layer).
//!
//! Keys and nonces are **deterministic** so a report diffs cleanly between runs
//! (the acceptance criterion). This is a UAT harness, not a production keyring.

use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use co::proto::sync::CoFile;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use prost::Message;
use sha2::{Digest, Sha256};

use crate::combo::Combo;

/// Per-file timing + size measurements, captured around the encode/decode
/// phases. Mirrors the `Telemetry` shape recorded server-side (CO-88b).
#[derive(Debug, Clone, Copy)]
pub struct Telemetry {
    /// Raw authoring bytes (markdown body before any layer).
    pub size_uncompressed: usize,
    /// Final bytes that travel the wire (after every applied layer).
    pub size_on_wire: usize,
    /// Nanoseconds spent in [`encode`].
    pub encode_ns: u128,
    /// Nanoseconds spent in [`decode`].
    pub decode_ns: u128,
}

impl Telemetry {
    /// `size_uncompressed / size_on_wire`; `1.0` when nothing shrank.
    pub fn compression_ratio(&self) -> f64 {
        if self.size_on_wire == 0 {
            return 1.0;
        }
        self.size_uncompressed as f64 / self.size_on_wire as f64
    }
}

const DEK_CONTEXT: &[u8] = b"co-pipeline-dek-v1\0";
const SIGNING_SEED_CONTEXT: &[u8] = b"co-pipeline-signing-seed-v1";

/// Deterministic per-universe data-encryption key (32 bytes).
fn derive_dek(universe_key: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DEK_CONTEXT);
    hasher.update(universe_key.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Deterministic Ed25519 signing key shared by the harness.
fn signing_key() -> SigningKey {
    let seed = blake3::hash(SIGNING_SEED_CONTEXT);
    SigningKey::from_bytes(seed.as_bytes())
}

fn encrypt(plaintext: &[u8], universe_key: &str) -> Result<Vec<u8>> {
    let key = derive_dek(universe_key);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    // Deterministic nonce from the plaintext digest — fine for a UAT harness
    // where reproducible bytes matter more than nonce-reuse resistance.
    let digest = Sha256::digest(plaintext);
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&digest[..12]);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| anyhow!("chacha20 encrypt failed: {e}"))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt(wire: &[u8], universe_key: &str) -> Result<Vec<u8>> {
    if wire.len() < 12 {
        bail!("ciphertext too short to contain a nonce");
    }
    let (nonce_bytes, ciphertext) = wire.split_at(12);
    let key = derive_dek(universe_key);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| anyhow!("chacha20 decrypt failed: {e}"))
}

/// Encode an authoring markdown body into wire bytes for the given combo.
///
/// Returns the wire bytes plus the elapsed encode time in nanoseconds.
pub fn encode(
    path: &str,
    content: &[u8],
    combo: Combo,
    universe_key: &str,
) -> Result<(Vec<u8>, u128)> {
    let start = Instant::now();

    // legacy-md: ship the bare markdown body; the receiver auto-wraps.
    if combo.is_legacy() {
        return Ok((content.to_vec(), start.elapsed().as_nanos()));
    }

    let sha = hex_sha256(content);
    let cofile = CoFile {
        path: path.to_string(),
        content: content.to_vec(),
        sha256: sha,
        mime_type: "text/markdown".to_string(),
        modified_ns: 0, // fixed → deterministic encode
    };
    let mut bytes = cofile.encode_to_vec();

    if combo.compresses() {
        bytes = zstd::encode_all(bytes.as_slice(), 3).context("zstd encode")?;
    }
    if combo.encrypts() {
        bytes = encrypt(&bytes, universe_key)?;
    }
    if combo.signs() {
        let sig = signing_key().sign(&bytes);
        bytes.extend_from_slice(&sig.to_bytes());
    }

    let ns = start.elapsed().as_nanos();
    Ok((bytes, ns))
}

/// Decode wire bytes back into the authoring markdown body.
///
/// Returns the decoded body plus the elapsed decode time in nanoseconds.
pub fn decode(wire: &[u8], combo: Combo, universe_key: &str) -> Result<(Vec<u8>, u128)> {
    let start = Instant::now();

    // legacy-md: the receiver auto-wraps bare markdown into a CoFile and reads
    // the body back out — a no-op on the bytes, exercising the compat path.
    if combo.is_legacy() {
        let wrapped = CoFile {
            path: String::new(),
            content: wire.to_vec(),
            sha256: hex_sha256(wire),
            mime_type: "text/markdown".to_string(),
            modified_ns: 0,
        };
        return Ok((wrapped.content, start.elapsed().as_nanos()));
    }

    let mut bytes = wire.to_vec();

    if combo.signs() {
        if bytes.len() < 64 {
            bail!("signed payload shorter than a 64-byte signature");
        }
        let split = bytes.len() - 64;
        let sig_bytes: [u8; 64] = bytes[split..]
            .try_into()
            .map_err(|_| anyhow!("signature slice not 64 bytes"))?;
        let signed = bytes[..split].to_vec();
        let key = signing_key();
        key.verifying_key()
            .verify(&signed, &Signature::from_bytes(&sig_bytes))
            .context("Ed25519 signature verification failed")?;
        bytes = signed;
    }
    if combo.encrypts() {
        bytes = decrypt(&bytes, universe_key)?;
    }
    if combo.compresses() {
        bytes = zstd::decode_all(bytes.as_slice()).context("zstd decode")?;
    }

    let cofile = CoFile::decode(bytes.as_slice()).context("CoFile protobuf decode")?;

    // Integrity: the embedded sha256 must match the decoded body.
    let actual = hex_sha256(&cofile.content);
    if actual != cofile.sha256 {
        bail!(
            "sha256 mismatch: header {} != body {}",
            cofile.sha256,
            actual
        );
    }

    Ok((cofile.content, start.elapsed().as_nanos()))
}

/// Encode then decode in-process, returning the decoded body and telemetry.
/// Does **not** compare against the input — that is the caller's job.
pub fn roundtrip(
    path: &str,
    content: &[u8],
    combo: Combo,
    universe_key: &str,
) -> Result<(Vec<u8>, Telemetry)> {
    let (wire, encode_ns) = encode(path, content, combo, universe_key)?;
    let size_on_wire = wire.len();
    let (decoded, decode_ns) = decode(&wire, combo, universe_key)?;
    Ok((
        decoded,
        Telemetry {
            size_uncompressed: content.len(),
            size_on_wire,
            encode_ns,
            decode_ns,
        },
    ))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] = b"---\ntitle: Hello\ntags: [test]\n---\n\n# Hello world\n\nA paragraph with some repeated words words words words.\n";

    #[test]
    fn every_combo_roundtrips_byte_for_byte() {
        for combo in Combo::ALL {
            let (decoded, _tel) = roundtrip("notes/a.md", BODY, combo, "artelonga")
                .unwrap_or_else(|e| panic!("{} failed: {e}", combo.label()));
            assert_eq!(decoded, BODY, "combo {} drifted", combo.label());
        }
    }

    #[test]
    fn zstd_actually_compresses_repetitive_content() {
        let repetitive = "lorem ipsum dolor sit amet ".repeat(200);
        let (_d, tel) = roundtrip("a.md", repetitive.as_bytes(), Combo::Zstd, "u").unwrap();
        assert!(tel.size_on_wire < tel.size_uncompressed);
        assert!(tel.compression_ratio() > 1.5);
    }

    #[test]
    fn encode_is_deterministic_across_runs() {
        for combo in Combo::ALL {
            let (a, _) = encode("p.md", BODY, combo, "artelonga").unwrap();
            let (b, _) = encode("p.md", BODY, combo, "artelonga").unwrap();
            assert_eq!(a, b, "combo {} non-deterministic", combo.label());
        }
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let (mut wire, _) = encode("p.md", BODY, Combo::Signed, "u").unwrap();
        let len = wire.len();
        wire[len - 1] ^= 0xff; // flip a signature bit
        assert!(decode(&wire, Combo::Signed, "u").is_err());
    }

    #[test]
    fn wrong_universe_key_fails_to_decrypt() {
        let (wire, _) = encode("p.md", BODY, Combo::Private, "artelonga").unwrap();
        assert!(decode(&wire, Combo::Private, "quilomboaraucaria").is_err());
    }
}
