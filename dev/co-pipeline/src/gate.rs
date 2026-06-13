//! CO-88c/CO-88d — CI delta comparison + the pre-prod-deploy gate.
//!
//! - [`compute_deltas`] diffs a current report against the previous run and
//!   surfaces compression-ratio / transfer-time movements (CO-88c, PR comment).
//! - [`evaluate_gate`] is the deploy gate (CO-88d): the report must be green,
//!   younger than a max age, and free of regressions beyond a tolerance.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::report::PipelineReport;

/// One per-universe movement between two runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    pub universe: String,
    pub metric: String,
    pub previous: f64,
    pub current: f64,
    /// Signed percent change `(current - previous) / previous * 100`.
    pub pct_change: f64,
}

impl Delta {
    /// Human-readable one-liner, e.g. `artelonga compression_ratio_zstd: 2.05 → 2.30 (+12.2%)`.
    pub fn describe(&self) -> String {
        format!(
            "{} {}: {:.2} → {:.2} ({:+.1}%)",
            self.universe, self.metric, self.previous, self.current, self.pct_change
        )
    }
}

fn pct(previous: f64, current: f64) -> f64 {
    if previous == 0.0 {
        return 0.0;
    }
    (current - previous) / previous * 100.0
}

/// Per-universe deltas for the metrics that matter to size/perf budgets.
pub fn compute_deltas(current: &PipelineReport, previous: &PipelineReport) -> Vec<Delta> {
    let mut deltas = Vec::new();
    for cur in &current.per_universe {
        let Some(prev) = previous.per_universe.iter().find(|u| u.key == cur.key) else {
            continue;
        };
        deltas.push(Delta {
            universe: cur.key.clone(),
            metric: "compression_ratio_zstd".into(),
            previous: prev.compression_ratio_zstd,
            current: cur.compression_ratio_zstd,
            pct_change: pct(prev.compression_ratio_zstd, cur.compression_ratio_zstd),
        });
        deltas.push(Delta {
            universe: cur.key.clone(),
            metric: "transfer_ms_uat_p50".into(),
            previous: prev.transfer_ms_uat_p50 as f64,
            current: cur.transfer_ms_uat_p50 as f64,
            pct_change: pct(
                prev.transfer_ms_uat_p50 as f64,
                cur.transfer_ms_uat_p50 as f64,
            ),
        });
    }
    deltas
}

/// Gate parameters.
pub struct GateConfig {
    /// Maximum report age before it is considered stale.
    pub max_age_hours: i64,
    /// Maximum tolerated regression, as a percentage (e.g. 20.0).
    pub max_regression_pct: f64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            max_age_hours: 24,
            max_regression_pct: 20.0,
        }
    }
}

/// Result of evaluating the deploy gate.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOutcome {
    pub passed: bool,
    /// One reason per failed check (empty when passed).
    pub reasons: Vec<String>,
}

/// Evaluate the deploy gate for `report` as of `now`, optionally comparing
/// against `baseline` to catch regressions.
///
/// A regression is a *worsening* beyond the tolerance:
/// - compression ratio dropping (less compression), or
/// - UAT transfer p50 rising (slower).
pub fn evaluate_gate(
    report: &PipelineReport,
    baseline: Option<&PipelineReport>,
    now: DateTime<Utc>,
    config: &GateConfig,
) -> GateOutcome {
    let mut reasons = Vec::new();

    // 1. Must be green.
    if !report.is_green() {
        reasons.push(format!("{} matrix cell(s) failed", report.errors.len()));
    }

    // 2. Must be fresh.
    match DateTime::parse_from_rfc3339(&report.generated_at) {
        Ok(generated) => {
            let age = now.signed_duration_since(generated.with_timezone(&Utc));
            if age.num_hours() > config.max_age_hours {
                reasons.push(format!(
                    "report is {}h old (max {}h)",
                    age.num_hours(),
                    config.max_age_hours
                ));
            }
        }
        Err(e) => reasons.push(format!("unparseable generated_at: {e}")),
    }

    // 3. Regressions within tolerance.
    if let Some(base) = baseline {
        for d in compute_deltas(report, base) {
            let regression_pct = match d.metric.as_str() {
                // Lower ratio = worse → regression is a negative pct_change.
                "compression_ratio_zstd" => -d.pct_change,
                // Higher transfer = worse → regression is a positive pct_change.
                "transfer_ms_uat_p50" => d.pct_change,
                _ => continue,
            };
            if regression_pct > config.max_regression_pct {
                reasons.push(format!(
                    "regression: {} ({:+.1}%, tolerance {:.0}%)",
                    d.describe(),
                    d.pct_change,
                    config.max_regression_pct
                ));
            }
        }
    }

    GateOutcome {
        passed: reasons.is_empty(),
        reasons,
    }
}

