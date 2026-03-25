use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Path to Obsidian vault (optional)
    pub vault_path: Option<PathBuf>,
    /// Default GitHub owner/org for issue/PR creation
    pub default_owner: Option<String>,
    /// Claude model to use (default: claude-sonnet-4-20250514)
    pub claude_model: String,
    /// Whether to auto-approve pipeline stages
    pub auto_approve: bool,
    /// Directory for run logs
    pub logs_dir: Option<PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            vault_path: None,
            default_owner: None,
            claude_model: "claude-sonnet-4-20250514".into(),
            auto_approve: false,
            logs_dir: None,
        }
    }
}

impl EngineConfig {
    pub fn load() -> Self {
        let config_path = dirs_config_path();
        if config_path.exists()
            && let Ok(contents) = std::fs::read_to_string(&config_path)
            && let Ok(config) = serde_yaml::from_str(&contents)
        {
            return config;
        }
        Self::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = dirs_config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&config_path, yaml)?;
        Ok(())
    }
}

fn dirs_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/co-engine/config.yaml")
}
