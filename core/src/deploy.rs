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

// ─── Adapter supporting types ─────────────────────────────────────────────────

use std::sync::Arc;

/// A built universe tarball ready for deployment.
#[derive(Debug, Clone)]
pub struct BuildArtifact {
    /// Universe identifier (slug or UUID).
    pub universe_id: String,
    /// Path to the `.tar.gz` archive of built output.
    pub tarball: PathBuf,
    /// Uncompressed size in bytes.
    pub size_bytes: u64,
    /// SHA-256 hex digest of the tarball.
    pub checksum: String,
}

/// Outcome of a successful deployment.
#[derive(Debug, Clone)]
pub struct DeployResult {
    /// Stable identifier: `{universe_id}/{timestamp}-{suffix}`.
    pub deploy_id: String,
    /// Public URL where the universe is served.
    pub url: String,
    /// Content-addressable snapshot identifier.
    pub snapshot_id: String,
}

/// Observed state of a specific deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployStatus {
    /// This deployment is live and serving traffic.
    Active { deploy_id: String, url: String },
    /// This deployment exists but is not currently active.
    Inactive { deploy_id: String },
    /// Deployment ID not found or state is unknown.
    Unknown,
}

/// Errors produced by a deployer adapter.
#[derive(Debug, thiserror::Error)]
pub enum DeployAdapterError {
    #[error("upload failed: {0}")]
    Upload(String),
    #[error("deployment not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for DeployAdapterError {
    fn from(e: std::io::Error) -> Self {
        DeployAdapterError::Io(e.to_string())
    }
}

/// Resolves named secrets for a deployment.
pub trait SecretResolver: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
}

/// Resolves secrets from environment variables.
pub struct EnvSecretResolver;

impl SecretResolver for EnvSecretResolver {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

// ─── DeployerAdapter trait ────────────────────────────────────────────────────

/// Pluggable deployment backend.
///
/// Each implementation handles upload, pointer-swap, rollback, and status for a
/// specific hosting platform. CO-134 ships the first implementation: R2 static.
#[async_trait::async_trait]
pub trait DeployerAdapter: Send + Sync {
    /// Short identifier for this adapter (e.g. `"static-on-r2"`).
    fn name(&self) -> &'static str;

    /// Publish the artifact, returning a stable deploy ID and public URL.
    async fn deploy(
        &self,
        artifact: BuildArtifact,
        manifest: &DeployManifest,
        secrets: &dyn SecretResolver,
    ) -> Result<DeployResult, DeployAdapterError>;

    /// Restore a previous deployment by ID.
    async fn rollback(&self, deploy_id: &str) -> Result<(), DeployAdapterError>;

    /// Query the current state of a deployment.
    async fn status(&self, deploy_id: &str) -> DeployStatus;
}

// ─── S3Backend abstraction ────────────────────────────────────────────────────

/// Low-level S3-compatible object-store operations.
///
/// Abstracted so unit tests can inject a mock without a live bucket.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait S3Backend: Send + Sync {
    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), DeployAdapterError>;

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, DeployAdapterError>;

    async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<Vec<String>, DeployAdapterError>;

    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<String>, DeployAdapterError>;
}

// ─── AwsS3Backend ─────────────────────────────────────────────────────────────

/// `S3Backend` backed by the AWS SDK (also works against Cloudflare R2 via S3 compat).
#[cfg(feature = "deploy-r2")]
pub struct AwsS3Backend {
    client: aws_sdk_s3::Client,
}

#[cfg(feature = "deploy-r2")]
impl AwsS3Backend {
    pub fn new(client: aws_sdk_s3::Client) -> Self {
        Self { client }
    }
}

