//! CO-87 bench harness — pipeline throughput per layer.
//!
//! Wraps each layer of a write/read stack in [`Measured`] and reports the
//! per-layer `encode_ns` / `decode_ns` (CO-86-style telemetry).
//!
//! Run with:
//! ```text
//! cargo run --release --example layer_bench -p co
//! ```

use std::time::Instant;

use co::layer::compression::ZstdCompressed;
use co::layer::measured::{LayerMetrics, Measured};
use co::layer::physical::FilesystemBytes;
use co::layer::privacy::ChaCha20Encrypted;
use co::layer::security::Ed25519Signed;
use co::layer::{Layer, LayerExt};
use co::proto::sync::CoFile;
use std::sync::Arc;

const ITERS: usize = 2_000;

fn report(label: &str, m: &Arc<LayerMetrics>) {
    println!(
        "{label:>12}  writes={:>5} encode_ns/op={:>8}   reads={:>5} decode_ns/op={:>8}",
        m.writes(),
        m.mean_encode_ns(),
        m.reads(),
        m.mean_decode_ns(),
    );
}

fn main() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Compose Filesystem + Signed + Encrypted + Compressed, each Measured.
    let fs = Measured::new(FilesystemBytes::new(dir.path()), "filesystem");
    let signed = Measured::new(Ed25519Signed::new([42u8; 32]), "ed25519");
    let enc = Measured::new(ChaCha20Encrypted::new([7u8; 32]), "chacha20");
    let zstd = Measured::new(ZstdCompressed::new(3), "zstd");

    let (m_fs, m_signed, m_enc, m_zstd) = (
        fs.metrics(),
        signed.metrics(),
        enc.metrics(),
        zstd.metrics(),
    );

    let stack = fs.stack(signed).stack(enc).stack(zstd);

    let body = b"# Bench payload\n\nThe quick brown fox jumps over the lazy dog. ".repeat(16);
    let original = CoFile {
        path: "bench/sample.md".into(),
        content: body,
        sha256: String::new(),
        mime_type: "text/markdown".into(),
        modified_ns: 0,
    };
    let key = CoFile {
        content: Vec::new(),
        ..original.clone()
    };

    let wall = Instant::now();
    for _ in 0..ITERS {
        stack.write(original.clone()).expect("write");
        let got = stack.read(key.clone()).expect("read");
        debug_assert_eq!(got.content, original.content);
    }
    let elapsed = wall.elapsed();

    println!(
        "CO-87 layer bench — {ITERS} write+read round-trips in {:?} ({:.1} op/s)\n",
        elapsed,
        (ITERS as f64 * 2.0) / elapsed.as_secs_f64(),
    );
    println!("per-layer telemetry (encode = write dir, decode = read dir):");
    report("filesystem", &m_fs);
    report("ed25519", &m_signed);
    report("chacha20", &m_enc);
    report("zstd", &m_zstd);
}
