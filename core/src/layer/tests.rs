//! CO-87 acceptance tests: end-to-end stacks, layer swapping, isolation mocks,
//! and the round-trip property over compression × encryption × signing.

use super::cache::LruCache;
use super::compression::{BrotliCompressed, ZstdCompressed};
use super::measured::Measured;
use super::physical::FilesystemBytes;
use super::privacy::{ChaCha20Encrypted, PassthroughPlain};
use super::security::{Ed25519Signed, UnverifiedTrusted};
use super::{Layer, LayerError, LayerExt};
use crate::proto::sync::CoFile;

fn cofile(path: &str, body: &[u8]) -> CoFile {
    CoFile {
        path: path.into(),
        content: body.to_vec(),
        sha256: String::new(),
        mime_type: "text/markdown".into(),
        modified_ns: 1_700_000_000_000_000_000,
    }
}

/// A read request: the addressing metadata with `content` cleared.
fn key_of(c: &CoFile) -> CoFile {
    CoFile {
        content: Vec::new(),
        ..c.clone()
    }
}

const SEED: [u8; 32] = [42u8; 32];
const ENC_KEY: [u8; 32] = [7u8; 32];

#[test]
fn five_layer_stack_round_trips_bit_perfect() {
    let dir = tempfile::tempdir().unwrap();
    // Filesystem + Signed + Encrypted + Compressed + LRU.
    // Build order makes the on-disk bytes sign(encrypt(compress(plain))):
    // compression below encryption, privacy below security.
    let stack = FilesystemBytes::new(dir.path())
        .stack(Ed25519Signed::new(SEED))
        .stack(ChaCha20Encrypted::new(ENC_KEY))
        .stack(ZstdCompressed::new(3))
        .stack(LruCache::new(64));

    let original = cofile(
        "diary/2026-04-27.md",
        b"# Private thoughts\n\nThe quick brown fox."
            .repeat(4)
            .as_slice(),
    );
    stack.write(original.clone()).unwrap();

    let got = stack.read(key_of(&original)).unwrap();
    assert_eq!(got.content, original.content, "content must round-trip");
    assert_eq!(got.path, original.path);
    assert_eq!(got.mime_type, original.mime_type);
    assert_eq!(got.modified_ns, original.modified_ns);
}

#[test]
fn swapping_one_layer_requires_no_other_changes() {
    // Same shape, but PassthroughPlain instead of ChaCha20Encrypted, and
    // UnverifiedTrusted instead of Ed25519Signed. Zero changes to surrounding
    // code — only the layer constructors differ.
    let dir = tempfile::tempdir().unwrap();
    let stack = FilesystemBytes::new(dir.path())
        .stack(UnverifiedTrusted)
        .stack(PassthroughPlain)
        .stack(ZstdCompressed::new(3))
        .stack(LruCache::new(64));

    let original = cofile("public/readme.md", b"# Hello, world");
    stack.write(original.clone()).unwrap();
    let got = stack.read(key_of(&original)).unwrap();
    assert_eq!(got.content, original.content);
}

/// A hand-rolled mock layer, demonstrating each layer is testable in isolation
/// against the `Layer` trait surface with no real I/O.
struct MockLayer {
    tag: u8,
}
impl Layer for MockLayer {
    super::cofile_io!();
    type Error = LayerError;
    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        // reverse of write: strip the tag byte we appended.
        match input.content.pop() {
            Some(b) if b == self.tag => Ok(input),
            _ => Err(LayerError::Encoding("mock tag mismatch".into())),
        }
    }
    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        input.content.push(self.tag);
        Ok(input)
    }
}

#[test]
fn each_layer_mocks_in_isolation() {
    // The mock composes like any real layer.
    let stack = MockLayer { tag: 0xAB }.stack(MockLayer { tag: 0xCD });
    let original = cofile("x.md", b"payload");
    let written = stack.write(original.clone()).unwrap();
    // top (0xCD) then bottom (0xAB) appended on write.
    assert_eq!(written.content.last(), Some(&0xABu8));
    let back = stack.read(key_with(&original, written.content)).unwrap();
    assert_eq!(back.content, original.content);
}

fn key_with(c: &CoFile, content: Vec<u8>) -> CoFile {
    CoFile {
        content,
        ..c.clone()
    }
}

#[test]
fn mock_layer_propagates_named_errors() {
    let stack = MockLayer { tag: 1 }.stack(MockLayer { tag: 2 });
    // Feed bytes that don't end in the expected tags → the named layer error
    // surfaces through StackError.
    let err = stack.read(cofile("x.md", b"\x00\x00")).unwrap_err();
    assert!(err.to_string().contains("layer `"), "got: {err}");
}