/// Load and parse a report YAML from disk.
pub fn load_report(path: &std::path::Path) -> Result<PipelineReport> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read report {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parse report {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{GlobalStats, UniverseStats};

    fn report(generated_at: &str, ratio: f64, transfer: u64, errors: usize) -> PipelineReport {
        PipelineReport {
            generated_at: generated_at.into(),
            binary_version: "test".into(),
            paths: vec!["local".into()],
            global: GlobalStats {
                total_universes: 1,
                total_files: 1,
                total_raw_bytes: 100,
                total_encoded_bytes: 50,
                compression_ratio_p50: 2.0,
                compression_ratio_p99: 2.0,
                encode_ns_p50: 0,
                encode_ns_p99: 0,
                decode_ns_p50: 0,
                decode_ns_p99: 0,
            },
            per_universe: vec![UniverseStats {
                key: "artelonga".into(),
                visibility: "public-subscribable".into(),
                files: 1,
                raw_bytes: 100,
                encoded_bytes_zstd: 50,
                encoded_bytes_private: 52,
                compression_ratio_zstd: ratio,
                transfer_ms_uat_p50: transfer,
                transfer_ms_prod_p50_read: 0,
            }],
            errors: (0..errors)
                .map(|i| crate::report::CellError {
                    path: "uat".into(),
                    universe: "rfq".into(),
                    combo: "private".into(),
                    detail: format!("boom {i}"),
                })
                .collect(),
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-27T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn fresh_green_report_passes() {
        let r = report("2026-04-27T11:00:00Z", 2.0, 30, 0);
        let out = evaluate_gate(&r, None, now(), &GateConfig::default());
        assert!(out.passed, "{:?}", out.reasons);
    }

    #[test]
    fn failing_cells_block() {
        let r = report("2026-04-27T11:00:00Z", 2.0, 30, 2);
        let out = evaluate_gate(&r, None, now(), &GateConfig::default());
        assert!(!out.passed);
        assert!(out.reasons[0].contains("failed"));
    }

    #[test]
    fn stale_report_blocks() {
        let r = report("2026-04-25T11:00:00Z", 2.0, 30, 0); // ~49h old
        let out = evaluate_gate(&r, None, now(), &GateConfig::default());
        assert!(!out.passed);
        assert!(out.reasons.iter().any(|x| x.contains("old")));
    }

    #[test]
    fn transfer_regression_beyond_tolerance_blocks() {
        let baseline = report("2026-04-26T11:00:00Z", 2.0, 30, 0);
        let current = report("2026-04-27T11:00:00Z", 2.0, 40, 0); // +33% transfer
        let out = evaluate_gate(&current, Some(&baseline), now(), &GateConfig::default());
        assert!(!out.passed);
        assert!(out.reasons.iter().any(|x| x.contains("regression")));
    }

    #[test]
    fn compression_regression_blocks() {
        let baseline = report("2026-04-26T11:00:00Z", 2.5, 30, 0);
        let current = report("2026-04-27T11:00:00Z", 1.8, 30, 0); // -28% ratio
        let out = evaluate_gate(&current, Some(&baseline), now(), &GateConfig::default());
        assert!(!out.passed);
    }

    #[test]
    fn small_movement_within_tolerance_passes() {
        let baseline = report("2026-04-26T11:00:00Z", 2.0, 30, 0);
        let current = report("2026-04-27T11:00:00Z", 1.95, 33, 0); // -2.5% ratio, +10% transfer
        let out = evaluate_gate(&current, Some(&baseline), now(), &GateConfig::default());
        assert!(out.passed, "{:?}", out.reasons);
    }

    #[test]
    fn deltas_describe_movement() {
        let baseline = report("2026-04-26T11:00:00Z", 2.0, 30, 0);
        let current = report("2026-04-27T11:00:00Z", 2.3, 30, 0);
        let deltas = compute_deltas(&current, &baseline);
        let ratio_delta = deltas
            .iter()
            .find(|d| d.metric == "compression_ratio_zstd")
            .unwrap();
        assert!((ratio_delta.pct_change - 15.0).abs() < 0.1);
        assert!(ratio_delta.describe().contains("artelonga"));
    }
}
