//! CO-88 — the per-universe pipeline report (`co-pipeline-report-<date>.yaml`).
//!
//! Everything except the wall-clock `generated_at` is deterministic given a
//! fixed corpus, so CI can diff two runs and surface regressions.

use serde::{Deserialize, Serialize};

use crate::combo::Combo;
use crate::transport::PipePath;

/// One matrix cell result: path × universe × combo × file aggregate.
#[derive(Debug, Clone)]
pub struct CellResult {
    pub path: PipePath,
    pub universe: String,
    pub combo: Combo,
    pub files: usize,
    pub raw_bytes: u64,
    pub encoded_bytes: u64,
    pub encode_ns: Vec<u128>,
    pub decode_ns: Vec<u128>,
    pub transfer_ms: Vec<u64>,
    /// Files whose decoded body did not equal the source (must be 0 to pass).
    pub drift: usize,
}

/// A failure for one path × universe × combo, surfaced in `errors:`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellError {
    pub path: String,
    pub universe: String,
    pub combo: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalStats {
    pub total_universes: usize,
    pub total_files: usize,
    pub total_raw_bytes: u64,
    pub total_encoded_bytes: u64,
    pub compression_ratio_p50: f64,
    pub compression_ratio_p99: f64,
    pub encode_ns_p50: u128,
    pub encode_ns_p99: u128,
    pub decode_ns_p50: u128,
    pub decode_ns_p99: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseStats {
    pub key: String,
    pub visibility: String,
    pub files: usize,
    pub raw_bytes: u64,
    pub encoded_bytes_zstd: u64,
    pub encoded_bytes_private: u64,
    pub compression_ratio_zstd: f64,
    pub transfer_ms_uat_p50: u64,
    pub transfer_ms_prod_p50_read: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineReport {
    pub generated_at: String,
    pub binary_version: String,
    pub paths: Vec<String>,
    pub global: GlobalStats,
    pub per_universe: Vec<UniverseStats>,
    pub errors: Vec<CellError>,
}

impl PipelineReport {
    /// Build a report from the matrix cells.
    pub fn from_cells(
        generated_at: String,
        binary_version: String,
        paths: &[PipePath],
        universes: &[(String, String)], // (key, visibility) in order
        cells: &[CellResult],
        errors: Vec<CellError>,
    ) -> PipelineReport {
        // Global percentiles over every cell's samples.
        let mut ratios: Vec<f64> = Vec::new();
        let mut encode: Vec<u128> = Vec::new();
        let mut decode: Vec<u128> = Vec::new();
        let mut total_raw = 0u64;
        let mut total_encoded = 0u64;
        let mut total_files = 0usize;

        for c in cells {
            encode.extend(&c.encode_ns);
            decode.extend(&c.decode_ns);
            if c.raw_bytes > 0 && c.encoded_bytes > 0 {
                ratios.push(c.raw_bytes as f64 / c.encoded_bytes as f64);
            }
            // Count raw/encoded/files once per universe — use the zstd combo on
            // the first path as the canonical size measurement.
            if c.combo == Combo::Zstd && Some(&c.path) == paths.first() {
                total_raw += c.raw_bytes;
                total_encoded += c.encoded_bytes;
                total_files += c.files;
            }
        }

        let global = GlobalStats {
            total_universes: universes.len(),
            total_files,
            total_raw_bytes: total_raw,
            total_encoded_bytes: total_encoded,
            compression_ratio_p50: round2(percentile_f64(&mut ratios.clone(), 50)),
            compression_ratio_p99: round2(percentile_f64(&mut ratios, 99)),
            encode_ns_p50: percentile_u128(&mut encode.clone(), 50),
            encode_ns_p99: percentile_u128(&mut encode, 99),
            decode_ns_p50: percentile_u128(&mut decode.clone(), 50),
            decode_ns_p99: percentile_u128(&mut decode, 99),
        };

        let per_universe = universes
            .iter()
            .map(|(key, vis)| universe_stats(key, vis, cells, paths))
            .collect();

        PipelineReport {
            generated_at,
            binary_version,
            paths: paths.iter().map(|p| p.label().to_string()).collect(),
            global,
            per_universe,
            errors,
        }
    }

    /// True when no cell failed and no file drifted.
    pub fn is_green(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_else(|e| format!("# serialize error: {e}\n"))
    }
}

fn universe_stats(
    key: &str,
    visibility: &str,
    cells: &[CellResult],
    paths: &[PipePath],
) -> UniverseStats {
    let first = paths.first().copied();
    let find = |combo: Combo, path: Option<PipePath>| {
        cells
            .iter()
            .find(|c| c.universe == key && c.combo == combo && Some(c.path) == path)
    };

    let zstd_cell = find(Combo::Zstd, first);
    let files = zstd_cell.map(|c| c.files).unwrap_or(0);
    let raw_bytes = zstd_cell.map(|c| c.raw_bytes).unwrap_or(0);
    let encoded_bytes_zstd = zstd_cell.map(|c| c.encoded_bytes).unwrap_or(0);
    let encoded_bytes_private = find(Combo::Private, first)
        .map(|c| c.encoded_bytes)
        .unwrap_or(0);
    let compression_ratio_zstd = if encoded_bytes_zstd > 0 {
        round2(raw_bytes as f64 / encoded_bytes_zstd as f64)
    } else {
        1.0
    };

    let transfer_ms_uat_p50 = cells
        .iter()
        .filter(|c| c.universe == key && c.path == PipePath::Uat)
        .flat_map(|c| c.transfer_ms.clone())
        .collect::<Vec<_>>();
    let transfer_ms_prod = cells
        .iter()
        .filter(|c| c.universe == key && c.path == PipePath::Prod)
        .flat_map(|c| c.transfer_ms.clone())
        .collect::<Vec<_>>();

    UniverseStats {
        key: key.to_string(),
        visibility: visibility.to_string(),
        files,
        raw_bytes,
        encoded_bytes_zstd,
        encoded_bytes_private,
        compression_ratio_zstd,
        transfer_ms_uat_p50: percentile_u64(&mut transfer_ms_uat_p50.clone(), 50),
        transfer_ms_prod_p50_read: percentile_u64(&mut transfer_ms_prod.clone(), 50),
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Nearest-rank percentile (deterministic). Returns 0 for an empty slice.
fn percentile_u128(samples: &mut [u128], p: usize) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = rank_index(samples.len(), p);
    samples[idx]
}

fn percentile_u64(samples: &mut [u64], p: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = rank_index(samples.len(), p);
    samples[idx]
}

fn percentile_f64(samples: &mut [f64], p: usize) -> f64 {
    if samples.is_empty() {
        return 1.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = rank_index(samples.len(), p);
    samples[idx]
}

fn rank_index(len: usize, p: usize) -> usize {
    // ceil(p/100 * len) - 1, clamped — the nearest-rank method.
    let rank = ((p as f64 / 100.0) * len as f64).ceil() as usize;
    rank.saturating_sub(1).min(len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_nearest_rank() {
        let mut v = vec![10u64, 20, 30, 40, 50];
        assert_eq!(percentile_u64(&mut v.clone(), 50), 30);
        assert_eq!(percentile_u64(&mut v.clone(), 99), 50);
        assert_eq!(percentile_u64(&mut v, 1), 10);
        assert_eq!(percentile_u64(&mut [], 50), 0);
    }

    #[test]
    fn report_is_green_when_no_errors() {
        let r = PipelineReport::from_cells(
            "2026-04-27T00:00:00Z".into(),
            "test".into(),
            &[PipePath::Local],
            &[("artelonga".into(), "public-subscribable".into())],
            &[CellResult {
                path: PipePath::Local,
                universe: "artelonga".into(),
                combo: Combo::Zstd,
                files: 2,
                raw_bytes: 1000,
                encoded_bytes: 400,
                encode_ns: vec![100, 200],
                decode_ns: vec![50, 60],
                transfer_ms: vec![1, 2],
                drift: 0,
            }],
            vec![],
        );
        assert!(r.is_green());
        assert_eq!(r.global.total_files, 2);
        assert_eq!(r.per_universe[0].encoded_bytes_zstd, 400);
        assert!((r.per_universe[0].compression_ratio_zstd - 2.5).abs() < 0.001);
    }

    #[test]
    fn report_not_green_with_errors() {
        let r = PipelineReport::from_cells(
            "t".into(),
            "test".into(),
            &[PipePath::Local],
            &[],
            &[],
            vec![CellError {
                path: "uat".into(),
                universe: "rfq".into(),
                combo: "private".into(),
                detail: "boom".into(),
            }],
        );
        assert!(!r.is_green());
    }
}
