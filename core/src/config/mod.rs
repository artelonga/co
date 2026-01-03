//! Configuration system for CO
//!
//! Handles global config (~/.co/config.yaml), per-repo config (.co/config.yaml),
//! and group definitions (~/.co/groups.yaml) for multi-repository federation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A registered repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Absolute path to the repository
    pub path: PathBuf,
    /// Short alias for the repo
    pub alias: String,
}

/// Global CO configuration (~/.co/config.yaml)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Config version
    #[serde(default = "default_version")]
    pub version: u32,
    /// Registered repositories
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
}

fn default_version() -> u32 {
    1
}

impl GlobalConfig {
    /// Get path to global config file
    pub fn path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".co").join("config.yaml"))
    }

    /// Load global config from ~/.co/config.yaml
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save global config to ~/.co/config.yaml
    pub fn save(&self) -> Result<(), ConfigError> {
        let Some(path) = Self::path() else {
            return Err(ConfigError::NoHomeDir);
        };

        // Ensure .co directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Io(parent.to_path_buf(), e))?;
        }

        let content =
            serde_yaml::to_string(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;

        std::fs::write(&path, content).map_err(|e| ConfigError::Io(path, e))
    }

    /// Add a repository
    pub fn add_repo(&mut self, path: PathBuf, alias: String) {
        // Remove any existing repo with same path or alias
        self.repos.retain(|r| r.path != path && r.alias != alias);
        self.repos.push(RepoConfig { path, alias });
    }

    /// Remove a repository by alias or path
    pub fn remove_repo(&mut self, identifier: &str) -> bool {
        let before = self.repos.len();
        self.repos
            .retain(|r| r.alias != identifier && r.path.to_string_lossy() != identifier);
        self.repos.len() < before
    }

    /// Get repo by alias
    pub fn get_by_alias(&self, alias: &str) -> Option<&RepoConfig> {
        self.repos.iter().find(|r| r.alias == alias)
    }
}

/// Per-repo configuration (.co/config.yaml)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoLocalConfig {
    /// Tags for this repository (used in group queries)
    #[serde(default)]
    pub tags: Vec<String>,
}

impl RepoLocalConfig {
    /// Load per-repo config from .co/config.yaml in the given directory
    pub fn load(repo_root: &Path) -> Self {
        let path = repo_root.join(".co").join("config.yaml");
        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save per-repo config to .co/config.yaml
    pub fn save(&self, repo_root: &Path) -> Result<(), ConfigError> {
        let co_dir = repo_root.join(".co");
        std::fs::create_dir_all(&co_dir).map_err(|e| ConfigError::Io(co_dir.clone(), e))?;

        let path = co_dir.join("config.yaml");
        let content =
            serde_yaml::to_string(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;

        std::fs::write(&path, content).map_err(|e| ConfigError::Io(path, e))
    }
}

/// A group definition for federated queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupDef {
    /// Tags that repos must have to be in this group
    #[serde(default)]
    pub tags: Vec<String>,
    /// Explicit repo paths/aliases in this group
    #[serde(default)]
    pub repos: Vec<String>,
}

/// Group definitions (~/.co/groups.yaml)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupsConfig {
    /// Named groups
    #[serde(default)]
    pub groups: HashMap<String, GroupDef>,
}

impl GroupsConfig {
    /// Get path to groups config file
    pub fn path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".co").join("groups.yaml"))
    }

    /// Load groups config from ~/.co/groups.yaml
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save groups config to ~/.co/groups.yaml
    pub fn save(&self) -> Result<(), ConfigError> {
        let Some(path) = Self::path() else {
            return Err(ConfigError::NoHomeDir);
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Io(parent.to_path_buf(), e))?;
        }

        let content =
            serde_yaml::to_string(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;

        std::fs::write(&path, content).map_err(|e| ConfigError::Io(path, e))
    }

    /// Get a group by name
    pub fn get(&self, name: &str) -> Option<&GroupDef> {
        self.groups.get(name)
    }

    /// Add or update a group
    pub fn set(&mut self, name: String, def: GroupDef) {
        self.groups.insert(name, def);
    }

    /// Remove a group
    pub fn remove(&mut self, name: &str) -> bool {
        self.groups.remove(name).is_some()
    }
}

/// Resolve which repos match a filter specification
pub fn resolve_repos(
    filter: &str,
    global: &GlobalConfig,
    groups: &GroupsConfig,
) -> Vec<RepoConfig> {
    if let Some(group_name) = filter.strip_prefix('@') {
        // Group filter: @groupname
        if let Some(group) = groups.get(group_name) {
            let mut result = Vec::new();

            // Add repos matching tags
            for repo in &global.repos {
                let local_config = RepoLocalConfig::load(&repo.path);
                let has_tag = group.tags.iter().any(|t| local_config.tags.contains(t));
                let in_list = group
                    .repos
                    .iter()
                    .any(|r| r == &repo.alias || r.as_str() == repo.path.to_string_lossy());

                if has_tag || in_list {
                    result.push(repo.clone());
                }
            }

            return result;
        }
        Vec::new()
    } else if let Some(tag) = filter.strip_prefix('#') {
        // Tag filter: #tagname
        global
            .repos
            .iter()
            .filter(|repo| {
                let local_config = RepoLocalConfig::load(&repo.path);
                local_config.tags.contains(&tag.to_string())
            })
            .cloned()
            .collect()
    } else {
        // Alias or path filter
        global
            .repos
            .iter()
            .filter(|r| r.alias == filter || r.path.to_string_lossy() == filter)
            .cloned()
            .collect()
    }
}

/// Configuration errors
#[derive(Debug)]
pub enum ConfigError {
    /// No home directory found
    NoHomeDir,
    /// IO error
    Io(PathBuf, std::io::Error),
    /// Serialization error
    Serialize(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NoHomeDir => write!(f, "Could not determine home directory"),
            ConfigError::Io(path, e) => write!(f, "IO error at {}: {}", path.display(), e),
            ConfigError::Serialize(e) => write!(f, "Serialization error: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_repo_local_config_default() {
        let config = RepoLocalConfig::default();
        assert!(config.tags.is_empty());
    }

    #[test]
    fn test_repo_local_config_save_load() {
        let temp = TempDir::new().unwrap();
        let config = RepoLocalConfig {
            tags: vec!["work".to_string(), "client-x".to_string()],
        };

        config.save(temp.path()).unwrap();
        let loaded = RepoLocalConfig::load(temp.path());

        assert_eq!(loaded.tags, config.tags);
    }

    #[test]
    fn test_global_config_add_remove_repo() {
        let mut config = GlobalConfig::default();

        config.add_repo(PathBuf::from("/path/to/repo"), "myrepo".to_string());
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].alias, "myrepo");

        // Adding same path with different alias replaces
        config.add_repo(PathBuf::from("/path/to/repo"), "newname".to_string());
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].alias, "newname");

        assert!(config.remove_repo("newname"));
        assert!(config.repos.is_empty());
    }

    #[test]
    fn test_groups_config() {
        let mut config = GroupsConfig::default();

        config.set(
            "work".to_string(),
            GroupDef {
                tags: vec!["work".to_string()],
                repos: vec![],
            },
        );

        assert!(config.get("work").is_some());
        assert!(config.get("nonexistent").is_none());

        assert!(config.remove("work"));
        assert!(config.get("work").is_none());
    }
}
