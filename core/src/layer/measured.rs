//! `Measured` — telemetry wrapper recording per-layer read/write metrics.
//!
//! Composable with any `CoFile → CoFile` layer; produces metrics (counts,
//! bytes-on-wire, cumulative latency) without changing the inner layer's
//! signature. The bench harness (`examples/layer_bench.rs`) reads `encode_ns` /
//! `decode_ns` from these.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// Cumulative metrics for a single [`Measured`] layer. Cheap atomic counters,
/// safe to read while writes proceed.
#[derive(Default)]
pub struct LayerMetrics {
    pub reads: AtomicU64,
    pub writes: AtomicU64,
    /// Total nanoseconds spent in `read` (the "decode" direction).
    pub decode_ns: AtomicU64,
    /// Total nanoseconds spent in `write` (the "encode" direction).
    pub encode_ns: AtomicU64,
    /// Bytes of `content` produced by reads.
    pub read_bytes: AtomicU64,
    /// Bytes of `content` ingested by writes.
    pub write_bytes: AtomicU64,
    pub errors: AtomicU64,
}

impl LayerMetrics {
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }
    pub fn decode_ns(&self) -> u64 {
        self.decode_ns.load(Ordering::Relaxed)
    }
    pub fn encode_ns(&self) -> u64 {
        self.encode_ns.load(Ordering::Relaxed)
    }
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Mean encode (write) time in nanoseconds, or 0 with no writes.
    pub fn mean_encode_ns(&self) -> u64 {
        let n = self.writes();
        if n == 0 { 0 } else { self.encode_ns() / n }
    }

    /// Mean decode (read) time in nanoseconds, or 0 with no reads.
    pub fn mean_decode_ns(&self) -> u64 {
        let n = self.reads();
        if n == 0 { 0 } else { self.decode_ns() / n }
    }
}

/// Wraps a layer, recording [`LayerMetrics`] on every read/write. The `label`
/// identifies the layer in reports.
pub struct Measured<L> {
    inner: L,
    label: String,
    metrics: Arc<LayerMetrics>,
}

impl<L> Measured<L> {
    pub fn new(inner: L, label: impl Into<String>) -> Self {
        Self {
            inner,
            label: label.into(),
            metrics: Arc::new(LayerMetrics::default()),
        }
    }

    /// A shared handle to the metrics — clone it before composing the layer
    /// into a stack so you can read counters afterwards.
    pub fn metrics(&self) -> Arc<LayerMetrics> {
        Arc::clone(&self.metrics)
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

impl<L> Layer for Measured<L>
where
    L: Layer<ReadInput = CoFile, ReadOutput = CoFile, WriteInput = CoFile, WriteOutput = CoFile>,
{
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let start = Instant::now();
        let result = self.inner.read(input);
        let elapsed = start.elapsed().as_nanos() as u64;
        self.metrics.decode_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.metrics.reads.fetch_add(1, Ordering::Relaxed);
        match result {
            Ok(cf) => {
                self.metrics
                    .read_bytes
                    .fetch_add(cf.content.len() as u64, Ordering::Relaxed);
                Ok(cf)
            }
            Err(e) => {
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                Err(LayerError::Backend(format!("{}: {e}", self.label)))
            }
        }
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let bytes = input.content.len() as u64;
        let start = Instant::now();
        let result = self.inner.write(input);
        let elapsed = start.elapsed().as_nanos() as u64;
        self.metrics.encode_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.metrics.writes.fetch_add(1, Ordering::Relaxed);
        self.metrics.write_bytes.fetch_add(bytes, Ordering::Relaxed);
        result.map_err(|e| {
            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
            LayerError::Backend(format!("{}: {e}", self.label))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::compression::ZstdCompressed;

    fn cofile(body: &[u8]) -> CoFile {
        CoFile {
            path: "a.md".into(),
            content: body.to_vec(),
            sha256: "s".into(),
            mime_type: "text/markdown".into(),
            modified_ns: 0,
        }
    }

    #[test]
    fn records_counts_and_bytes() {
        let m = Measured::new(ZstdCompressed::new(3), "zstd");
        let metrics = m.metrics();
        let written = m
            .write(cofile(b"hello world ".repeat(10).as_slice()))
            .unwrap();
        m.read(written).unwrap();

        assert_eq!(metrics.writes(), 1);
        assert_eq!(metrics.reads(), 1);
        assert_eq!(metrics.errors(), 0);
        // Timing is recorded (monotonic Instant; assert presence, not a value).
        assert_eq!(metrics.mean_encode_ns(), metrics.encode_ns());
    }

    #[test]
    fn counts_errors() {
        // zstd read on non-zstd bytes errors.
        let m = Measured::new(ZstdCompressed::new(3), "zstd");
        let metrics = m.metrics();
        assert!(m.read(cofile(b"not zstd")).is_err());
        assert_eq!(metrics.errors(), 1);
    }
}
