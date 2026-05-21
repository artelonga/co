//! Async HTTP client wrapping the CO Vault API.
//! Mirrors the TypeScript CoApiClient in co-vscode/src/api.ts.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

// Fields are part of the API contract; fully used by Phase 2 hover/stat features.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct VaultStat {
    pub ctime: i64,
    pub mtime: i64,
    pub size: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VaultFileInfo {
    pub path: String,
    // stat is part of the API contract; used by Phase 2 stat/hover features.
    #[allow(dead_code)]
    pub stat: VaultStat,
}

// Used by get_file — Phase 2 hover + definition features.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct VaultFile {
    pub path: String,
    pub content: String,
    pub frontmatter: serde_json::Value,
    pub tags: Vec<String>,
    pub stat: VaultStat,
}

#[derive(Debug, Deserialize)]
pub struct VaultListResponse {
    pub files: Vec<VaultFileInfo>,
}

pub struct VaultClient {
    http: Client,
    base_url: String,
    token: String,
}

impl VaultClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    fn vault_url(&self, slug: &str, path: &str) -> String {
        if path.is_empty() {
            format!("{}/api/v1/universes/{}/vault/", self.base_url, slug)
        } else {
            format!(
                "{}/api/v1/universes/{}/vault/{}",
                self.base_url,
                slug,
                path.trim_start_matches('/')
            )
        }
    }

    pub async fn list_files(&self, slug: &str) -> Result<Vec<VaultFileInfo>> {
        let resp: VaultListResponse = self
            .http
            .get(self.vault_url(slug, ""))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.files)
    }

    // Phase 2 — used by hover and definition providers.
    #[allow(dead_code)]
    pub async fn get_file(&self, slug: &str, path: &str) -> Result<VaultFile> {
        let file: VaultFile = self
            .http
            .get(self.vault_url(slug, path))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(file)
    }

    // Phase 2 — used by document-save handler.
    #[allow(dead_code)]
    pub async fn put_file(&self, slug: &str, path: &str, content: &str) -> Result<()> {
        self.http
            .put(self.vault_url(slug, path))
            .bearer_auth(&self.token)
            .header("Content-Type", "text/markdown")
            .body(content.to_owned())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_url_root() {
        let c = VaultClient::new("https://co.example.com", "tok");
        assert_eq!(
            c.vault_url("my-slug", ""),
            "https://co.example.com/api/v1/universes/my-slug/vault/"
        );
    }

    #[test]
    fn vault_url_file() {
        let c = VaultClient::new("https://co.example.com", "tok");
        assert_eq!(
            c.vault_url("my-slug", "notes/foo.md"),
            "https://co.example.com/api/v1/universes/my-slug/vault/notes/foo.md"
        );
    }

    #[test]
    fn vault_url_strips_leading_slash() {
        let c = VaultClient::new("https://co.example.com", "tok");
        assert_eq!(
            c.vault_url("my-slug", "/notes/foo.md"),
            "https://co.example.com/api/v1/universes/my-slug/vault/notes/foo.md"
        );
    }

    #[test]
    fn base_url_trailing_slash_stripped() {
        let c = VaultClient::new("https://co.example.com/", "tok");
        assert_eq!(
            c.vault_url("slug", ""),
            "https://co.example.com/api/v1/universes/slug/vault/"
        );
    }
}