#[cfg(feature = "deploy-r2")]
#[async_trait::async_trait]
impl S3Backend for AwsS3Backend {
    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), DeployAdapterError> {
        self.client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(aws_sdk_s3::primitives::ByteStream::from(body))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| DeployAdapterError::Upload(e.to_string()))?;
        Ok(())
    }

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, DeployAdapterError> {
        use aws_sdk_s3::error::SdkError;
        match self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(resp) => {
                let bytes = resp
                    .body
                    .collect()
                    .await
                    .map_err(|e| DeployAdapterError::Other(e.to_string()))?
                    .into_bytes();
                Ok(Some(bytes.to_vec()))
            }
            Err(SdkError::ServiceError(svc)) if svc.err().is_no_such_key() => Ok(None),
            Err(e) => Err(DeployAdapterError::Upload(e.to_string())),
        }
    }

    async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<Vec<String>, DeployAdapterError> {
        let resp = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .send()
            .await
            .map_err(|e| DeployAdapterError::Upload(e.to_string()))?;
        let keys = resp
            .contents()
            .iter()
            .filter_map(|obj| obj.key().map(str::to_string))
            .collect();
        Ok(keys)
    }

    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<String>, DeployAdapterError> {
        use aws_sdk_s3::error::SdkError;
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(resp) => Ok(resp.e_tag().map(str::to_string)),
            Err(SdkError::ServiceError(svc)) if svc.err().is_not_found() => Ok(None),
            Err(e) => Err(DeployAdapterError::Upload(e.to_string())),
        }
    }
}

// ─── Disabled S3 backend (stub when deploy-r2 feature is off) ─────────────────

#[cfg(not(feature = "deploy-r2"))]
struct DisabledS3Backend;

#[cfg(not(feature = "deploy-r2"))]
#[async_trait::async_trait]
impl S3Backend for DisabledS3Backend {
    async fn put_object(
        &self,
        _: &str,
        _: &str,
        _: Vec<u8>,
        _: &str,
    ) -> Result<(), DeployAdapterError> {
        Err(DeployAdapterError::Other(
            "R2 deployer disabled — rebuild with --features deploy-r2".into(),
        ))
    }
    async fn get_object(&self, _: &str, _: &str) -> Result<Option<Vec<u8>>, DeployAdapterError> {
        Err(DeployAdapterError::Other(
            "R2 deployer disabled — rebuild with --features deploy-r2".into(),
        ))
    }
    async fn list_objects(&self, _: &str, _: &str) -> Result<Vec<String>, DeployAdapterError> {
        Err(DeployAdapterError::Other(
            "R2 deployer disabled — rebuild with --features deploy-r2".into(),
        ))
    }
    async fn head_object(&self, _: &str, _: &str) -> Result<Option<String>, DeployAdapterError> {
        Err(DeployAdapterError::Other(
            "R2 deployer disabled — rebuild with --features deploy-r2".into(),
        ))
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Map file extension to HTTP content-type.
fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("pdf") => "application/pdf",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Extract a `.tar.gz` archive into `dest`.
fn extract_tarball(tarball: &Path, dest: &Path) -> Result<(), DeployAdapterError> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    let file = std::fs::File::open(tarball).map_err(|e| DeployAdapterError::Io(e.to_string()))?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    archive
        .unpack(dest)
        .map_err(|e| DeployAdapterError::Io(e.to_string()))?;
    Ok(())
}

/// Pack a directory into a `.tar.gz` archive at `dest`.
pub fn create_tarball_from_dir(src: &Path, dest: &Path) -> Result<(), DeployAdapterError> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;
    let file = std::fs::File::create(dest).map_err(|e| DeployAdapterError::Io(e.to_string()))?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(enc);
    builder
        .append_dir_all(".", src)
        .map_err(|e| DeployAdapterError::Io(e.to_string()))?;
    builder
        .finish()
        .map_err(|e| DeployAdapterError::Io(e.to_string()))?;
    Ok(())
}

// ─── StaticOnR2Adapter ────────────────────────────────────────────────────────

const R2_DEPLOYS_PREFIX: &str = "co-deployments";
const R2_CURRENT_PREFIX: &str = "co-deployments-current";

/// Uploads a static universe to Cloudflare R2 (S3-compatible).
///
/// Deploy layout in the bucket:
/// ```text
/// co-deployments/{deploy_id}/index.html
/// co-deployments/{deploy_id}/style.css
/// co-deployments-current/{universe_id}/current  → deploy_id (text body)
/// ```
pub struct StaticOnR2Adapter {
    s3: Arc<dyn S3Backend>,
    bucket: String,
    base_domain: String,
}

impl StaticOnR2Adapter {
    /// Create from any `S3Backend` implementation.
    pub fn new(
        s3: impl S3Backend + 'static,
        bucket: impl Into<String>,
        base_domain: impl Into<String>,
    ) -> Self {
        Self {
            s3: Arc::new(s3),
            bucket: bucket.into(),
            base_domain: base_domain.into(),
        }
    }

