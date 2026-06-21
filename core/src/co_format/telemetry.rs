//! CO-86 — wire-level telemetry populated on encode. Tracks uncompressed vs
//! compressed vs on-wire sizes and the codec, feeding CO-88's per-universe
//! bytes-on-wire accounting.

use super::{CoError, CoFile, Telemetry, canonical_body, co_file, to_bytes};

/// Compute and attach [`Telemetry`] to `co` for the current body state.
///
/// - `size_uncompressed` — plaintext body length (post-decrypt/decompress)
/// - `size_compressed` — stored body length (compressed when zstd'd, else same)
/// - `size_on_wire` — full `.co` byte length (magic + protobuf, post-encryption)
/// - `codec` — `"raw"`, `"zstd-N"`, or `"encrypted"`
///
/// `encode_ns` is the measured encode duration (0 if not timed).
pub fn populate(co: &mut CoFile, encode_ns: i64) {
    let size_uncompressed = canonical_body(co).map(|b| b.len() as i64).unwrap_or(0);

    let (size_compressed, codec) = match co.body.as_ref() {
        Some(co_file::Body::Markdown(b)) => (b.len() as i64, "raw".to_string()),
        Some(co_file::Body::Compressed(b)) => (b.len() as i64, "zstd".to_string()),
        Some(co_file::Body::Composite(c)) => (c.markdown.len() as i64, "raw".to_string()),
        Some(co_file::Body::Encrypted(e)) => (e.ciphertext.len() as i64, "encrypted".to_string()),
        None => (0, "none".to_string()),
    };

    let size_on_wire = to_bytes(co).len() as i64;

    co.telemetry = Some(Telemetry {
        size_uncompressed,
        size_compressed,
        size_on_wire,
        codec,
        encode_ns,
        decode_ns: 0,
    });
}

/// Compression ratio (`size_compressed / size_uncompressed`) for the envelope's
/// telemetry, or `None` when telemetry is absent or the uncompressed size is 0.
pub fn compression_ratio(co: &CoFile) -> Option<f64> {
    let t = co.telemetry.as_ref()?;
    if t.size_uncompressed == 0 {
        return None;
    }
    Some(t.size_compressed as f64 / t.size_uncompressed as f64)
}

/// Convenience: the on-wire `.co` byte length for an envelope.
pub fn wire_size(co: &CoFile) -> Result<usize, CoError> {
    Ok(to_bytes(co).len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co_format::{codec, from_markdown};

    #[test]
    fn telemetry_records_sizes() {
        let md = format!("---\ntitle: T\n---\n{}", "repeated body ".repeat(100));
        let mut co = from_markdown(&md);
        codec::compress_body(&mut co, codec::DEFAULT_LEVEL).unwrap();
        populate(&mut co, 1234);

        let t = co.telemetry.as_ref().unwrap();
        assert!(t.size_uncompressed > 0);
        assert!(t.size_compressed > 0);
        assert!(t.size_compressed < t.size_uncompressed); // repetitive → shrinks
        assert!(t.size_on_wire > 0);
        assert_eq!(t.codec, "zstd");
        assert_eq!(t.encode_ns, 1234);
        assert!(compression_ratio(&co).unwrap() < 1.0);
    }

    #[test]
    fn raw_body_codec_label() {
        let mut co = from_markdown("body\n");
        populate(&mut co, 0);
        assert_eq!(co.telemetry.as_ref().unwrap().codec, "raw");
    }
}
