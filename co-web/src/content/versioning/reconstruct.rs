//! CO-75: version reconstruction — replay the op log / version history to any
//! past instant, diff two instants, and auto-generate a changelog.
//!
//! **Reuse, not duplication** (per the CO-75 pre-flight note): this builds on
//! the two pieces that already exist on `main` instead of introducing a parallel
//! `entry_snapshots` table:
//!
//! * **CO-54 `entry_versions`** (`crate::storage::entry_versions`) — the
//!   pre-overwrite snapshot store. Each row holds the *previous* `(body,
//!   frontmatter)` of an entry, stamped with the RFC3339 `timestamp` of the
//!   moment it was overwritten. So a row is the content that was *live until*
//!   its timestamp. The state at instant `T` is therefore the content of the
//!   **earliest version whose `timestamp > T`**; if no such version exists,
//!   nothing has changed since `T` and the current live entry is the answer.
//!   This module's [`reconstruct_at`] / [`diff_frontmatter`] run on those rows.
//! * **`entry_events`** (the CO-95 op log) — the append-only `(op, path,
//!   ts_micros, prev_body_hash, frontmatter_json)` log. [`build_changelog`]
//!   aggregates these ops into a Keep-a-Changelog document, classifying each op
//!   (`put` with no `prev_body_hash` → Added, `put` with one → Changed,
//!   `delete` → Removed) and rendering its line via the content type's
//!   manifest-declared `changelog_summary` template.
//!
//! Everything here is pure (no axum / no DB) so it is exhaustively unit-tested;
//! the HTTP/DB glue lives in `content::entries::routes` alongside the sibling
//! `history` / `versions` handlers.

use chrono::{DateTime, TimeZone, Utc};
use co::manifest::Manifest;
use serde::Serialize;
use serde_json::{Map, Value as JsonValue};
use std::collections::BTreeSet;

use crate::storage::entry_versions::EntryVersionRow;

/// The reconstructed content of an entry at a past instant.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconstructedState {
    pub frontmatter: JsonValue,
    pub body: String,
    /// Version number that supplied this state, or `None` when it is the
    /// current live state (no later version recorded the change).
    pub version: Option<i64>,
    /// RFC3339 timestamp of the snapshot used, or `None` when falling back to
    /// the live entry.
    pub source_timestamp: Option<String>,
    /// `true` when the reconstruction is the current live entry.
    pub is_current: bool,
}

/// Reconstruct an entry's content as it was at `as_of`.
///
/// `versions` is the entry's history **newest-first** (as returned by
/// [`crate::storage::Storage::list_entry_versions`]); `current_frontmatter` /
/// `current_body` are the present on-disk state. See the module docs for the
/// "earliest version with `timestamp > as_of`" rule.
pub fn reconstruct_at(
    versions: &[EntryVersionRow],
    current_frontmatter: &JsonValue,
    current_body: &str,
    as_of: DateTime<Utc>,
) -> ReconstructedState {
    // Walk oldest-first so the first row whose snapshot was taken *after*
    // `as_of` is the earliest such row — its content was live at `as_of`.
    let chosen = versions.iter().rev().find(|v| {
        DateTime::parse_from_rfc3339(&v.timestamp)
            .map(|ts| ts.with_timezone(&Utc) > as_of)
            .unwrap_or(false)
    });

    match chosen {
        Some(v) => ReconstructedState {
            frontmatter: serde_json::from_str(&v.frontmatter_json).unwrap_or(JsonValue::Null),
            body: v.body.clone(),
            version: Some(v.version),
            source_timestamp: Some(v.timestamp.clone()),
            is_current: false,
        },
        None => ReconstructedState {
            frontmatter: current_frontmatter.clone(),
            body: current_body.to_string(),
            version: None,
            source_timestamp: None,
            is_current: true,
        },
    }
}

/// A single frontmatter field that differs between two instants.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldChange {
    pub field: String,
    pub before: Option<JsonValue>,
    pub after: Option<JsonValue>,
}

/// Field-level frontmatter diff: every key whose value differs (added, removed,
/// or changed), sorted for deterministic output.
pub fn diff_frontmatter(before: &JsonValue, after: &JsonValue) -> Vec<FieldChange> {
    let empty = Map::new();
    let b = before.as_object().unwrap_or(&empty);
    let a = after.as_object().unwrap_or(&empty);

    let keys: BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    keys.into_iter()
        .filter_map(|k| {
            let bv = b.get(k);
            let av = a.get(k);
            (bv != av).then(|| FieldChange {
                field: k.clone(),
                before: bv.cloned(),
                after: av.cloned(),
            })
        })
        .collect()
}

