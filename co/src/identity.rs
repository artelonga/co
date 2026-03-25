use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A GitHub identity with SSH key and credential info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitIdentity {
    /// Display name (e.g., "personal", "work")
    pub name: String,
    /// GitHub username
    pub github_user: String,
    /// SSH host alias (e.g., "github.com", "github-work")
    /// Maps to ~/.ssh/config Host entries
    pub ssh_host: String,
    /// Git user.email for commits
    pub git_email: String,
    /// Git user.name for commits
    pub git_name: String,
}

/// Manages multiple GitHub identities per machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityStore {
    /// Active identity name
    pub active: String,
    /// All configured identities
    pub identities: HashMap<String, GitIdentity>,
}

impl Default for IdentityStore {
    fn default() -> Self {
        // Auto-detect current git identity as default
        let (user, email) = detect_git_identity();
        let gh_user = detect_gh_user().unwrap_or_else(|| user.clone());

        let default = GitIdentity {
            name: "default".into(),
            github_user: gh_user,
            ssh_host: "github.com".into(),
            git_email: email,
            git_name: user,
        };

        let mut identities = HashMap::new();
        identities.insert("default".into(), default);

        Self {
            active: "default".into(),
            identities,
        }
    }
}

impl IdentityStore {
    pub fn load() -> Self {
        let path = identity_store_path();
        if path.exists()
            && let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(store) = serde_yaml::from_str(&contents)
        {
            return store;
        }
        Self::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = identity_store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&path, yaml)?;
        Ok(())
    }

    /// Get the currently active identity
    pub fn active_identity(&self) -> Option<&GitIdentity> {
        self.identities.get(&self.active)
    }

    /// Switch to a different identity
    pub fn switch(&mut self, name: &str) -> anyhow::Result<()> {
        if !self.identities.contains_key(name) {
            anyhow::bail!(
                "Unknown identity '{}'. Available: {}",
                name,
                self.identities
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        self.active = name.into();
        self.save()?;
        Ok(())
    }

    /// Add a new identity
    pub fn add(&mut self, identity: GitIdentity) -> anyhow::Result<()> {
        let name = identity.name.clone();
        self.identities.insert(name, identity);
        self.save()?;
        Ok(())
    }

    /// Rewrite a remote URL to use the active identity's SSH host
    /// e.g., git@github.com:owner/repo → git@github-work:owner/repo
    pub fn rewrite_remote(&self, remote_url: &str) -> String {
        if let Some(identity) = self.active_identity()
            && identity.ssh_host != "github.com"
        {
            return remote_url.replace("github.com", &identity.ssh_host);
        }
        remote_url.to_string()
    }

    /// Get the repo string for gh CLI, using active identity's user context
    pub fn gh_repo_flag(&self) -> Vec<String> {
        // gh uses the token from `gh auth`, not SSH keys
        // For multi-account, user should `gh auth switch` or use GH_TOKEN env
        Vec::new()
    }

    /// Get git config args for the active identity (for commits)
    pub fn git_config_args(&self) -> Vec<String> {
        if let Some(identity) = self.active_identity() {
            vec![
                "-c".into(),
                format!("user.name={}", identity.git_name),
                "-c".into(),
                format!("user.email={}", identity.git_email),
            ]
        } else {
            Vec::new()
        }
    }

    pub fn list(&self) -> Vec<(&str, &GitIdentity, bool)> {
        self.identities
            .iter()
            .map(|(name, id)| (name.as_str(), id, name == &self.active))
            .collect()
    }
}

fn identity_store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/co-engine/identities.yaml")
}

fn detect_git_identity() -> (String, String) {
    let name = std::process::Command::new("git")
        .args(["config", "--global", "user.name"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into());

    let email = std::process::Command::new("git")
        .args(["config", "--global", "user.email"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown@unknown".into());

    (name, email)
}

fn detect_gh_user() -> Option<String> {
    let output = std::process::Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
