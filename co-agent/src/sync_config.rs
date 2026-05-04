//! `~/.co/sync.toml` — configuration for the co-sync daemon.
//!
//! ```toml
//! server   = "https://co.artelonga.com.br"
//! ws_url   = "wss://co.artelonga.com.br/api/v1/sync/ws"
//! token    = "tok_xxxxxxxxxxxx"  # long-lived API token, never expires manually
//!
//! [[universes]]
//! slug  = "co"
//! local = "/Users/yuri/projects/co"
//!
//! [[universes]]
//! slug  = "mbya"
//! local = "/Users/yuri/projects/mbya"
//!
//! [[universes]]
//! slug  = "topologia"
//! local = "/Users/yuri/projects/topologia"
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// HTTPS base URL, e.g. `https://co.artelonga.com.br`.
    #[serde(default = "default_server")]
    pub server: String,

    /// WebSocket URL for the CO-151 sync endpoint.
    #[serde(default = "default_ws_url")]
    pub ws_url: String,

    /// Long-lived API token (CO-35). Set once via `co-sync init`.
    pub token: String,

    /// Universe → local directory mappings.
    #[serde(default)]
    pub universes: Vec<UniverseMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseMapping {
    /// Universe slug on the server (e.g. `co`, `mbya`, `topologia`).
    pub slug: String,
    /// Absolute path to the local directory to watch.
    pub local: String,
}

fn default_server() -> String {
    "https://co.artelonga.com.br".into()
}

fn default_ws_url() -> String {
    "wss://co.artelonga.com.br/api/v1/sync/ws".into()
}

impl SyncConfig {
    /// Default config file path: `~/.co/sync.toml`.
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
        // Restrict permissions to owner-only (token is sensitive).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}