/// Op-level diff of one entry between two reconstructed instants. Shows only the
/// net change across the interval (CO-75 acceptance: "only changes in that
/// interval").
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EntryDiff {
    pub path: String,
    pub from: String,
    pub to: String,
    pub field_changes: Vec<FieldChange>,
    pub body_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_after: Option<String>,
}

/// Build the diff between two already-reconstructed states.
pub fn diff_states(
    path: &str,
    from: &str,
    to: &str,
    before: &ReconstructedState,
    after: &ReconstructedState,
) -> EntryDiff {
    let body_changed = before.body != after.body;
    EntryDiff {
        path: path.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        field_changes: diff_frontmatter(&before.frontmatter, &after.frontmatter),
        body_changed,
        body_before: body_changed.then(|| before.body.clone()),
        body_after: body_changed.then(|| after.body.clone()),
    }
}

// ---------------------------------------------------------------------------
// Auto-changelog (aggregates `entry_events` ops)
// ---------------------------------------------------------------------------

/// One op from the `entry_events` log, projected to what the changelog needs.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub path: String,
    /// `"put"` or `"delete"`.
    pub op: String,
    pub ts_micros: i64,
    pub prev_body_hash: Option<String>,
    pub frontmatter_json: Option<String>,
}

/// One rendered changelog line.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChangelogLine {
    pub path: String,
    pub timestamp: String,
    pub summary: String,
}

/// A generated changelog over a time window, in Keep-a-Changelog shape.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Changelog {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    pub added: Vec<ChangelogLine>,
    pub changed: Vec<ChangelogLine>,
    pub removed: Vec<ChangelogLine>,
    pub total: usize,
    /// Rendered Keep-a-Changelog markdown (empty sections omitted).
    pub markdown: String,
}

/// Classify + render a batch of ops (oldest-first) into a [`Changelog`].
///
/// `manifest` supplies per-content-type `changelog_summary` templates; when a
/// type declares none (or there is no manifest) a sensible default line (the
/// entry title or path) is used.
pub fn build_changelog(
    events: &[ChangeEvent],
    manifest: Option<&Manifest>,
    since: Option<String>,
    until: Option<String>,
) -> Changelog {
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut removed = Vec::new();

    for ev in events {
        let fm = ev
            .frontmatter_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<JsonValue>(s).ok())
            .unwrap_or(JsonValue::Null);
        let summary = render_summary(manifest, &fm, &ev.path);
        let line = ChangelogLine {
            path: ev.path.clone(),
            timestamp: micros_to_rfc3339(ev.ts_micros),
            summary,
        };
        match ev.op.as_str() {
            "delete" => removed.push(line),
            // A `put` with no prior body is the first time we saw the entry.
            "put" if ev.prev_body_hash.is_none() => added.push(line),
            _ => changed.push(line),
        }
    }

    let total = added.len() + changed.len() + removed.len();
    let markdown = render_markdown(
        since.as_deref(),
        until.as_deref(),
        &added,
        &changed,
        &removed,
    );
    Changelog {
        since,
        until,
        added,
        changed,
        removed,
        total,
        markdown,
    }
}

/// Resolve a content type's `changelog_summary` template from the manifest and
/// render it; fall back to the entry title or path when none is declared.
fn render_summary(manifest: Option<&Manifest>, frontmatter: &JsonValue, path: &str) -> String {
    let entry_type = frontmatter.get("type").and_then(|v| v.as_str());
    let template = manifest.zip(entry_type).and_then(|(m, t)| {
        m.content_types
            .iter()
            .find(|ct| ct.name == t)
            .and_then(|ct| ct.changelog_summary.clone())
    });
    match template {
        Some(t) => render_template(&t, frontmatter, path),
        None => title_or_path(frontmatter, path),
    }
}

