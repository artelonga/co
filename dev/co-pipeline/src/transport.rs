//! CO-88 — the four pipeline paths and how wire bytes travel each.
//!
//! - Path A `local`     — filesystem read/write only (no server).
//! - Path B `local-api` — PUT/GET against a locally-running co-web.
//! - Path C `uat`       — PUT/GET against co-artelonga-uat.fly.dev.
//! - Path D `prod`      — GET-only smoke against co-artelonga.fly.dev.
//!
//! A [`Transport`] round-trips already-encoded bytes: `put` stores them, `get`
//! reads them back. The encode/decode happens client-side in [`crate::pipeline`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

/// The four pipeline paths in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PipePath {
    Local,
    LocalApi,
    Uat,
    Prod,
}

impl PipePath {
    pub const ALL: [PipePath; 4] = [
        PipePath::Local,
        PipePath::LocalApi,
        PipePath::Uat,
        PipePath::Prod,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PipePath::Local => "local",
            PipePath::LocalApi => "local-api",
            PipePath::Uat => "uat",
            PipePath::Prod => "prod",
        }
    }

    pub fn from_label(s: &str) -> Option<PipePath> {
        PipePath::ALL.into_iter().find(|p| p.label() == s)
    }

    /// Path D is read-only — the harness never writes to prod.
    pub fn read_only(self) -> bool {
        matches!(self, PipePath::Prod)
    }
}

/// A bytes round-tripper for one path. Returns the read-back bytes plus the
/// observed transfer time in milliseconds.
pub trait Transport {
    fn put(&self, slug: &str, path: &str, bytes: &[u8]) -> Result<()>;
    fn get(&self, slug: &str, path: &str) -> Result<(Vec<u8>, u64)>;
}

/// Path A — write/read under a scratch directory, no network.
pub struct LocalFsTransport {
    root: PathBuf,
}

impl LocalFsTransport {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn file_path(&self, slug: &str, path: &str) -> PathBuf {
        // Flatten the vault path so arbitrary nesting maps to a single file.
        let safe = path.replace(['/', '\\'], "__");
        self.root.join(slug).join(safe)
    }
}

impl Transport for LocalFsTransport {
    fn put(&self, slug: &str, path: &str, bytes: &[u8]) -> Result<()> {
        let target = self.file_path(slug, path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).context("create scratch dir")?;
        }
        std::fs::write(&target, bytes).context("write scratch file")?;
        Ok(())
    }

    fn get(&self, slug: &str, path: &str) -> Result<(Vec<u8>, u64)> {
        let start = Instant::now();
        let target = self.file_path(slug, path);
        let bytes = std::fs::read(&target).context("read scratch file")?;
        Ok((bytes, start.elapsed().as_millis() as u64))
    }
}

/// Paths B/C/D — HTTP PUT/GET against a co-web vault endpoint.
///
/// Wire bytes ride the request/response body. The pipeline `combo` is announced
/// via `X-Co-*` headers so the server records matching telemetry (CO-88b).
pub struct HttpTransport {
    base: String,
    token: Option<String>,
    read_only: bool,
    combo_label: String,
    format: String,
    compression: String,
    encryption: String,
}

impl HttpTransport {
    pub fn new(base: impl Into<String>, token: Option<String>, read_only: bool) -> Self {
        Self {
            base: base.into(),
            token,
            read_only,
            combo_label: String::new(),
            format: "co/protobuf".into(),
            compression: "raw".into(),
            encryption: "none".into(),
        }
    }

    /// Annotate subsequent requests with the active combo's telemetry headers.
    pub fn with_combo(mut self, combo: crate::combo::Combo) -> Self {
        self.combo_label = combo.label().into();
        self.format = combo.format_label().into();
        self.compression = combo.compression_label().into();
        self.encryption = combo.encryption_label().into();
        self
    }

    fn url(&self, slug: &str, path: &str) -> String {
        format!(
            "{}/api/v1/universes/{}/vault/{}",
            self.base.trim_end_matches('/'),
            slug,
            path
        )
    }

    fn apply_headers<B>(&self, mut req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        if let Some(tok) = &self.token {
            req = req.header("Authorization", &format!("Bearer {tok}"));
        }
        req.header("X-Co-Format", &self.format)
            .header("X-Co-Compression", &self.compression)
            .header("X-Co-Encryption", &self.encryption)
            .header("X-Co-Combo", &self.combo_label)
    }
}

impl Transport for HttpTransport {
    fn put(&self, slug: &str, path: &str, bytes: &[u8]) -> Result<()> {
        if self.read_only {
            bail!("path is read-only; refusing to PUT");
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        let req = self.apply_headers(agent.put(&self.url(slug, path)));
        let resp = req
            .send(bytes)
            .with_context(|| format!("PUT {}", self.url(slug, path)))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            bail!("PUT returned {status}");
        }
        Ok(())
    }

    fn get(&self, slug: &str, path: &str) -> Result<(Vec<u8>, u64)> {
        let start = Instant::now();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        let req = self.apply_headers(agent.get(&self.url(slug, path)));
        let mut resp = req
            .call()
            .with_context(|| format!("GET {}", self.url(slug, path)))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            bail!("GET returned {status}");
        }
        let bytes = resp
            .body_mut()
            .read_to_vec()
            .context("read response body")?;
        Ok((bytes, start.elapsed().as_millis() as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_labels_roundtrip() {
        for p in PipePath::ALL {
            assert_eq!(PipePath::from_label(p.label()), Some(p));
        }
        assert!(PipePath::Prod.read_only());
        assert!(!PipePath::Local.read_only());
    }

    #[test]
    fn local_fs_transport_roundtrips_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let t = LocalFsTransport::new(dir.path().to_path_buf());
        t.put("artelonga", "notes/a.md", b"hello").unwrap();
        let (back, _ms) = t.get("artelonga", "notes/a.md").unwrap();
        assert_eq!(back, b"hello");
    }

    #[test]
    fn http_transport_refuses_put_when_read_only() {
        let t = HttpTransport::new("https://example.invalid", None, true);
        assert!(t.put("u", "p.md", b"x").is_err());
    }
}
