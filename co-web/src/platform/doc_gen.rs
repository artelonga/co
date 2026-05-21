//! Doc-generator adapter trait and stub implementations (CO-72).
//!
//! Each adapter is a thin wrapper around an external toolchain (scaladoc,
//! sphinx-build, mkdocs, redoc, rustdoc, jsdoc). In v1 these run in-process
//! as stubs; the full toolchain invocation happens on dedicated worker machines
//! once CO-78 Fly-machine autoscaling is live.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocFormat {
    Scaladoc,
    Sphinx,
    Mkdocs,
    Redoc,
    Rustdoc,
    Jsdoc,
}

impl DocFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            DocFormat::Scaladoc => "scaladoc",
            DocFormat::Sphinx => "sphinx",
            DocFormat::Mkdocs => "mkdocs",
            DocFormat::Redoc => "redoc",
            DocFormat::Rustdoc => "rustdoc",
            DocFormat::Jsdoc => "jsdoc",
        }
    }

    pub fn adapter_version(self) -> &'static str {
        match self {
            DocFormat::Scaladoc => "scaladoc-v1",
            DocFormat::Sphinx => "sphinx-v1",
            DocFormat::Mkdocs => "mkdocs-v1",
            DocFormat::Redoc => "redoc-v1",
            DocFormat::Rustdoc => "rustdoc-v1",
            DocFormat::Jsdoc => "jsdoc-v1",
        }
    }
}

impl std::str::FromStr for DocFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "scaladoc" => Ok(DocFormat::Scaladoc),
            "sphinx" => Ok(DocFormat::Sphinx),
            "mkdocs" => Ok(DocFormat::Mkdocs),
            "redoc" => Ok(DocFormat::Redoc),
            "rustdoc" => Ok(DocFormat::Rustdoc),
            "jsdoc" => Ok(DocFormat::Jsdoc),
            _ => Err(()),
        }
    }
}

/// A single documentation entry produced by an adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocEntry {
    /// Relative path within the universe (e.g. `docs/scaladoc/MyClass.md`).
    pub path: String,
    pub title: String,
    /// Entry type tag (e.g. `doc.scala`, `doc.rst`, `doc.endpoint`).
    pub entry_type: String,
    pub frontmatter: serde_json::Value,
    pub body: String,
}

/// Resource limits enforced per doc-gen job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum wall-clock seconds. Default: 300 (5 min).
    pub wall_time_secs: u64,
    /// Maximum RAM in MB. Default: 2048 (2 GB).
    pub ram_mb: u64,
    /// Maximum output size in MB. Default: 1024 (1 GB).
    pub output_mb: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            wall_time_secs: 300,
            ram_mb: 2048,
            output_mb: 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// Adapter trait
// ---------------------------------------------------------------------------

/// All doc-generator adapters implement this trait.
///
/// `run` validates the source path and produces entries. In v1 all adapters
/// are stubs — they verify the source directory exists and return one
/// placeholder entry per adapter so the job queue and error-surfacing paths
/// can be exercised end-to-end without requiring the real toolchain.
pub trait DocAdapter: Send + Sync {
    fn format(&self) -> DocFormat;
    fn adapter_version(&self) -> &'static str;

    fn run(
        &self,
        source_dir: &Path,
        output_type: &str,
        _limits: &ResourceLimits,
    ) -> Result<Vec<DocEntry>, String>;
}

// ---------------------------------------------------------------------------
// Concrete adapters (v1 stubs)
// ---------------------------------------------------------------------------

