//! Claude Code usage telemetry capture (CO-425).
//!
//! When co-auto runs `claude -p` headless, it can request
//! `--output-format stream-json --verbose`, which emits NDJSON: one JSON object
//! per line. Each `assistant` message carries a `message.usage` block
//! (`input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
//! `cache_read_input_tokens`, `model`), and a final `result` event carries
//! totals (`num_turns`, `duration_ms`, `total_cost_usd`).
//!
//! [`parse_stream_json`] aggregates those lines into a [`SessionUsage`] summary
//! that co-auto POSTs to the CO ingestion endpoint (CO-426). Everything here is
//! **best-effort**: a parse error on any single line is skipped, never
//! propagated; missing fields default to zero / `None`. Telemetry must never
//! fail or block a co-auto task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Aggregated token usage for one Claude Code session.
///
/// Serializable so co-auto can POST it to the CO ingestion endpoint. Token
/// fields are summed across every per-message `usage` block; `models` lists the
/// distinct models seen (usually one). `num_turns`, `duration_ms` and
/// `total_cost_usd` come from the final `result` event when present.
///
/// CO-437 enrichment: `tool_uses` (a name→count map harvested from `tool_use`
/// content blocks), `output_chars` (the total length of assistant text emitted)
/// and `pr_url` (the PR link co-auto opened, set by the caller from stdout —
/// never present in the stream itself). All best-effort / default-empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    /// Sum of `input_tokens` across all assistant messages.
    pub input_tokens: i64,
    /// Sum of `output_tokens` across all assistant messages.
    pub output_tokens: i64,
    /// Sum of `cache_creation_input_tokens` (cache writes).
    pub cache_creation_input_tokens: i64,
    /// Sum of `cache_read_input_tokens` (cache reads).
    pub cache_read_input_tokens: i64,
    /// Distinct models observed, in first-seen order (usually one).
    pub models: Vec<String>,
    /// CO-437: tool name → invocation count, from `tool_use` content blocks.
    /// `BTreeMap` so serialization is deterministic. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_uses: BTreeMap<String, i64>,
    /// CO-437: total characters of assistant text emitted across the session
    /// (a cheap proxy for output size when output tokens aren't enough).
    #[serde(default)]
    pub output_chars: i64,
    /// CO-437: the PR link co-auto opened, set by the caller from co-auto's
    /// stdout (not part of the stream-json). `None` when no PR was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Number of agent turns, from the `result` event (None if absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_turns: Option<i64>,
    /// Wall-clock duration in ms, from the `result` event (None if absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Subscription-aware cost: only set when the `result` event reports
    /// `total_cost_usd` (keychain/subscription auth reports no USD).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
}

impl SessionUsage {
    /// Total input-side tokens including both cache reads and writes.
    pub fn total_input(&self) -> i64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }

    /// Fraction of input tokens served from cache (0.0..=1.0). Returns 0.0 when
    /// there were no input tokens at all (avoids divide-by-zero).
    pub fn cached_fraction(&self) -> f64 {
        let total = self.total_input();
        if total == 0 {
            0.0
        } else {
            self.cache_read_input_tokens as f64 / total as f64
        }
    }

    /// The primary (first-seen) model, short-name only (e.g. `sonnet` from
    /// `claude-sonnet-4-5-20250929`). Returns `"?"` when no model was seen.
    pub fn primary_model_short(&self) -> String {
        match self.models.first() {
            Some(m) => short_model_name(m),
            None => "?".to_string(),
        }
    }

    /// A one-line human summary, e.g.
    /// `usage: 12.3k in (84% cached) / 4.1k out — sonnet`.
    pub fn summary_line(&self) -> String {
        format!(
            "usage: {} in ({:.0}% cached) / {} out — {}",
            human_tokens(self.total_input()),
            self.cached_fraction() * 100.0,
            human_tokens(self.output_tokens),
            self.primary_model_short(),
        )
    }
}

/// Shorten a full model id to its family name (`claude-sonnet-4-5-…` →
/// `sonnet`). Falls back to the input when no known family substring matches.
fn short_model_name(model: &str) -> String {
    for family in ["opus", "sonnet", "haiku", "fable", "mythos"] {
        if model.contains(family) {
            return family.to_string();
        }
    }
    model.to_string()
}