    /// Create from a shared `Arc<dyn S3Backend>` (useful for stateful test backends).
    pub fn with_shared_backend(
        s3: Arc<dyn S3Backend>,
        bucket: impl Into<String>,
        base_domain: impl Into<String>,
    ) -> Self {
        Self {
            s3,
            bucket: bucket.into(),
            base_domain: base_domain.into(),
        }
    }

    /// Build an adapter from R2 credentials (requires `deploy-r2` feature).
    ///
    /// Without the feature a disabled stub adapter is returned; all deploy/rollback
    /// calls on it will fail with "R2 deployer disabled".
    #[cfg(feature = "deploy-r2")]
    pub async fn from_credentials(
        account_id: &str,
        access_key_id: &str,
        secret_access_key: &str,
        bucket: impl Into<String>,
        base_domain: impl Into<String>,
    ) -> Self {
        use aws_sdk_s3::config::{Credentials, Region};
        let creds = Credentials::new(access_key_id, secret_access_key, None, None, "r2");
        let endpoint = format!("https://{account_id}.r2.cloudflarestorage.com");
        let config = aws_sdk_s3::config::Builder::new()
            .credentials_provider(creds)
            .endpoint_url(&endpoint)
            .region(Region::new("auto"))
            .force_path_style(true)
            .build();
        let client = aws_sdk_s3::Client::from_conf(config);
        Self::new(AwsS3Backend::new(client), bucket, base_domain)
    }

    #[cfg(not(feature = "deploy-r2"))]
    pub async fn from_credentials(
        _account_id: &str,
        _access_key_id: &str,
        _secret_access_key: &str,
        bucket: impl Into<String>,
        base_domain: impl Into<String>,
    ) -> Self {
        Self::new(DisabledS3Backend, bucket, base_domain)
    }

    /// Deploy directly from a build output directory (creates the tarball internally).
    pub async fn deploy_from_dir(
        &self,
        dir: &Path,
        universe_id: &str,
        manifest: &DeployManifest,
        secrets: &dyn SecretResolver,
    ) -> Result<DeployResult, DeployAdapterError> {
        let tmp = tempfile::tempdir().map_err(|e| DeployAdapterError::Io(e.to_string()))?;
        let tarball_path = tmp.path().join("artifact.tar.gz");
        create_tarball_from_dir(dir, &tarball_path)?;
        let artifact = BuildArtifact {
            universe_id: universe_id.to_string(),
            tarball: tarball_path,
            size_bytes: 0,
            checksum: String::new(),
        };
        self.deploy(artifact, manifest, secrets).await
    }

    fn pointer_key(universe_id: &str) -> String {
        format!("{R2_CURRENT_PREFIX}/{universe_id}/current")
    }

    fn deploy_url(&self, deploy_id: &str) -> String {
        format!(
            "https://{}/{R2_DEPLOYS_PREFIX}/{deploy_id}/",
            self.base_domain
        )
    }

    fn universe_id_from_deploy_id(deploy_id: &str) -> &str {
        deploy_id.split('/').next().unwrap_or(deploy_id)
    }
}

