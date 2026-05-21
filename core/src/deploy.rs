//! Deploy manifest — `deploy.yaml`
//!
//! A universe carries `deploy.yaml` at its root to declare how it should be
//! deployed to a target platform. This module defines the typed `DeployManifest`
//! struct, parsing, and semantic validation.
//!
//! Deployer adapters (CO-134 static-on-R2, CO-135 Cloudflare Pages) consume
//! a parsed `DeployManifest` rather than freeform YAML.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Filename for the deploy manifest inside a universe root.
pub const DEPLOY_FILENAME: &str = "deploy.yaml";
/// Only this schema version is supported; `version:` must equal this value.
pub const SUPPORTED_VERSION: u32 = 1;

// ─── Error ────────────────────────────────────────────────────────────────────

/// Errors from deploy manifest parsing or semantic validation.
#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("cannot read {file}: {source}")]
    Io {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}: YAML parse error: {message}")]
    YamlParse { file: PathBuf, message: String },
    #[error("{file}: field '{path}': {message}")]
    InvalidField {
        file: PathBuf,
        path: String,
        message: String,
    },
}

// ─── Manifest struct hierarchy ────────────────────────────────────────────────

/// Root deploy manifest (`deploy.yaml`).
///
/// Place this file at the universe root to declare the deployment target and
/// options. All fields except `version` and `target` are optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeployManifest {
    /// Schema version. Must be `1`.
    pub version: u32,
    /// Target platform for this deployment.
    pub target: DeployTarget,
    /// Custom domain. If absent the deployer assigns a default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Runtime configuration (required for non-static targets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeConfig>,
    /// Service bindings (storage, secrets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Bindings>,
    /// Auto-scaling parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaling: Option<Scaling>,
    /// Observability sink and sampling rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<Telemetry>,
    /// Automated backup policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<Backup>,
}

/// Deployment target platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeployTarget {
    StaticOnR2,
    CloudflarePages,
    Fly,
    Vercel,
    Fargate,
}

/// Runtime configuration for the deployed service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeConfig {
    /// Runtime kind; for static targets use `static`.
    pub kind: RuntimeKind,
    /// Build step configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildConfig>,
}

/// Kind of runtime environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    Static,
    Node,
    Rust,
    Python,
    Wasm,
}

/// Build step for the deployment pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BuildConfig {
    /// Shell command to build the universe (default: `co build`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Directory containing build output (default: `dist/`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Service bindings: object storage and named secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bindings {
    /// Object storage binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageBinding>,
    /// Secret names (values are resolved per-target from CO secrets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
}

/// Object storage binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StorageBinding {
    /// Storage backend type.
    #[serde(rename = "type")]
    pub storage_type: StorageType,
    /// Bucket name.
    pub bucket: String,
    /// Whether the bucket contents are encrypted at rest.
    #[serde(default)]
    pub encrypted: bool,
}

/// Storage backend type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageType {
    R2,
    S3,
    Gcs,
}

/// Auto-scaling bounds for the deployed service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scaling {
    /// Minimum replica count (>= 0). Default: 0.
    #[serde(default)]
    pub min: u32,
    /// Maximum replica count (>= min).
    pub max: u32,
}

/// Observability configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Telemetry {
    /// Where telemetry events are sent.
    pub sink: TelemetrySink,
    /// Fraction of events to sample (0.0 – 1.0). Default: 1.0.
    #[serde(default = "default_sampling")]
    pub sampling: f64,
}

fn default_sampling() -> f64 {
    1.0
}

/// Telemetry sink destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TelemetrySink {
    CoCentral,
    None,
}

/// Automated backup policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Backup {
    /// How often snapshots are taken.
    pub schedule: BackupSchedule,
    /// How long snapshots are kept (e.g. `"30d"`, `"24h"`, `"1w"`).
    pub retention: String,
}

/// Backup frequency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BackupSchedule {
    Hourly,
    Daily,
    Weekly,
    None,
}

// ─── Parsing ──────────────────────────────────────────────────────────────────

/// Parse and validate `deploy.yaml` from disk.
///
/// Errors carry the file path, and YAML parse errors include the line number
/// when the underlying parser can provide it.
pub fn parse_file(path: &Path) -> Result<DeployManifest, DeployError> {
    let content = std::fs::read_to_string(path).map_err(|e| DeployError::Io {
        file: path.to_path_buf(),
        source: e,
    })?;
    parse_str(&content, path)
}

/// Parse and validate a `deploy.yaml` from a string.
///
/// `file` is used only for error messages.
pub fn parse_str(s: &str, file: &Path) -> Result<DeployManifest, DeployError> {
    let manifest: DeployManifest = serde_yaml::from_str(s).map_err(|e| DeployError::YamlParse {
        file: file.to_path_buf(),
        message: if let Some(loc) = e.location() {
            format!(
                "line {}, column {}: {}",
                loc.line(),
                loc.column(),
                strip_location_prefix(&e.to_string())
            )
        } else {
            e.to_string()
        },
    })?;
    validate(&manifest, file)?;
    Ok(manifest)
}