/// Format a token count compactly: `12300` → `12.3k`, `900` → `900`.
fn human_tokens(n: i64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Parse Claude Code `--output-format stream-json` NDJSON into a
/// [`SessionUsage`].
///
/// Best-effort by construction: each line is parsed independently; a line that
/// is not valid JSON, or that lacks a `usage` block, is simply skipped. The
/// `result` event (`type == "result"`) contributes `num_turns`, `duration_ms`
/// and `total_cost_usd` when present. Never panics, never returns an error —
/// the worst case is a zero-valued [`SessionUsage`].
pub fn parse_stream_json(ndjson: &str) -> SessionUsage {
    let mut usage = SessionUsage::default();

    for line in ndjson.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // not JSON (or a partial line) — skip, never fail
        };

        // Per-message usage: assistant events carry `message.usage`.
        // Some event shapes nest under `message`, others put `usage` at top
        // level — accept either.
        if let Some(u) = value
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| value.get("usage"))
        {
            usage.input_tokens += u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            usage.output_tokens += u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            usage.cache_creation_input_tokens += u
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            usage.cache_read_input_tokens += u
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
        }

        // CO-437: harvest tool usage + output size from assistant content
        // blocks. `tool_use` blocks carry a `name`; `text` blocks carry the
        // human-readable output we measure for size. Both best-effort.
        if let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for block in content {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("tool_use") => {
                        if let Some(name) = block.get("name").and_then(|v| v.as_str())
                            && !name.is_empty()
                        {
                            *usage.tool_uses.entry(name.to_string()).or_insert(0) += 1;
                        }
                    }
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            usage.output_chars += t.chars().count() as i64;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Model id: prefer `message.model`, fall back to top-level `model`.
        if let Some(model) = value
            .get("message")
            .and_then(|m| m.get("model"))
            .or_else(|| value.get("model"))
            .and_then(|v| v.as_str())
            && !model.is_empty()
            && !usage.models.iter().any(|m| m == model)
        {
            usage.models.push(model.to_string());
        }

        // Final `result` event — totals.
        if value.get("type").and_then(|v| v.as_str()) == Some("result") {
            if let Some(n) = value.get("num_turns").and_then(|v| v.as_i64()) {
                usage.num_turns = Some(n);
            }
            if let Some(d) = value.get("duration_ms").and_then(|v| v.as_i64()) {
                usage.duration_ms = Some(d);
            }
            if let Some(c) = value.get("total_cost_usd").and_then(|v| v.as_f64()) {
                usage.total_cost_usd = Some(c);
            }
        }
    }

    usage
}

