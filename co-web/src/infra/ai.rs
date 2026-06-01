//! CO-328: AI provider abstraction.
//!
//! `AiProvider` is the trait; `OllamaProvider` and `ClaudeCodeProvider` are the
//! default impls. `AiRouter` dispatches to the correct backend based on the
//! caller's hint. `MockProvider` is for tests only.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Availability and metadata for a single AI provider.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `true` when the provider responded to a recent connectivity check.
    pub warm: bool,
}

/// Text response from an AI provider.
#[derive(Clone, Debug)]
pub struct AiResponse {
    pub text: String,
    pub provider: String,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over AI backends (Ollama, Claude Code, mock).
///
/// Follows the CO-296 `AuthProvider` pattern: a trait with multiple impls
/// injected into `CoreState` at startup.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Run a prompt and return the full response text.
    async fn query(&self, prompt: &str) -> anyhow::Result<AiResponse>;

    /// Check whether this provider is currently reachable.
    async fn status(&self) -> ProviderStatus;

    /// Stable provider identifier: `"ollama"`, `"claude"`, or `"mock"`.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// OllamaProvider
// ---------------------------------------------------------------------------

/// Calls the Ollama HTTP API at `http://localhost:11434`.
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self::with_base_url(model, "http://localhost:11434")
    }

    pub fn with_base_url(model: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl AiProvider for OllamaProvider {
    async fn query(&self, prompt: &str) -> anyhow::Result<AiResponse> {
        let url = format!("{}/api/generate", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama unreachable: {e}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("Ollama returned HTTP {}", resp.status());
        }
        let json: serde_json::Value = resp.json().await?;
        let text = json["response"].as_str().unwrap_or("").to_string();
        Ok(AiResponse {
            text,
            provider: "ollama".into(),
        })
    }

    async fn status(&self) -> ProviderStatus {
        let url = format!("{}/api/tags", self.base_url);
        let ok = self.client.get(&url).send().await.is_ok();
        ProviderStatus {
            available: ok,
            model: Some(self.model.clone()),
            version: None,
            warm: ok,
        }
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}

// ---------------------------------------------------------------------------
// ClaudeCodeProvider
// ---------------------------------------------------------------------------

/// Spawns the `claude` CLI headlessly via `claude --print "<prompt>"`.
pub struct ClaudeCodeProvider {
    bin: String,
}

impl ClaudeCodeProvider {
    pub fn new() -> Self {
        Self {
            bin: detect_claude_bin().unwrap_or_else(|| "claude".to_string()),
        }
    }

    #[cfg(test)]
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Walk common install locations then fall back to `which`.
fn detect_claude_bin() -> Option<String> {
    for path in &["/usr/local/bin/claude", "/opt/homebrew/bin/claude"] {
        if std::path::Path::new(path).exists() {
            return Some((*path).to_string());
        }
    }
    std::process::Command::new("which")
        .arg("claude")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[async_trait]
impl AiProvider for ClaudeCodeProvider {
    async fn query(&self, prompt: &str) -> anyhow::Result<AiResponse> {
        let output = tokio::process::Command::new(&self.bin)
            .args(["--print", prompt])
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to spawn claude: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("claude exited non-zero: {stderr}");
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(AiResponse {
            text,
            provider: "claude".into(),
        })
    }

    async fn status(&self) -> ProviderStatus {
        let version = tokio::process::Command::new(&self.bin)
            .arg("--version")
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        ProviderStatus {
            available: version.is_some(),
            model: None,
            version,
            warm: false,
        }
    }

    fn name(&self) -> &'static str {
        "claude"
    }
}

// ---------------------------------------------------------------------------
// MockProvider
// ---------------------------------------------------------------------------

/// In-test stand-in for any AI backend.
pub struct MockProvider {
    response: String,
    available: bool,
}

impl MockProvider {
    pub fn available(response: impl Into<String>) -> Arc<dyn AiProvider> {
        Arc::new(Self {
            response: response.into(),
            available: true,
        })
    }

    pub fn unavailable() -> Arc<dyn AiProvider> {
        Arc::new(Self {
            response: String::new(),
            available: false,
        })
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    async fn query(&self, _prompt: &str) -> anyhow::Result<AiResponse> {
        if self.available {
            Ok(AiResponse {
                text: self.response.clone(),
                provider: "mock".into(),
            })
        } else {
            Err(anyhow::anyhow!("provider unavailable"))
        }
    }

    async fn status(&self) -> ProviderStatus {
        ProviderStatus {
            available: self.available,
            model: Some("mock".into()),
            version: Some("0.0.0".into()),
            warm: self.available,
        }
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// AiRouter
// ---------------------------------------------------------------------------

/// Dispatches queries to the correct provider based on an explicit hint.
pub struct AiRouter {
    pub ollama: Arc<dyn AiProvider>,
    pub claude: Arc<dyn AiProvider>,
}

impl AiRouter {
    pub fn new(ollama: Arc<dyn AiProvider>, claude: Arc<dyn AiProvider>) -> Self {
        Self { ollama, claude }
    }

    /// Production default: `OllamaProvider(qwen2.5-coder:7b)` + `ClaudeCodeProvider`.
    pub fn from_env() -> Self {
        Self::new(
            Arc::new(OllamaProvider::new("qwen2.5-coder:7b")),
            Arc::new(ClaudeCodeProvider::new()),
        )
    }

    /// Route a query to the named provider.
    pub async fn query(&self, provider: &str, prompt: &str) -> anyhow::Result<AiResponse> {
        match provider {
            "ollama" => self.ollama.query(prompt).await,
            "claude" => self.claude.query(prompt).await,
            other => anyhow::bail!("Unknown AI provider '{other}'. Supported: 'ollama', 'claude'."),
        }
    }

    pub async fn ollama_status(&self) -> ProviderStatus {
        self.ollama.status().await
    }

    pub async fn claude_status(&self) -> ProviderStatus {
        self.claude.status().await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_available_returns_response() {
        let p = MockProvider::available("hello from mock");
        let resp = p.query("test prompt").await.unwrap();
        assert_eq!(resp.text, "hello from mock");
        assert_eq!(resp.provider, "mock");
    }

    #[tokio::test]
    async fn mock_unavailable_returns_error() {
        let p = MockProvider::unavailable();
        let result = p.query("test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_status_reflects_availability() {
        let available = MockProvider::available("x");
        let s = available.status().await;
        assert!(s.available);
        assert!(s.warm);

        let unavailable = MockProvider::unavailable();
        let s = unavailable.status().await;
        assert!(!s.available);
        assert!(!s.warm);
    }

    #[tokio::test]
    async fn router_dispatches_to_ollama() {
        let ollama = MockProvider::available("ollama result");
        let claude = MockProvider::unavailable();
        let router = AiRouter::new(ollama, claude);
        let resp = router.query("ollama", "ping").await.unwrap();
        assert_eq!(resp.text, "ollama result");
    }

    #[tokio::test]
    async fn router_dispatches_to_claude() {
        let ollama = MockProvider::unavailable();
        let claude = MockProvider::available("claude result");
        let router = AiRouter::new(ollama, claude);
        let resp = router.query("claude", "ping").await.unwrap();
        assert_eq!(resp.text, "claude result");
    }

    #[tokio::test]
    async fn router_unknown_provider_errors() {
        let router = AiRouter::new(MockProvider::available("x"), MockProvider::available("y"));
        let result = router.query("gpt4", "ping").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown AI provider")
        );
    }

    #[tokio::test]
    async fn router_unavailable_provider_errors() {
        let router = AiRouter::new(MockProvider::unavailable(), MockProvider::unavailable());
        let result = router.query("ollama", "ping").await;
        assert!(result.is_err());
    }

    #[test]
    fn detect_claude_bin_does_not_panic() {
        // Smoke test: must not panic regardless of whether claude is installed.
        let _ = detect_claude_bin();
    }

    #[tokio::test]
    async fn claude_provider_status_when_bin_absent() {
        let p = ClaudeCodeProvider::with_bin("/nonexistent/bin/claude-not-real");
        let s = p.status().await;
        assert!(!s.available);
        assert!(s.version.is_none());
    }
}