/// Strip the redundant "at line N column M" suffix that serde_yaml adds to its
/// Display output when we already include location separately.
fn strip_location_prefix(msg: &str) -> &str {
    msg.split(" at line ").next().unwrap_or(msg)
}

// ─── Semantic validation ──────────────────────────────────────────────────────

fn validate(manifest: &DeployManifest, file: &Path) -> Result<(), DeployError> {
    if manifest.version != SUPPORTED_VERSION {
        return Err(DeployError::InvalidField {
            file: file.to_path_buf(),
            path: "version".to_string(),
            message: format!("must be {} (got {})", SUPPORTED_VERSION, manifest.version),
        });
    }

    if let Some(scaling) = &manifest.scaling
        && scaling.max < scaling.min
    {
        return Err(DeployError::InvalidField {
            file: file.to_path_buf(),
            path: "scaling.max".to_string(),
            message: format!(
                "must be >= scaling.min ({}) but got {}",
                scaling.min, scaling.max
            ),
        });
    }

    if let Some(telemetry) = &manifest.telemetry
        && !(0.0..=1.0).contains(&telemetry.sampling)
    {
        return Err(DeployError::InvalidField {
            file: file.to_path_buf(),
            path: "telemetry.sampling".to_string(),
            message: format!("must be between 0.0 and 1.0 (got {})", telemetry.sampling),
        });
    }

    if let Some(backup) = &manifest.backup
        && !is_valid_duration(&backup.retention)
    {
        return Err(DeployError::InvalidField {
            file: file.to_path_buf(),
            path: "backup.retention".to_string(),
            message: format!(
                "invalid duration '{}'; expected a number followed by d/h/w/m (e.g. '30d', '24h', '1w')",
                backup.retention
            ),
        });
    }

    Ok(())
}