/// Extract the human-readable assistant text from a single stream-json line, if
/// the line is an assistant message with text content. Returns `None` for any
/// other event type or unparseable line. Used to re-emit the "human" stdout to
/// the launcher log while the structured events are consumed for telemetry.
pub fn assistant_text(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(|v| v.as_str()) != Some("assistant") {
        return None;
    }
    let content = value.get("message")?.get("content")?.as_array()?;
    let mut out = String::new();
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) == Some("text")
            && let Some(t) = block.get("text").and_then(|v| v.as_str())
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(t);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative stream-json transcript: an init line, two assistant
    /// messages (each with a `usage` block), and a final `result` event.
    const FIXTURE: &str = r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-5-20250929"}
{"type":"assistant","message":{"model":"claude-sonnet-4-5-20250929","content":[{"type":"text","text":"Reading the file."}],"usage":{"input_tokens":1200,"output_tokens":340,"cache_creation_input_tokens":500,"cache_read_input_tokens":8000}}}
{"type":"assistant","message":{"model":"claude-sonnet-4-5-20250929","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":300,"output_tokens":120,"cache_creation_input_tokens":0,"cache_read_input_tokens":9000}}}
{"type":"result","subtype":"success","num_turns":4,"duration_ms":372000,"total_cost_usd":0.0421}"#;

    #[test]
    fn parses_and_aggregates_usage_events() {
        let u = parse_stream_json(FIXTURE);
        assert_eq!(u.input_tokens, 1500);
        assert_eq!(u.output_tokens, 460);
        assert_eq!(u.cache_creation_input_tokens, 500);
        assert_eq!(u.cache_read_input_tokens, 17000);
        assert_eq!(u.models, vec!["claude-sonnet-4-5-20250929".to_string()]);
        assert_eq!(u.num_turns, Some(4));
        assert_eq!(u.duration_ms, Some(372000));
        assert_eq!(u.total_cost_usd, Some(0.0421));
    }

    #[test]
    fn captures_tool_uses_and_output_chars() {
        // Two assistant turns: one with two tool_use blocks (Read, Read, Edit),
        // both with text blocks. "Reading the file." = 17 chars + "Done." = 5.
        let stream = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"text","text":"Reading the file."},{"type":"tool_use","name":"Read"},{"type":"tool_use","name":"Read"}]}}
{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","name":"Edit"},{"type":"text","text":"Done."}]}}"#;
        let u = parse_stream_json(stream);
        assert_eq!(u.tool_uses.get("Read"), Some(&2));
        assert_eq!(u.tool_uses.get("Edit"), Some(&1));
        assert_eq!(u.output_chars, 17 + 5);
        // pr_url is never derived from the stream — caller sets it.
        assert_eq!(u.pr_url, None);
    }

    #[test]
    fn fixture_counts_output_chars_with_no_tools() {
        let u = parse_stream_json(FIXTURE);
        // "Reading the file." (17) + "Done." (5) = 22; no tool_use blocks.
        assert_eq!(u.output_chars, 22);
        assert!(u.tool_uses.is_empty());
    }

    #[test]
    fn enrichment_fields_serialize_only_when_present() {
        let mut u = SessionUsage {
            input_tokens: 10,
            ..Default::default()
        };
        // Empty tool_uses + None pr_url are omitted; output_chars always present.
        let json = serde_json::to_string(&u).unwrap();
        assert!(!json.contains("tool_uses"), "got: {json}");
        assert!(!json.contains("pr_url"), "got: {json}");
        assert!(json.contains("output_chars"), "got: {json}");

        u.tool_uses.insert("Bash".into(), 3);
        u.pr_url = Some("https://github.com/artelonga/co/pull/9".into());
        let json = serde_json::to_string(&u).unwrap();
        assert!(json.contains("tool_uses"), "got: {json}");
        assert!(json.contains("Bash"), "got: {json}");
        assert!(json.contains("/pull/9"), "got: {json}");
    }

    #[test]
    fn skips_unparseable_and_partial_lines_without_failing() {
        let mixed = "not json at all\n\
                     {\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\
                     {partial line with no close\n\
                     \n";
        let u = parse_stream_json(mixed);
        // Only the one valid usage line counts; the rest are silently skipped.
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 5);
    }

    #[test]
    fn empty_input_yields_zeroed_usage() {
        let u = parse_stream_json("");
        assert_eq!(u, SessionUsage::default());
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.num_turns, None);
        assert_eq!(u.total_cost_usd, None);
    }

    #[test]
    fn accepts_top_level_usage_shape() {
        // Some event shapes put `usage`/`model` at the top level.
        let line = r#"{"type":"assistant","model":"claude-haiku-4-5","usage":{"input_tokens":50,"output_tokens":20}}"#;
        let u = parse_stream_json(line);
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 20);
        assert_eq!(u.models, vec!["claude-haiku-4-5".to_string()]);
    }

    #[test]
    fn dedupes_repeated_model_ids() {
        let u = parse_stream_json(FIXTURE);
        assert_eq!(u.models.len(), 1, "same model seen twice → one entry");
    }

    #[test]
    fn summary_line_is_human_readable() {
        let u = parse_stream_json(FIXTURE);
        let line = u.summary_line();
        // total input = 1500 + 500 + 17000 = 19000 → "19.0k in"
        assert!(line.contains("19.0k in"), "got: {line}");
        assert!(line.contains("out"), "got: {line}");
        assert!(line.contains("sonnet"), "got: {line}");
        // cached = 17000 / 19000 ≈ 89%
        assert!(line.contains("89% cached"), "got: {line}");
    }

    #[test]
    fn cached_fraction_handles_zero_input() {
        let u = SessionUsage::default();
        assert_eq!(u.cached_fraction(), 0.0);
        assert_eq!(u.primary_model_short(), "?");
    }

    #[test]
    fn short_model_name_extracts_family() {
        assert_eq!(short_model_name("claude-opus-4-8-20251101"), "opus");
        assert_eq!(short_model_name("claude-sonnet-4-5"), "sonnet");
        assert_eq!(short_model_name("claude-haiku-4-5"), "haiku");
        assert_eq!(short_model_name("some-fable-model"), "fable");
        assert_eq!(short_model_name("unknown-x"), "unknown-x");
    }

    #[test]
    fn assistant_text_extracts_only_text_blocks() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"},{"type":"tool_use","name":"Read"}]}}"#;
        assert_eq!(assistant_text(line).as_deref(), Some("hello"));
        // Non-assistant events yield None.
        assert_eq!(assistant_text(r#"{"type":"result"}"#), None);
        // Unparseable yields None, never panics.
        assert_eq!(assistant_text("garbage"), None);
    }

    #[test]
    fn session_usage_serializes_with_optional_fields_skipped() {
        let u = SessionUsage {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        };
        let json = serde_json::to_string(&u).unwrap();
        // num_turns / duration_ms / total_cost_usd are None → omitted.
        assert!(!json.contains("num_turns"), "got: {json}");
        assert!(!json.contains("total_cost_usd"), "got: {json}");
        assert!(json.contains("input_tokens"), "got: {json}");
    }
}