/// Substitute `{field}` tokens in a template with frontmatter values.
///
/// `{path}` resolves to the entry path; `{title}` falls back to the path when
/// absent; any other unknown token resolves to an empty string. Values are
/// rendered as plain text (strings without surrounding quotes).
fn render_template(template: &str, frontmatter: &JsonValue, path: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        if let Some(close) = rest[open..].find('}') {
            let key = &rest[open + 1..open + close];
            out.push_str(&resolve_token(key, frontmatter, path));
            rest = &rest[open + close + 1..];
        } else {
            // Unbalanced brace — emit the remainder verbatim.
            out.push_str(&rest[open..]);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn resolve_token(key: &str, frontmatter: &JsonValue, path: &str) -> String {
    match key {
        "path" => path.to_string(),
        "title" => frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string()),
        other => frontmatter
            .get(other)
            .map(json_to_plain)
            .unwrap_or_default(),
    }
}

/// Render a JSON scalar as plain text (no surrounding quotes for strings).
fn json_to_plain(v: &JsonValue) -> String {
    match v {
        JsonValue::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn title_or_path(frontmatter: &JsonValue, path: &str) -> String {
    frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

fn micros_to_rfc3339(ts_micros: i64) -> String {
    Utc.timestamp_micros(ts_micros)
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn render_markdown(
    since: Option<&str>,
    until: Option<&str>,
    added: &[ChangelogLine],
    changed: &[ChangelogLine],
    removed: &[ChangelogLine],
) -> String {
    let mut md = String::new();
    let header = match (since, until) {
        (Some(s), Some(u)) => format!("## [{s} – {u}]\n"),
        (Some(s), None) => format!("## [{s} – ]\n"),
        (None, Some(u)) => format!("## [ – {u}]\n"),
        (None, None) => "## [Unreleased]\n".to_string(),
    };
    md.push_str(&header);

    for (title, lines) in [("Added", added), ("Changed", changed), ("Removed", removed)] {
        if lines.is_empty() {
            continue;
        }
        md.push_str(&format!("\n### {title}\n\n"));
        for l in lines {
            md.push_str(&format!("- {} ({})\n", l.summary, l.path));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn version(v: i64, ts: &str, body: &str, fm: JsonValue) -> EntryVersionRow {
        EntryVersionRow {
            id: v,
            universe_key: "u".into(),
            path: "p.md".into(),
            version: v,
            body: body.into(),
            frontmatter_json: fm.to_string(),
            hash: format!("h{v}"),
            actor: None,
            timestamp: ts.into(),
        }
    }

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // --- reconstruct_at -----------------------------------------------------

    #[test]
    fn reconstruct_returns_current_when_no_later_version() {
        // No history at all → always current.
        let s = reconstruct_at(
            &[],
            &json!({"status": "done"}),
            "now",
            t("2026-01-01T00:00:00Z"),
        );
        assert!(s.is_current);
        assert_eq!(s.body, "now");
        assert_eq!(s.version, None);
    }

    #[test]
    fn reconstruct_picks_content_live_at_t() {
        // Timeline: created "c0"; replaced at t1 (row v1 holds c0), replaced at
        // t2 (row v2 holds c1); current = c2. Rows are newest-first.
        let versions = vec![
            version(2, "2026-03-01T00:00:00Z", "c1", json!({"status": "doing"})),
            version(1, "2026-02-01T00:00:00Z", "c0", json!({"status": "todo"})),
        ];
        let cur = json!({"status": "done"});

        // Before any change → earliest version (v1) holds the original content.
        let s = reconstruct_at(&versions, &cur, "c2", t("2026-01-15T00:00:00Z"));
        assert_eq!(s.body, "c0");
        assert_eq!(s.version, Some(1));

        // Between t1 and t2 → content live then is c1 (held by v2).
        let s = reconstruct_at(&versions, &cur, "c2", t("2026-02-15T00:00:00Z"));
        assert_eq!(s.body, "c1");
        assert_eq!(s.version, Some(2));

        // At/after the last recorded change → current live content.
        let s = reconstruct_at(&versions, &cur, "c2", t("2026-04-01T00:00:00Z"));
        assert_eq!(s.body, "c2");
        assert!(s.is_current);
    }

    #[test]
    fn reconstruct_boundary_is_exclusive_on_lower_edge() {
        // Exactly at t2: the change at t2 has taken effect, so state is current.
        let versions = vec![version(1, "2026-02-01T00:00:00Z", "old", json!({}))];
        let s = reconstruct_at(&versions, &json!({}), "new", t("2026-02-01T00:00:00Z"));
        assert!(s.is_current);
        assert_eq!(s.body, "new");
    }

    #[test]
    fn reconstruct_handles_offset_timestamps() {
        // Query in a non-UTC offset must compare correctly against UTC rows.
        let versions = vec![version(1, "2026-02-01T12:00:00Z", "old", json!({}))];
        // 2026-02-01T08:00:00-05:00 == 13:00:00Z, which is AFTER the row → current.
        let s = reconstruct_at(&versions, &json!({}), "new", t("2026-02-01T08:00:00-05:00"));
        assert!(s.is_current);
    }

    // --- diff ---------------------------------------------------------------

    #[test]
    fn diff_reports_only_changed_fields_and_body() {
        let before = ReconstructedState {
            frontmatter: json!({"status": "todo", "title": "T", "owner": "a"}),
            body: "b0".into(),
            version: Some(1),
            source_timestamp: None,
            is_current: false,
        };
        let after = ReconstructedState {
            frontmatter: json!({"status": "done", "title": "T", "assignee": "b"}),
            body: "b1".into(),
            version: None,
            source_timestamp: None,
            is_current: true,
        };
        let d = diff_states("p.md", "t1", "t2", &before, &after);
        // status changed, owner removed, assignee added; title unchanged → not listed.
        let fields: Vec<&str> = d.field_changes.iter().map(|c| c.field.as_str()).collect();
        assert_eq!(fields, vec!["assignee", "owner", "status"]);
        assert!(d.body_changed);
        assert_eq!(d.body_before.as_deref(), Some("b0"));
        assert_eq!(d.body_after.as_deref(), Some("b1"));
    }

    #[test]
    fn diff_empty_when_unchanged() {
        let st = ReconstructedState {
            frontmatter: json!({"a": 1}),
            body: "same".into(),
            version: None,
            source_timestamp: None,
            is_current: true,
        };
        let d = diff_states("p.md", "t1", "t2", &st, &st);
        assert!(d.field_changes.is_empty());
        assert!(!d.body_changed);
        assert!(d.body_before.is_none());
    }

    // --- changelog ----------------------------------------------------------

    fn ev(path: &str, op: &str, ts: i64, prev: Option<&str>, fm: JsonValue) -> ChangeEvent {
        ChangeEvent {
            path: path.into(),
            op: op.into(),
            ts_micros: ts,
            prev_body_hash: prev.map(str::to_string),
            frontmatter_json: Some(fm.to_string()),
        }
    }

    #[test]
    fn changelog_classifies_added_changed_removed() {
        let events = vec![
            ev(
                "a.md",
                "put",
                1,
                None,
                json!({"type": "tarefa", "title": "A"}),
            ),
            ev(
                "a.md",
                "put",
                2,
                Some("h1"),
                json!({"type": "tarefa", "title": "A"}),
            ),
            ev(
                "b.md",
                "delete",
                3,
                Some("h2"),
                json!({"type": "tarefa", "title": "B"}),
            ),
        ];
        let cl = build_changelog(&events, None, None, None);
        assert_eq!(cl.added.len(), 1);
        assert_eq!(cl.changed.len(), 1);
        assert_eq!(cl.removed.len(), 1);
        assert_eq!(cl.total, 3);
        assert_eq!(cl.added[0].summary, "A"); // default = title
    }

    #[test]
    fn changelog_uses_manifest_template() {
        let yaml = r#"
schema_version: 1
name: Test
content_types:
  - name: tarefa
    changelog_summary: "{title} marcado como {status}"
"#;
        let manifest = co::manifest::parse_str(yaml).unwrap().manifest;
        let events = vec![ev(
            "a.md",
            "put",
            1,
            Some("h0"),
            json!({"type": "tarefa", "title": "Lavar louça", "status": "done"}),
        )];
        let cl = build_changelog(&events, Some(&manifest), None, None);
        assert_eq!(cl.changed[0].summary, "Lavar louça marcado como done");
    }

    #[test]
    fn changelog_markdown_is_keep_a_changelog() {
        let events = vec![
            ev("a.md", "put", 1, None, json!({"title": "A"})),
            ev("b.md", "delete", 2, Some("h"), json!({"title": "B"})),
        ];
        let cl = build_changelog(
            &events,
            None,
            Some("2026-01-01T00:00:00Z".into()),
            Some("2026-06-01T00:00:00Z".into()),
        );
        assert!(cl.markdown.contains("### Added"));
        assert!(cl.markdown.contains("### Removed"));
        // No Changed ops → that section is omitted.
        assert!(!cl.markdown.contains("### Changed"));
        assert!(cl.markdown.contains("- A (a.md)"));
    }

    #[test]
    fn template_unknown_token_is_empty_and_path_resolves() {
        let fm = json!({"title": "T"});
        assert_eq!(render_template("{title}@{path}", &fm, "x.md"), "T@x.md");
        assert_eq!(render_template("[{missing}]", &fm, "x.md"), "[]");
        // title falls back to path when absent.
        assert_eq!(render_template("{title}", &json!({}), "x.md"), "x.md");
    }
}
