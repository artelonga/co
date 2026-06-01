//! CO-328: AI provider abstraction.
//!
//! `AiProvider` is the trait; `OllamaProvider` and `ClaudeCodeProvider` are the
//! default impls. `AiRouter` dispatches to the correct backend based on the
//! caller's hint. `MockProvider` is for tests only.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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
// Chat types — CO-332
// ---------------------------------------------------------------------------

/// A single message in a multi-turn chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AssistantToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A tool call made by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

/// The function component of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// JSON-encoded arguments string (as returned by the model).
    pub arguments: String,
}

/// Tool definition sent to the LLM (OpenAI function-calling format).
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunctionDef,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDef {
    pub fn function(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            tool_type: "function".into(),
            function: ToolFunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// The result of one LLM turn — either content text or tool calls.
pub struct ChatTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<AssistantToolCall>,
    pub provider: String,
}

// ---------------------------------------------------------------------------
// ChatProvider trait — CO-332
// ---------------------------------------------------------------------------

/// Trait for LLMs that support multi-turn chat with function calling.
///
/// CO-332: Implementations MUST NOT have `kind() == "claude"`.
/// The `/chat` endpoint is reserved for non-Claude providers (Ollama, OpenAI).
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Stable provider identifier — MUST NOT be `"claude"`.
    fn kind(&self) -> &'static str;

    /// Execute one turn of a multi-turn conversation, optionally with tools.
    async fn chat(&self, messages: &[ChatMessage], tools: &[ToolDef]) -> anyhow::Result<ChatTurn>;
}

// ---------------------------------------------------------------------------
// OpenAiCompatChatProvider — Ollama + real OpenAI via the same format
// ---------------------------------------------------------------------------

/// Calls any OpenAI-compatible `/v1/chat/completions` endpoint.
///
/// - Ollama (local): `base_url = "http://localhost:11434"`, `api_key = None`
/// - OpenAI (cloud): `base_url = "https://api.openai.com"`, `api_key = Some(key)`
pub struct OpenAiCompatChatProvider {
    kind: &'static str,
    model: String,
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiCompatChatProvider {
    pub fn ollama(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            kind: "ollama",
            model: model.into(),
            base_url: base_url.into(),
            api_key: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            kind: "openai",
            model: model.into(),
            base_url: "https://api.openai.com".into(),
            api_key: Some(api_key.into()),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatChatProvider {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn chat(&self, messages: &[ChatMessage], tools: &[ToolDef]) -> anyhow::Result<ChatTurn> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools)?;
            body["tool_choice"] = serde_json::json!("auto");
        }

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Chat provider unreachable: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Chat provider returned {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        let message = &json["choices"][0]["message"];

        let content = message["content"].as_str().map(|s| s.to_string());

        let tool_calls: Vec<AssistantToolCall> = message["tool_calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| serde_json::from_value(c.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(ChatTurn {
            content,
            tool_calls,
            provider: self.kind.into(),
        })
    }
}

// ---------------------------------------------------------------------------
// MockChatProvider — for tests (never "claude")
// ---------------------------------------------------------------------------

pub struct MockChatProvider {
    kind: &'static str,
    content: String,
}

impl MockChatProvider {
    /// Construct a mock chat provider. Panics if `kind == "claude"`.
    pub fn mock(kind: &'static str, content: impl Into<String>) -> Arc<dyn ChatProvider> {
        assert_ne!(kind, "claude", "MockChatProvider cannot impersonate claude");
        Arc::new(Self {
            kind,
            content: content.into(),
        })
    }
}

#[async_trait]
impl ChatProvider for MockChatProvider {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolDef],
    ) -> anyhow::Result<ChatTurn> {
        Ok(ChatTurn {
            content: Some(self.content.clone()),
            tool_calls: vec![],
            provider: self.kind.into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Factory — CO-332: /chat NEVER uses Claude
// ---------------------------------------------------------------------------

/// Build the public-facing chat provider for `POST /api/v1/chat/:slug`.
///
/// CO-332: This function MUST NOT return a Claude provider.
/// Claude is reserved for Yuri's authenticated dev tooling (co-auto, CLI).
/// Visitor-facing chat uses Ollama (default) or OpenAI (fallback).
///
/// Selection:
/// - `CO_CHAT_FALLBACK=openai` + `OPENAI_API_KEY` → OpenAI
/// - anything else → Ollama at `CO_OLLAMA_URL` (default: `http://localhost:11434`)
pub fn build_chat_provider() -> Arc<dyn ChatProvider> {
    let provider: Arc<dyn ChatProvider> =
        match std::env::var("CO_CHAT_FALLBACK").as_deref().unwrap_or("") {
            "openai" => {
                let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                let model = std::env::var("CO_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
                Arc::new(OpenAiCompatChatProvider::openai(key, model))
            }
            _ => {
                let base_url = std::env::var("CO_OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".into());
                let model =
                    std::env::var("CO_CHAT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:7b".into());
                Arc::new(OpenAiCompatChatProvider::ollama(base_url, model))
            }
        };
    debug_assert_ne!(
        provider.kind(),
        "claude",
        "CO-332 invariant: /chat must never use Claude"
    );
    provider
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

    // --- CO-332: ChatProvider tests ---

    #[test]
    fn build_chat_provider_is_never_claude() {
        let p = build_chat_provider();
        assert_ne!(
            p.kind(),
            "claude",
            "CO-332: /chat provider must never be claude"
        );
    }

    #[tokio::test]
    async fn mock_chat_provider_returns_content() {
        let p = MockChatProvider::mock("mock", "hello world");
        let turn = p.chat(&[], &[]).await.unwrap();
        assert_eq!(turn.content.as_deref(), Some("hello world"));
        assert!(turn.tool_calls.is_empty());
        assert_eq!(turn.provider, "mock");
    }

    #[tokio::test]
    async fn mock_chat_provider_kind_is_not_claude() {
        let p = MockChatProvider::mock("mock", "hi");
        assert_ne!(p.kind(), "claude");
    }

    #[test]
    fn chat_message_constructors() {
        let sys = ChatMessage::system("hello");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content.as_deref(), Some("hello"));

        let user = ChatMessage::user("world");
        assert_eq!(user.role, "user");

        let tool = ChatMessage::tool_result("id-1", "result");
        assert_eq!(tool.role, "tool");
        assert_eq!(tool.tool_call_id.as_deref(), Some("id-1"));
    }

    #[test]
    fn tool_def_constructor() {
        let t = ToolDef::function(
            "search_entries",
            "search content",
            serde_json::json!({"type": "object", "properties": {}}),
        );
        assert_eq!(t.tool_type, "function");
        assert_eq!(t.function.name, "search_entries");
    }
}