/// Round-trip property: every combination of {zstd, brotli, none} ×
/// {chacha, plain} × {ed25519, trusted} round-trips bit-perfect over a set of
/// content samples. Exhaustive enumeration stands in for a property test.
#[test]
fn property_compression_x_encryption_x_signing_round_trips() {
    let samples: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"a".to_vec(),
        b"# markdown\n\nwith some prose and `code`.".to_vec(),
        (0u8..=255).cycle().take(4096).collect(), // binary-ish
        b"repeat ".iter().cycle().take(10_000).copied().collect(),
    ];

    #[derive(Clone, Copy)]
    enum Comp {
        None,
        Zstd,
        Brotli,
    }
    #[derive(Clone, Copy)]
    enum Enc {
        Plain,
        ChaCha,
    }
    #[derive(Clone, Copy)]
    enum Sec {
        Trusted,
        Signed,
    }

    let comps = [Comp::None, Comp::Zstd, Comp::Brotli];
    let encs = [Enc::Plain, Enc::ChaCha];
    let secs = [Sec::Trusted, Sec::Signed];

    for &comp in &comps {
        for &enc in &encs {
            for &sec in &secs {
                for sample in &samples {
                    let original = cofile("p.md", sample);

                    // Apply write transforms top→bottom: compress → encrypt → sign.
                    let mut cur = original.clone();
                    cur = apply_comp_write(comp, cur);
                    cur = apply_enc_write(enc, cur);
                    cur = apply_sec_write(sec, cur);

                    // Reverse on read bottom→top: verify → decrypt → decompress.
                    cur = apply_sec_read(sec, cur);
                    cur = apply_enc_read(enc, cur);
                    cur = apply_comp_read(comp, cur);

                    assert_eq!(
                        cur.content,
                        original.content,
                        "round-trip failed for sample len {}",
                        sample.len()
                    );
                }
            }
        }
    }

    fn apply_comp_write(c: Comp, f: CoFile) -> CoFile {
        match c {
            Comp::None => f,
            Comp::Zstd => ZstdCompressed::new(3).write(f).unwrap(),
            Comp::Brotli => BrotliCompressed::new(5).write(f).unwrap(),
        }
    }
    fn apply_comp_read(c: Comp, f: CoFile) -> CoFile {
        match c {
            Comp::None => f,
            Comp::Zstd => ZstdCompressed::new(3).read(f).unwrap(),
            Comp::Brotli => BrotliCompressed::new(5).read(f).unwrap(),
        }
    }
    fn apply_enc_write(e: Enc, f: CoFile) -> CoFile {
        match e {
            Enc::Plain => PassthroughPlain.write(f).unwrap(),
            Enc::ChaCha => ChaCha20Encrypted::new(ENC_KEY).write(f).unwrap(),
        }
    }
    fn apply_enc_read(e: Enc, f: CoFile) -> CoFile {
        match e {
            Enc::Plain => PassthroughPlain.read(f).unwrap(),
            Enc::ChaCha => ChaCha20Encrypted::new(ENC_KEY).read(f).unwrap(),
        }
    }
    fn apply_sec_write(s: Sec, f: CoFile) -> CoFile {
        match s {
            Sec::Trusted => UnverifiedTrusted.write(f).unwrap(),
            Sec::Signed => Ed25519Signed::new(SEED).write(f).unwrap(),
        }
    }
    fn apply_sec_read(s: Sec, f: CoFile) -> CoFile {
        match s {
            Sec::Trusted => UnverifiedTrusted.read(f).unwrap(),
            Sec::Signed => Ed25519Signed::new(SEED).read(f).unwrap(),
        }
    }
}

#[test]
fn measured_wrapper_reports_per_layer_timing() {
    // Bench-shaped: wrap each layer in Measured, run the stack, read the
    // encode/decode counters back (CO-86-style telemetry).
    let dir = tempfile::tempdir().unwrap();
    let zstd = Measured::new(ZstdCompressed::new(3), "zstd");
    let zstd_metrics = zstd.metrics();
    let stack = FilesystemBytes::new(dir.path())
        .stack(Ed25519Signed::new(SEED))
        .stack(ChaCha20Encrypted::new(ENC_KEY))
        .stack(zstd);

    let original = cofile("m.md", b"measure me ".repeat(50).as_slice());
    stack.write(original.clone()).unwrap();
    stack.read(key_of(&original)).unwrap();

    assert_eq!(zstd_metrics.writes(), 1);
    assert_eq!(zstd_metrics.reads(), 1);
    assert_eq!(zstd_metrics.errors(), 0);
}
