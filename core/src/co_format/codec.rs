//! CO-86 — compression adapters for the `.co` body (zstd via the workspace
//! `zstd` dep). Compression is a *layer* around the envelope: the plaintext
//! markdown body becomes `body.compressed`; everything else is unchanged.

use super::{CoError, CoFile, co_file};

/// Default zstd level — a good ratio/speed balance for content payloads.
pub const DEFAULT_LEVEL: i32 = 3;

/// Maximum zstd level (slow, best ratio) — used by the `--max` CLI flag.
pub const MAX_LEVEL: i32 = 19;

/// Compress `data` with zstd at `level`.
pub fn zstd_compress(data: &[u8], level: i32) -> Result<Vec<u8>, CoError> {
    zstd::encode_all(data, level).map_err(CoError::Codec)
}

/// Decompress zstd `data`.
pub fn zstd_decompress(data: &[u8]) -> Result<Vec<u8>, CoError> {
    zstd::decode_all(data).map_err(CoError::Codec)
}

/// The codec label for a zstd level, as recorded in [`super::Telemetry::codec`].
pub fn codec_label(level: i32) -> String {
    format!("zstd-{level}")
}

/// Replace a plaintext markdown body with its zstd-compressed form in place.
/// No-op (returns `Ok`) if the body is not the plaintext `markdown` variant.
pub fn compress_body(co: &mut CoFile, level: i32) -> Result<(), CoError> {
    if let Some(co_file::Body::Markdown(md)) = co.body.as_ref() {
        let compressed = zstd_compress(md, level)?;
        co.body = Some(co_file::Body::Compressed(compressed));
    }
    Ok(())
}

/// Replace a compressed body with its decompressed plaintext `markdown` form.
/// No-op if the body is not the `compressed` variant.
pub fn decompress_body(co: &mut CoFile) -> Result<(), CoError> {
    if let Some(co_file::Body::Compressed(c)) = co.body.as_ref() {
        let plain = zstd_decompress(c)?;
        co.body = Some(co_file::Body::Markdown(plain));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co_format::{canonical_body, from_markdown};

    #[test]
    fn compress_decompress_roundtrip() {
        let md = "---\ntitle: T\n---\n# Heading\n\nlots of repeated text ".repeat(20);
        let mut co = from_markdown(&md);
        let plain = canonical_body(&co).unwrap();

        compress_body(&mut co, DEFAULT_LEVEL).unwrap();
        assert!(matches!(co.body, Some(co_file::Body::Compressed(_))));
        // canonical_body transparently decompresses.
        assert_eq!(canonical_body(&co).unwrap(), plain);

        decompress_body(&mut co).unwrap();
        assert!(matches!(co.body, Some(co_file::Body::Markdown(_))));
        assert_eq!(canonical_body(&co).unwrap(), plain);
    }

    #[test]
    fn compression_shrinks_repetitive_content() {
        let body = "abcabcabc ".repeat(500);
        let compressed = zstd_compress(body.as_bytes(), DEFAULT_LEVEL).unwrap();
        assert!(compressed.len() < body.len());
    }
}