#[async_trait::async_trait]
impl DeployerAdapter for StaticOnR2Adapter {
    fn name(&self) -> &'static str {
        "static-on-r2"
    }

    async fn deploy(
        &self,
        artifact: BuildArtifact,
        _manifest: &DeployManifest,
        _secrets: &dyn SecretResolver,
    ) -> Result<DeployResult, DeployAdapterError> {
        use sha2::{Digest, Sha256};
        use uuid::Uuid;

        // 1. Generate a unique deploy ID
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let suffix = &Uuid::new_v4().to_string()[..8];
        let deploy_id = format!("{}/{ts}-{suffix}", artifact.universe_id);

        // 2. Extract tarball to a temp dir
        let tmp = tempfile::tempdir().map_err(|e| DeployAdapterError::Io(e.to_string()))?;
        extract_tarball(&artifact.tarball, tmp.path())?;

        // 3. Upload all files under the versioned prefix
        for entry in walkdir::WalkDir::new(tmp.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let abs = entry.path();
            let rel = abs
                .strip_prefix(tmp.path())
                .map_err(|e| DeployAdapterError::Other(e.to_string()))?;
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let key = format!("{R2_DEPLOYS_PREFIX}/{deploy_id}/{rel_str}");
            let body = std::fs::read(abs).map_err(|e| DeployAdapterError::Io(e.to_string()))?;
            let ct = content_type_for_path(rel);
            self.s3.put_object(&self.bucket, &key, body, ct).await?;
        }

        // 4. Update the "current" pointer
        let pointer_key = Self::pointer_key(&artifact.universe_id);
        self.s3
            .put_object(
                &self.bucket,
                &pointer_key,
                deploy_id.as_bytes().to_vec(),
                "text/plain",
            )
            .await?;

        // 5. Compute snapshot_id from deploy_id
        let mut hasher = Sha256::new();
        hasher.update(deploy_id.as_bytes());
        let snapshot_id = format!("{:x}", hasher.finalize());

        Ok(DeployResult {
            url: self.deploy_url(&deploy_id),
            deploy_id,
            snapshot_id,
        })
    }

    async fn rollback(&self, deploy_id: &str) -> Result<(), DeployAdapterError> {
        let universe_id = Self::universe_id_from_deploy_id(deploy_id);

        // Verify the deployment has objects in the bucket
        let prefix = format!("{R2_DEPLOYS_PREFIX}/{deploy_id}/");
        let keys = self.s3.list_objects(&self.bucket, &prefix).await?;
        if keys.is_empty() {
            return Err(DeployAdapterError::NotFound(format!(
                "deployment '{deploy_id}' has no objects in bucket"
            )));
        }

        // Rewrite the pointer to point at the target deploy
        let pointer_key = Self::pointer_key(universe_id);
        self.s3
            .put_object(
                &self.bucket,
                &pointer_key,
                deploy_id.as_bytes().to_vec(),
                "text/plain",
            )
            .await?;

        Ok(())
    }

    async fn status(&self, deploy_id: &str) -> DeployStatus {
        let universe_id = Self::universe_id_from_deploy_id(deploy_id);
        let pointer_key = Self::pointer_key(universe_id);

        match self.s3.get_object(&self.bucket, &pointer_key).await {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(current_id) if current_id.trim() == deploy_id => DeployStatus::Active {
                    deploy_id: deploy_id.to_string(),
                    url: self.deploy_url(deploy_id),
                },
                Ok(_) => DeployStatus::Inactive {
                    deploy_id: deploy_id.to_string(),
                },
                Err(_) => DeployStatus::Unknown,
            },
            Ok(None) | Err(_) => DeployStatus::Unknown,
        }
    }
}

