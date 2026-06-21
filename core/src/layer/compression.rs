//! Compression layers — transform `content` bytes, preserve metadata.
//!
//! Per the composition rules, compression sits *below* encryption: on write we
//! compress first so the encrypted payload leaks less structure.

use std::io::{Read, Write};

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// zstd compression. `write` compresses `content`; `read` decompresses.
pub struct ZstdCompressed {
    pub level: i32,
}

impl ZstdCompressed {
    pub fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Layer for ZstdCompressed {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        input.content = zstd::decode_all(input.content.as_slice())
            .map_err(|e| LayerError::Encoding(format!("zstd decode: {e}")))?;
        Ok(input)
    }

    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        input.content = zstd::encode_all(input.content.as_slice(), self.level)
            .map_err(|e| LayerError::Encoding(format!("zstd encode: {e}")))?;
        Ok(input)
    }
}

/// Brotli compression. `write` compresses `content`; `read` decompresses.
pub struct BrotliCompressed {
    pub quality: u32,
    pub window: u32,
}

impl BrotliCompressed {
    /// `quality` 0–11, `window` (lg) 10–24. Sensible defaults: q5, w22.
    pub fn new(quality: u32) -> Self {
        Self {
            quality,
            window: 22,
        }
    }
}

impl Layer for BrotliCompressed {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        let mut out = Vec::new();
        let mut reader =
            brotli::Decompressor::new(input.content.as_slice(), 4096 /* buffer */);
        reader
            .read_to_end(&mut out)
            .map_err(|e| LayerError::Encoding(format!("brotli decode: {e}")))?;
        input.content = out;
        Ok(input)
    }

    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        let mut out = Vec::new();
        {
            let mut writer =
                brotli::CompressorWriter::new(&mut out, 4096, self.quality, self.window);
            writer
                .write_all(&input.content)
                .map_err(|e| LayerError::Encoding(format!("brotli encode: {e}")))?;
        }
        input.content = out;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofile(body: &[u8]) -> CoFile {
        CoFile {
            path: "a.md".into(),
            content: body.to_vec(),
            sha256: "sha".into(),
            mime_type: "text/markdown".into(),
            modified_ns: 1,
        }
    }

    #[test]
    fn zstd_round_trips_and_preserves_metadata() {
        let z = ZstdCompressed::new(3);
        let original = cofile(
            b"the quick brown fox jumps over the lazy dog"
                .repeat(8)
                .as_slice(),
        );
        let written = z.write(original.clone()).unwrap();
        assert_ne!(written.content, original.content, "should be compressed");
        assert_eq!(written.path, "a.md");
        let back = z.read(written).unwrap();
        assert_eq!(back.content, original.content);
        assert_eq!(back.sha256, "sha");
        assert_eq!(back.modified_ns, 1);
    }

    #[test]
    fn brotli_round_trips() {
        let b = BrotliCompressed::new(5);
        let original = cofile(b"compress me ".repeat(20).as_slice());
        let written = b.write(original.clone()).unwrap();
        let back = b.read(written).unwrap();
        assert_eq!(back.content, original.content);
    }

    #[test]
    fn zstd_handles_empty() {
        let z = ZstdCompressed::new(1);
        let original = cofile(b"");
        let back = z.read(z.write(original.clone()).unwrap()).unwrap();
        assert_eq!(back.content, original.content);
    }
}
