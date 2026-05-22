//! Configuration for the co-sync daemon.
//!
//! Two config files:
//!   `co-universes.yaml`  — repo-level, committed, declares all universes
//!   `~/.co/sync.toml`   — user-level, stores token + resolved paths

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// co-universes.yaml  (repo-level, committed)
// ---------------------------------------------------------------------------

/// One-shot sync entry used by `co-sync push` in CI pipelines.
///
/// Sister repos declare these in their own `co-universes.yaml`:
/// ```yaml
/// syncs:
///   - source: work/yggdrasil/
///     target: yggdrasil
///     include: ["*.md"]
///     exclude: ["CLAUDE.md", ".gitkeep"]
///   - source: CHANGELOG.md
///     target: yggdrasil
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct SyncEntry {
    /// Local path (file or directory) relative to co-universes.yaml.
    pub source: String,
    /// CO universe slug to push into.
    pub target: String,
    /// Glob patterns to include (e.g. `["*.md"]`). Empty = include all.
    #[serde(default)]
    pub include: Vec<String>,
    /// Glob patterns to exclude (e.g. `["CLAUDE.md", ".gitkeep"]`).
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UniverseRegistry {
    pub server: String,
    #[serde(default)]
    pub universes: Vec<UniverseDecl>,
    /// One-shot push specs for CI use (`co-sync push`).
    #[serde(default)]
    pub syncs: Vec<SyncEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UniverseDecl {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `local` path (relative to co-universes.yaml or absolute).
    /// Absent means server_only = true (no local files to sync).
    pub local: Option<String>,
    /// Parent universe slug for hierarchical grouping.
    pub parent: Option<String>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// If true, universe is created on server but no files are synced.
    #[serde(default)]
    pub server_only: bool,
    /// Optional GitHub repo URL (informational; co-sync does not use git).
    pub github: Option<String>,
}

fn default_visibility() -> String {
    "private".into()
}

impl UniverseRegistry {
    /// Load from `co-universes.yaml`.
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Search for `co-universes.yaml` starting from `start_dir` and walking up.
    pub fn find(start_dir: &Path) -> Option<PathBuf> {
        let mut dir = start_dir.to_path_buf();
        loop {
            let candidate = dir.join("co-universes.yaml");
            if candidate.exists() {
                return Some(candidate);
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    /// Resolve the local path for a universe declaration relative to the
    /// registry file's directory (if the path is relative) or the home dir
    /// (if it starts with `~/`).
    pub fn resolve_local(decl: &UniverseDecl, registry_dir: &Path) -> Option<PathBuf> {
        let raw = decl.local.as_deref()?;
        let p = if raw.starts_with("~/") {
            dirs_next::home_dir()?.join(raw.strip_prefix("~/").unwrap_or(raw))
        } else if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            registry_dir.join(raw)
        };
        Some(p)
    }
}

// ---------------------------------------------------------------------------
// ~/.co/sync.toml  (user-level, stores token)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default = "default_server")]
    pub server: String,
    #[serde(default = "default_ws_url")]
    pub ws_url: String,
    pub token: String,
    #[serde(default)]
    pub universes: Vec<UniverseMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseMapping {
    pub slug: String,
    pub local: String,
}

fn default_server() -> String {
    "https://co.artelonga.com.br".into()
}

fn default_ws_url() -> String {
    "wss://co.artelonga.com.br/api/v1/sync/ws".into()
}

impl SyncConfig {
    pub fn default_path() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".co")
            .join("sync.toml")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml = toml::to_string_pretty(self)?;
        std::fs::write(path, toml.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}