// ─── Adapter tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use mockall::predicate::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    // ── In-memory S3 backend for stateful tests ───────────────────────────────

    struct InMemoryS3 {
        objects: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl InMemoryS3 {
        fn new() -> Self {
            Self {
                objects: Mutex::new(HashMap::new()),
            }
        }

        fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
            self.objects.lock().unwrap().get(key).cloned()
        }
    }

    #[async_trait::async_trait]
    impl S3Backend for InMemoryS3 {
        async fn put_object(
            &self,
            _bucket: &str,
            key: &str,
            body: Vec<u8>,
            _ct: &str,
        ) -> Result<(), DeployAdapterError> {
            self.objects.lock().unwrap().insert(key.to_string(), body);
            Ok(())
        }

        async fn get_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<Vec<u8>>, DeployAdapterError> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }

        async fn list_objects(
            &self,
            _bucket: &str,
            prefix: &str,
        ) -> Result<Vec<String>, DeployAdapterError> {
            let objects = self.objects.lock().unwrap();
            Ok(objects
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }

        async fn head_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<String>, DeployAdapterError> {
            let objects = self.objects.lock().unwrap();
            Ok(objects.get(key).map(|_| "\"test-etag\"".to_string()))
        }
    }

    // ── Fixture helpers ───────────────────────────────────────────────────────

    fn make_manifest() -> DeployManifest {
        parse_str(
            "version: 1\ntarget: static-on-r2\n",
            &PathBuf::from("deploy.yaml"),
        )
        .unwrap()
    }

    fn make_artifact(tarball: PathBuf, universe_id: &str) -> BuildArtifact {
        BuildArtifact {
            universe_id: universe_id.to_string(),
            tarball,
            size_bytes: 0,
            checksum: "test-checksum".to_string(),
        }
    }

    fn make_universe_dir(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("index.html"), b"<h1>Hello</h1>").unwrap();
        std::fs::write(dir.join("style.css"), b"body { color: red; }").unwrap();
        std::fs::write(dir.join("app.js"), b"console.log('hi')").unwrap();
    }

    fn make_tarball(src: &Path, out: &Path) {
        create_tarball_from_dir(src, out).unwrap();
    }

    // ── Unit tests (MockS3Backend via mockall) ────────────────────────────────

    #[tokio::test]
    async fn test_deploy_uploads_files_and_pointer() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("universe");
        make_universe_dir(&src);
        let tarball = tmp.path().join("artifact.tar.gz");
        make_tarball(&src, &tarball);

        let mut mock = MockS3Backend::new();
        // 3 file uploads + 1 pointer write = 4 put_object calls
        mock.expect_put_object()
            .times(4)
            .returning(|_, _, _, _| Ok(()));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        let result = adapter
            .deploy(
                make_artifact(tarball, "u_test"),
                &make_manifest(),
                &EnvSecretResolver,
            )
            .await
            .unwrap();

        assert!(result.deploy_id.starts_with("u_test/"));
        assert!(result.url.contains("co.app"));
        assert!(result.url.contains("co-deployments"));
        assert!(!result.snapshot_id.is_empty());
        assert_eq!(result.snapshot_id.len(), 64); // sha256 hex
    }

    #[tokio::test]
    async fn test_name_returns_static_on_r2() {
        let mock = MockS3Backend::new();
        let adapter = StaticOnR2Adapter::new(mock, "b", "d");
        assert_eq!(adapter.name(), "static-on-r2");
    }

    #[tokio::test]
    async fn test_status_active_when_pointer_matches() {
        let mut mock = MockS3Backend::new();
        mock.expect_get_object()
            .with(eq("my-bucket"), eq("co-deployments-current/u_test/current"))
            .returning(|_, _| Ok(Some(b"u_test/20240101T000000Z-abc12345".to_vec())));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        let status = adapter.status("u_test/20240101T000000Z-abc12345").await;
        assert!(matches!(status, DeployStatus::Active { .. }));
    }

    #[tokio::test]
    async fn test_status_inactive_when_pointer_differs() {
        let mut mock = MockS3Backend::new();
        mock.expect_get_object()
            .with(eq("my-bucket"), eq("co-deployments-current/u_test/current"))
            .returning(|_, _| Ok(Some(b"u_test/20240102T000000Z-xyz99999".to_vec())));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        let status = adapter.status("u_test/20240101T000000Z-abc12345").await;
        assert!(matches!(status, DeployStatus::Inactive { .. }));
    }

    #[tokio::test]
    async fn test_status_unknown_when_no_pointer() {
        let mut mock = MockS3Backend::new();
        mock.expect_get_object().returning(|_, _| Ok(None));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        let status = adapter.status("u_test/20240101T000000Z-abc12345").await;
        assert_eq!(status, DeployStatus::Unknown);
    }

    #[tokio::test]
    async fn test_rollback_updates_pointer() {
        let mut mock = MockS3Backend::new();
        mock.expect_list_objects()
            .with(
                eq("my-bucket"),
                eq("co-deployments/u_test/20240101T000000Z-abc12345/"),
            )
            .returning(|_, _| {
                Ok(vec![
                    "co-deployments/u_test/20240101T000000Z-abc12345/index.html".to_string(),
                ])
            });
        mock.expect_put_object()
            .with(
                eq("my-bucket"),
                eq("co-deployments-current/u_test/current"),
                eq(b"u_test/20240101T000000Z-abc12345".to_vec()),
                eq("text/plain"),
            )
            .returning(|_, _, _, _| Ok(()));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        adapter
            .rollback("u_test/20240101T000000Z-abc12345")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_rollback_fails_when_deploy_not_found() {
        let mut mock = MockS3Backend::new();
        mock.expect_list_objects().returning(|_, _| Ok(vec![]));

        let adapter = StaticOnR2Adapter::new(mock, "my-bucket", "co.app");
        let err = adapter.rollback("u_test/nonexistent").await.unwrap_err();
        assert!(matches!(err, DeployAdapterError::NotFound(_)));
    }

    // ── Stateful rollback scenario (InMemoryS3) ───────────────────────────────

    #[tokio::test]
    async fn test_deploy_v1_deploy_v2_rollback_to_v1() {
        let s3 = Arc::new(InMemoryS3::new());
        let adapter = StaticOnR2Adapter::with_shared_backend(
            s3.clone() as Arc<dyn S3Backend>,
            "my-bucket",
            "co.app",
        );
        let manifest = make_manifest();

        // v1: universe with a landing page
        let tmp = tempdir().unwrap();
        let src_v1 = tmp.path().join("v1");
        make_universe_dir(&src_v1);
        let tarball_v1 = tmp.path().join("v1.tar.gz");
        make_tarball(&src_v1, &tarball_v1);

        let r1 = adapter
            .deploy(
                make_artifact(tarball_v1, "u_test"),
                &manifest,
                &EnvSecretResolver,
            )
            .await
            .unwrap();
        let id_v1 = r1.deploy_id.clone();

        // v1 should be active
        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Active { .. }
        ));

        // v2: updated content
        let src_v2 = tmp.path().join("v2");
        make_universe_dir(&src_v2);
        std::fs::write(src_v2.join("index.html"), b"<h1>v2</h1>").unwrap();
        let tarball_v2 = tmp.path().join("v2.tar.gz");
        make_tarball(&src_v2, &tarball_v2);

        let r2 = adapter
            .deploy(
                make_artifact(tarball_v2, "u_test"),
                &manifest,
                &EnvSecretResolver,
            )
            .await
            .unwrap();
        let id_v2 = r2.deploy_id.clone();

        // v2 active, v1 inactive
        assert!(matches!(
            adapter.status(&id_v2).await,
            DeployStatus::Active { .. }
        ));
        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Inactive { .. }
        ));

        // Rollback to v1
        adapter.rollback(&id_v1).await.unwrap();

        // v1 active again, v2 inactive
        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Active { .. }
        ));
        assert!(matches!(
            adapter.status(&id_v2).await,
            DeployStatus::Inactive { .. }
        ));

        // v1 content is still in the bucket
        let v1_index_key = format!("co-deployments/{id_v1}/index.html");
        let content = s3.get_raw(&v1_index_key).expect("v1 index.html in bucket");
        assert_eq!(content, b"<h1>Hello</h1>");
    }

    // ── Integration test (real R2 bucket — skipped in CI) ────────────────────

    #[cfg(feature = "deploy-r2")]
    #[tokio::test]
    #[ignore = "requires real R2 bucket — set R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_BUCKET, CO_BASE_DOMAIN"]
    async fn test_integration_r2_deploy_and_rollback() {
        let account_id = std::env::var("R2_ACCOUNT_ID").expect("R2_ACCOUNT_ID");
        let access_key = std::env::var("R2_ACCESS_KEY_ID").expect("R2_ACCESS_KEY_ID");
        let secret_key = std::env::var("R2_SECRET_ACCESS_KEY").expect("R2_SECRET_ACCESS_KEY");
        let bucket = std::env::var("R2_BUCKET").expect("R2_BUCKET");
        let base_domain = std::env::var("CO_BASE_DOMAIN").unwrap_or_else(|_| "co.app".to_string());

        let adapter = StaticOnR2Adapter::from_credentials(
            &account_id,
            &access_key,
            &secret_key,
            &bucket,
            &base_domain,
        )
        .await;
        let manifest = make_manifest();

        let tmp = tempdir().unwrap();
        let src = tmp.path().join("u_integ");
        make_universe_dir(&src);
        let tarball_v1 = tmp.path().join("v1.tar.gz");
        make_tarball(&src, &tarball_v1);

        // Deploy v1
        let r1 = adapter
            .deploy(
                make_artifact(tarball_v1, "u_integ"),
                &manifest,
                &EnvSecretResolver,
            )
            .await
            .unwrap();
        let id_v1 = r1.deploy_id.clone();

        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Active { .. }
        ));

        // Deploy v2
        std::fs::write(src.join("index.html"), b"<h1>v2</h1>").unwrap();
        let tarball_v2 = tmp.path().join("v2.tar.gz");
        make_tarball(&src, &tarball_v2);

        let r2 = adapter
            .deploy(
                make_artifact(tarball_v2, "u_integ"),
                &manifest,
                &EnvSecretResolver,
            )
            .await
            .unwrap();
        let id_v2 = r2.deploy_id.clone();

        assert!(matches!(
            adapter.status(&id_v2).await,
            DeployStatus::Active { .. }
        ));
        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Inactive { .. }
        ));

        // Rollback to v1
        adapter.rollback(&id_v1).await.unwrap();

        assert!(matches!(
            adapter.status(&id_v1).await,
            DeployStatus::Active { .. }
        ));
        assert!(matches!(
            adapter.status(&id_v2).await,
            DeployStatus::Inactive { .. }
        ));
    }
}
