//! tower-lsp backend — wikilink completion, hover, definition, diagnostics.

use std::sync::Arc;

use anyhow::Result;
use lsp_types::*;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types;
use tower_lsp::{Client, LanguageServer};

use crate::config::LspConfig;
use crate::vault_client::{VaultClient, VaultFileInfo};

// ─── Wikilink parsing ─────────────────────────────────────────────────────────

/// Extract all `[[target]]` or `[[target|label]]` wikilinks from text.
pub fn extract_wikilinks(text: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            if let Some(end) = text[start..].find("]]") {
                let inner = &text[start..start + end];
                // target is the part before "|"
                let target = inner.split('|').next().unwrap_or(inner).trim();
                if !target.is_empty() {
                    results.push(target);
                }
                i = start + end + 2;
                continue;
            }
        }
        i += 1;
    }
    results
}

/// Returns true if `pos` in `line` is immediately after `[[`.
/// Used by unit tests and by editor integrations that inspect trigger context.
#[allow(dead_code)]
pub fn is_wikilink_trigger(line: &str, char_offset: usize) -> bool {
    if char_offset < 2 {
        return false;
    }
    line[..char_offset].ends_with("[[")
}

// ─── Entry index cache ────────────────────────────────────────────────────────

#[derive(Default)]
struct EntryCache {
    files: Vec<VaultFileInfo>,
}

// ─── Backend ─────────────────────────────────────────────────────────────────

pub struct CoLspBackend {
    client: Client,
    config: LspConfig,
    vault: Arc<VaultClient>,
    cache: Arc<RwLock<EntryCache>>,
}

impl CoLspBackend {
    pub fn new(client: Client, config: LspConfig) -> Self {
        let vault = Arc::new(VaultClient::new(&config.instance_url, &config.api_token));
        Self {
            client,
            config,
            vault,
            cache: Arc::new(RwLock::new(EntryCache::default())),
        }
    }

    async fn refresh_cache(&self, slug: &str) -> Result<()> {
        let files = self.vault.list_files(slug).await?;
        let mut cache = self.cache.write().await;
        cache.files = files;
        Ok(())
    }

    /// Infer universe slug from the document URI (co://slug/path).
    fn slug_from_uri(uri: &Url) -> Option<String> {
        let host = uri.host_str()?;
        if !host.is_empty() {
            return Some(host.to_owned());
        }
        None
    }

    async fn diagnostics_for_doc(&self, uri: &Url, text: &str) -> Vec<Diagnostic> {
        let Some(slug) = Self::slug_from_uri(uri) else {
            return vec![];
        };

        // Ensure cache is populated
        if self.cache.read().await.files.is_empty()
            && let Err(e) = self.refresh_cache(&slug).await
        {
            self.client
                .log_message(MessageType::WARNING, format!("cache refresh: {e}"))
                .await;
        }

        let cache = self.cache.read().await;
        let known: std::collections::HashSet<&str> = cache
            .files
            .iter()
            .map(|f| f.path.trim_end_matches(".md"))
            .collect();

        let mut diags = Vec::new();
        for (line_idx, line) in text.lines().enumerate() {
            for target in extract_wikilinks(line) {
                if !known.contains(target) && !known.contains(&format!("{}.md", target).as_str()) {
                    // Find column position of [[target
                    if let Some(col) = line.find(&format!("[[{}", target)) {
                        diags.push(Diagnostic {
                            range: Range {
                                start: Position {
                                    line: line_idx as u32,
                                    character: col as u32,
                                },
                                end: Position {
                                    line: line_idx as u32,
                                    character: (col + target.len() + 4) as u32,
                                },
                            },
                            severity: Some(DiagnosticSeverity::WARNING),
                            source: Some("co-lsp".into()),
                            message: format!("Wikilink '[[{target}]]' — entry not found in universe"),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        diags
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CoLspBackend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["[".into()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "co-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "co-lsp initialized")
            .await;

        if let Some(slug) = &self.config.universe_slug
            && let Err(e) = self.refresh_cache(slug).await
        {
            self.client
                .log_message(MessageType::WARNING, format!("initial cache load: {e}"))
                .await;
        }
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = &params.text_document.uri;
        let text = &params.text_document.text;

        if let Some(slug) = Self::slug_from_uri(uri) {
            let _ = self.refresh_cache(&slug).await;
        }

        let diags = self.diagnostics_for_doc(uri, text).await;
        self.client
            .publish_diagnostics(uri.clone(), diags, None)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = &params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().next() {
            let diags = self.diagnostics_for_doc(uri, &change.text).await;
            self.client
                .publish_diagnostics(uri.clone(), diags, None)
                .await;
        }
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let Some(slug) = Self::slug_from_uri(uri) else {
            return Ok(None);
        };

        if self.cache.read().await.files.is_empty() {
            let _ = self.refresh_cache(&slug).await;
        }

        let cache = self.cache.read().await;
        let items: Vec<CompletionItem> = cache
            .files
            .iter()
            .filter(|f| f.path.ends_with(".md"))
            .map(|f| {
                let label = f.path.trim_end_matches(".md");
                CompletionItem {
                    label: label.to_owned(),
                    kind: Some(CompletionItemKind::FILE),
                    insert_text: Some(format!("{label}]]")),
                    detail: Some(f.path.clone()),
                    ..Default::default()
                }
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let _ = params;
        // Hover shows frontmatter for the entry under cursor — implementation
        // requires document text which is not cached here yet. Return None for now.
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let _ = params;
        Ok(None)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_wikilinks_basic() {
        let text = "See [[notes/foo]] and [[projects/bar.md|Bar project]] for details.";
        let links = extract_wikilinks(text);
        assert_eq!(links, vec!["notes/foo", "projects/bar.md"]);
    }

    #[test]
    fn extract_wikilinks_empty() {
        assert!(extract_wikilinks("No links here").is_empty());
    }

    #[test]
    fn extract_wikilinks_multiple_on_same_line() {
        let text = "[[a]] and [[b]] and [[c]]";
        let links = extract_wikilinks(text);
        assert_eq!(links, vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_wikilinks_strips_pipe_label() {
        let links = extract_wikilinks("[[CO-21|Title of Entry]]");
        assert_eq!(links, vec!["CO-21"]);
    }

    #[test]
    fn is_wikilink_trigger_true() {
        assert!(is_wikilink_trigger("See [[", 6));
    }

    #[test]
    fn is_wikilink_trigger_false_single_bracket() {
        assert!(!is_wikilink_trigger("See [", 5));
    }

    #[test]
    fn is_wikilink_trigger_false_offset_too_small() {
        assert!(!is_wikilink_trigger("[[", 1));
    }
}