/// Validate a duration string: one or more digits followed by `d`, `h`, `w`, or `m`.
fn is_valid_duration(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let (digits, unit) = s.split_at(s.len() - 1);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(unit, "d" | "h" | "w" | "m")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fake_path() -> PathBuf {
        PathBuf::from("deploy.yaml")
    }

    fn fixture(rel: &str) -> PathBuf {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        PathBuf::from(manifest_dir)
            .parent()
            .unwrap()
            .join("tests/fixtures/deploy")
            .join(rel)
    }

    // ── Inline round-trip tests ──────────────────────────────────────────────

    #[test]
    fn test_minimal_valid_manifest() {
        let yaml = "version: 1\ntarget: fly\n";
        let m = parse_str(yaml, &fake_path()).expect("minimal manifest must parse");
        assert_eq!(m.version, 1);
        assert_eq!(m.target, DeployTarget::Fly);
        assert!(m.domain.is_none());
    }

    #[test]
    fn test_round_trip_stable() {
        let yaml = "version: 1\ntarget: cloudflare-pages\ndomain: example.co.app\n";
        let m = parse_str(yaml, &fake_path()).unwrap();
        let back = serde_yaml::to_string(&m).unwrap();
        let m2 = parse_str(&back, &fake_path()).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_full_manifest_round_trip() {
        let yaml = r#"version: 1
target: fly
domain: my.example.com
runtime:
  kind: static
  build:
    command: co build
    output: dist/
bindings:
  storage:
    type: r2
    bucket: my-bucket
    encrypted: true
  secrets:
    - STRIPE_KEY
scaling:
  min: 0
  max: 10
telemetry:
  sink: co-central
  sampling: 0.5
backup:
  schedule: daily
  retention: 30d
"#;
        let m = parse_str(yaml, &fake_path()).unwrap();
        let serialized = serde_yaml::to_string(&m).unwrap();
        let m2 = parse_str(&serialized, &fake_path()).unwrap();
        assert_eq!(m, m2);
    }

    // ── Semantic validation tests ────────────────────────────────────────────

    #[test]
    fn test_version_must_be_1() {
        let yaml = "version: 2\ntarget: fly\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "version"),
            "expected InvalidField at version, got: {err}"
        );
    }

    #[test]
    fn test_scaling_max_less_than_min_rejected() {
        let yaml = "version: 1\ntarget: fly\nscaling:\n  min: 50\n  max: 10\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "scaling.max"),
            "expected InvalidField at scaling.max, got: {err}"
        );
    }

    #[test]
    fn test_telemetry_sampling_above_1_rejected() {
        let yaml = "version: 1\ntarget: fly\ntelemetry:\n  sink: co-central\n  sampling: 1.5\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "telemetry.sampling"),
            "expected InvalidField at telemetry.sampling, got: {err}"
        );
    }

    #[test]
    fn test_telemetry_sampling_below_0_rejected() {
        let yaml = "version: 1\ntarget: fly\ntelemetry:\n  sink: co-central\n  sampling: -0.1\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "telemetry.sampling"),
        );
    }

    #[test]
    fn test_invalid_duration_rejected() {
        let yaml = "version: 1\ntarget: fly\nbackup:\n  schedule: daily\n  retention: 30days\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "backup.retention"),
            "expected InvalidField at backup.retention, got: {err}"
        );
    }

    #[test]
    fn test_valid_duration_strings() {
        assert!(is_valid_duration("30d"));
        assert!(is_valid_duration("24h"));
        assert!(is_valid_duration("1w"));
        assert!(is_valid_duration("60m"));
        assert!(is_valid_duration("7d"));
        assert!(!is_valid_duration(""));
        assert!(!is_valid_duration("d"));
        assert!(!is_valid_duration("30days"));
        assert!(!is_valid_duration("30"));
        assert!(!is_valid_duration("1y"));
    }

    #[test]
    fn test_missing_version_is_yaml_error() {
        let yaml = "target: fly\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(err, DeployError::YamlParse { .. }),
            "missing required field should be a YAML parse error"
        );
    }

    #[test]
    fn test_missing_target_is_yaml_error() {
        let yaml = "version: 1\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(err, DeployError::YamlParse { .. }),
            "missing required field should be a YAML parse error"
        );
    }

    #[test]
    fn test_unknown_target_is_yaml_error() {
        let yaml = "version: 1\ntarget: unknown-platform\n";
        let err = parse_str(yaml, &fake_path()).unwrap_err();
        assert!(
            matches!(err, DeployError::YamlParse { .. }),
            "unknown enum variant should be a YAML parse error"
        );
    }

    #[test]
    fn test_scaling_equal_min_max_valid() {
        let yaml = "version: 1\ntarget: fly\nscaling:\n  min: 5\n  max: 5\n";
        assert!(parse_str(yaml, &fake_path()).is_ok());
    }

    #[test]
    fn test_telemetry_sampling_boundaries_valid() {
        let yaml0 = "version: 1\ntarget: fly\ntelemetry:\n  sink: none\n  sampling: 0.0\n";
        let yaml1 = "version: 1\ntarget: fly\ntelemetry:\n  sink: co-central\n  sampling: 1.0\n";
        assert!(parse_str(yaml0, &fake_path()).is_ok());
        assert!(parse_str(yaml1, &fake_path()).is_ok());
    }

    // ── Fixture-based tests ──────────────────────────────────────────────────

    #[test]
    fn test_valid_fixture_static_on_r2() {
        let path = fixture("valid/static-on-r2.yaml");
        assert!(
            parse_file(&path).is_ok(),
            "valid/static-on-r2.yaml must parse successfully"
        );
    }

    #[test]
    fn test_valid_fixture_cloudflare_pages() {
        let path = fixture("valid/cloudflare-pages.yaml");
        assert!(
            parse_file(&path).is_ok(),
            "valid/cloudflare-pages.yaml must parse successfully"
        );
    }

    #[test]
    fn test_valid_fixture_fly() {
        let path = fixture("valid/fly.yaml");
        assert!(
            parse_file(&path).is_ok(),
            "valid/fly.yaml must parse successfully"
        );
    }

    #[test]
    fn test_valid_fixture_vercel() {
        let path = fixture("valid/vercel.yaml");
        assert!(
            parse_file(&path).is_ok(),
            "valid/vercel.yaml must parse successfully"
        );
    }

    #[test]
    fn test_valid_fixture_with_all_options() {
        let path = fixture("valid/with-all-options.yaml");
        assert!(
            parse_file(&path).is_ok(),
            "valid/with-all-options.yaml must parse successfully"
        );
    }

    #[test]
    fn test_invalid_fixture_missing_version() {
        let path = fixture("invalid/missing-version.yaml");
        assert!(
            parse_file(&path).is_err(),
            "invalid/missing-version.yaml must fail validation"
        );
    }

    #[test]
    fn test_invalid_fixture_invalid_version() {
        let path = fixture("invalid/invalid-version.yaml");
        let err = parse_file(&path).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "version"),
            "invalid-version.yaml should fail at version field, got: {err}"
        );
    }

    #[test]
    fn test_invalid_fixture_missing_target() {
        let path = fixture("invalid/missing-target.yaml");
        assert!(
            parse_file(&path).is_err(),
            "invalid/missing-target.yaml must fail validation"
        );
    }

    #[test]
    fn test_invalid_fixture_invalid_target() {
        let path = fixture("invalid/invalid-target.yaml");
        assert!(
            parse_file(&path).is_err(),
            "invalid/invalid-target.yaml must fail validation"
        );
    }

    #[test]
    fn test_invalid_fixture_scaling_max_less_than_min() {
        let path = fixture("invalid/scaling-max-less-than-min.yaml");
        let err = parse_file(&path).unwrap_err();
        assert!(
            matches!(&err, DeployError::InvalidField { path, .. } if path == "scaling.max"),
            "scaling-max-less-than-min.yaml should fail at scaling.max, got: {err}"
        );
    }
}