macro_rules! stub_adapter {
    ($name:ident, $fmt:expr, $section:expr, $entry_type:expr) => {
        pub struct $name;

        impl DocAdapter for $name {
            fn format(&self) -> DocFormat {
                $fmt
            }

            fn adapter_version(&self) -> &'static str {
                $fmt.adapter_version()
            }

            fn run(
                &self,
                source_dir: &Path,
                output_type: &str,
                _limits: &ResourceLimits,
            ) -> Result<Vec<DocEntry>, String> {
                if !source_dir.exists() {
                    return Err(format!(
                        "{}: source directory does not exist: {}",
                        self.format().as_str(),
                        source_dir.display()
                    ));
                }
                let otype = if output_type.is_empty() {
                    $entry_type
                } else {
                    output_type
                };
                Ok(vec![DocEntry {
                    path: format!("docs/{}/index.md", self.format().as_str()),
                    title: format!("{} documentation", self.format().as_str()),
                    entry_type: otype.to_string(),
                    frontmatter: json!({
                        "type": otype,
                        "source_dir": source_dir.to_string_lossy(),
                        "adapter": self.format().as_str(),
                        "adapter_version": self.adapter_version(),
                        "section": $section,
                        "tags": ["generated", self.format().as_str()]
                    }),
                    body: format!(
                        "# {} documentation\n\nGenerated from `{}`.",
                        self.format().as_str(),
                        source_dir.display()
                    ),
                }])
            }
        }
    };
}

stub_adapter!(ScaladocAdapter, DocFormat::Scaladoc, "package", "doc.scala");
stub_adapter!(SphinxAdapter, DocFormat::Sphinx, "module", "doc.rst");
stub_adapter!(MkdocsAdapter, DocFormat::Mkdocs, "page", "doc.md");
stub_adapter!(RedocAdapter, DocFormat::Redoc, "endpoint", "doc.endpoint");
stub_adapter!(RustdocAdapter, DocFormat::Rustdoc, "crate", "doc.rust");
stub_adapter!(JsdocAdapter, DocFormat::Jsdoc, "module", "doc.js");

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Build the adapter for a given format and call `run`.
pub fn run_adapter(
    format: DocFormat,
    source_dir: &Path,
    output_type: &str,
    limits: &ResourceLimits,
) -> Result<Vec<DocEntry>, String> {
    match format {
        DocFormat::Scaladoc => ScaladocAdapter.run(source_dir, output_type, limits),
        DocFormat::Sphinx => SphinxAdapter.run(source_dir, output_type, limits),
        DocFormat::Mkdocs => MkdocsAdapter.run(source_dir, output_type, limits),
        DocFormat::Redoc => RedocAdapter.run(source_dir, output_type, limits),
        DocFormat::Rustdoc => RustdocAdapter.run(source_dir, output_type, limits),
        DocFormat::Jsdoc => JsdocAdapter.run(source_dir, output_type, limits),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scaladoc_adapter_runs_on_existing_dir() {
        let dir = tempdir().unwrap();
        let adapter = ScaladocAdapter;
        let result = adapter.run(dir.path(), "doc.scala", &ResourceLimits::default());
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_type, "doc.scala");
    }

    #[test]
    fn adapter_fails_on_missing_dir() {
        let adapter = ScaladocAdapter;
        let result = adapter.run(
            Path::new("/nonexistent/path/xyz"),
            "doc.scala",
            &ResourceLimits::default(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn all_formats_round_trip_from_str() {
        for (s, expected) in &[
            ("scaladoc", DocFormat::Scaladoc),
            ("sphinx", DocFormat::Sphinx),
            ("mkdocs", DocFormat::Mkdocs),
            ("redoc", DocFormat::Redoc),
            ("rustdoc", DocFormat::Rustdoc),
            ("jsdoc", DocFormat::Jsdoc),
        ] {
            assert_eq!(s.parse::<DocFormat>().ok(), Some(*expected));
        }
        assert!("unknown".parse::<DocFormat>().is_err());
    }

    #[test]
    fn run_adapter_dispatches_all_formats() {
        let dir = tempdir().unwrap();
        let limits = ResourceLimits::default();
        for fmt in [
            DocFormat::Scaladoc,
            DocFormat::Sphinx,
            DocFormat::Mkdocs,
            DocFormat::Redoc,
            DocFormat::Rustdoc,
            DocFormat::Jsdoc,
        ] {
            let result = run_adapter(fmt, dir.path(), fmt.as_str(), &limits);
            assert!(result.is_ok(), "{} adapter failed", fmt.as_str());
        }
    }

    #[test]
    fn output_type_defaults_when_empty() {
        let dir = tempdir().unwrap();
        let adapter = RustdocAdapter;
        let entries = adapter
            .run(dir.path(), "", &ResourceLimits::default())
            .unwrap();
        assert_eq!(entries[0].entry_type, "doc.rust");
    }
}
