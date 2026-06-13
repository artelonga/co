//! CO-396: project-timeline lens — co-web's consumer of the shared layout
//! engine (`game_core::time_layout`).
//!
//! The board and the timeline are two *forms* of the same canonical entries:
//! this module maps an [`EntryRow`] (kanban row) onto the engine's neutral
//! [`TimelineEntry`], then [`layout_project_timeline`] arranges them into
//! lanes + markers + backlog. No new data structure — the same entries that
//! fill the quadro fill the timeline.

use chrono::DateTime;
use serde_json::Value as JsonValue;

use game_core::time_layout::{LaneField, ProjectTimeline, TimelineEntry, layout_project_timeline};

use crate::content::entries::index::EntryRow;

/// Frontmatter aliases for each schedule field (mirrors the JS
/// `MS_FIELD_SOURCES` in `static/shared/lib/co-time.js`).
const EVENT_KEYS: &[&str] = &["event_at", "date"];
const DUE_KEYS: &[&str] = &["due_at", "due_date"];
const SCHEDULED_KEYS: &[&str] = &["scheduled_at", "start_date"];
const DONE_KEYS: &[&str] = &["done_at", "completed_at", "closed_at"];

/// Status values (case-insensitive) that mean the work item is finished —
/// used to derive a `done_ms` from `updated_at` when no explicit done date is
/// present.
const DONE_STATUSES: &[&str] = &["done", "completed", "closed", "concluido", "concluído"];

/// Parse an RFC3339 datetime or a bare `YYYY-MM-DD` date into Unix ms.
fn iso_to_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = s.parse::<DateTime<chrono::Utc>>() {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = format!("{s}T00:00:00Z").parse::<DateTime<chrono::Utc>>() {
        return Some(dt.timestamp_millis());
    }
    None
}

/// First parseable date among `keys` in the frontmatter object. Accepts ISO
/// strings or a raw numeric ms value.
fn fm_date_ms(fm: &JsonValue, keys: &[&str]) -> Option<i64> {
    for k in keys {
        match fm.get(*k) {
            Some(JsonValue::String(s)) => {
                if let Some(ms) = iso_to_ms(s) {
                    return Some(ms);
                }
            }
            Some(JsonValue::Number(n)) => {
                if let Some(ms) = n.as_i64() {
                    return Some(ms);
                }
            }
            _ => {}
        }
    }
    None
}

fn fm_str(fm: &JsonValue, key: &str) -> Option<String> {
    fm.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Map a kanban entry row onto the engine's neutral timeline entry.
pub fn entry_to_timeline_entry(row: &EntryRow) -> TimelineEntry {
    let fm = &row.frontmatter;
    let status = fm_str(fm, "status");

    // done_ms: explicit done/completed date wins; otherwise, a done-like
    // status falls back to `updated_at` (when the row last moved to done).
    let done_ms = fm_date_ms(fm, DONE_KEYS).or_else(|| {
        let is_done = status
            .as_deref()
            .map(|s| DONE_STATUSES.iter().any(|d| s.eq_ignore_ascii_case(d)))
            .unwrap_or(false);
        if is_done {
            row.updated_at.as_deref().and_then(iso_to_ms)
        } else {
            None
        }
    });

    TimelineEntry {
        id: row.path.clone(),
        title: row.title.clone().unwrap_or_else(|| row.path.clone()),
        entry_type: row.entry_type.clone(),
        epic: fm_str(fm, "epic"),
        module: fm_str(fm, "module"),
        status,
        created_ms: row
            .created_at
            .as_deref()
            .and_then(iso_to_ms)
            .or_else(|| fm_date_ms(fm, &["created", "created_at"])),
        scheduled_ms: fm_date_ms(fm, SCHEDULED_KEYS),
        due_ms: fm_date_ms(fm, DUE_KEYS),
        event_ms: fm_date_ms(fm, EVENT_KEYS),
        done_ms,
    }
}

/// Build the full project timeline for a set of entry rows.
pub fn build_project_timeline(rows: &[EntryRow], group_by: LaneField) -> ProjectTimeline {
    let entries: Vec<TimelineEntry> = rows.iter().map(entry_to_timeline_entry).collect();
    layout_project_timeline(&entries, group_by)
}

/// Parse the `group_by` query value into a [`LaneField`], defaulting to
/// `Status` (the kanban-native grouping) for absent/unknown values.
pub fn parse_lane_field(raw: Option<&str>) -> LaneField {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("epic") => LaneField::Epic,
        Some("module") => LaneField::Module,
        _ => LaneField::Status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(path: &str, entry_type: &str, fm: JsonValue) -> EntryRow {
        EntryRow {
            path: path.into(),
            universe_key: "u".into(),
            entry_type: entry_type.into(),
            title: Some(path.into()),
            frontmatter: fm,
            body: String::new(),
            body_hash: String::new(),
            created_at: None,
            updated_at: None,
            _score: None,
        }
    }

    #[test]
    fn maps_iso_dates_and_lane_fields() {
        let r = row(
            "tasks/1.md",
            "task",
            json!({
                "epic": "auth",
                "module": "web",
                "status": "doing",
                "scheduled_at": "2026-01-01",
                "due_at": "2026-01-10T00:00:00Z",
            }),
        );
        let te = entry_to_timeline_entry(&r);
        assert_eq!(te.epic.as_deref(), Some("auth"));
        assert_eq!(te.module.as_deref(), Some("web"));
        assert_eq!(te.status.as_deref(), Some("doing"));
        assert!(te.scheduled_ms.is_some());
        assert!(te.due_ms.is_some());
        // scheduled < due → a positive gantt span.
        let span = game_core::time_layout::gantt_span(&te).unwrap();
        assert!(span.duration_ms() > 0);
    }

    #[test]
    fn done_status_derives_done_ms_from_updated_at() {
        let mut r = row("tasks/2.md", "task", json!({"status": "done"}));
        r.created_at = Some("2026-01-01T00:00:00Z".into());
        r.updated_at = Some("2026-01-05T00:00:00Z".into());
        let te = entry_to_timeline_entry(&r);
        assert!(te.created_ms.is_some());
        assert!(te.done_ms.is_some());
        let span = game_core::time_layout::gantt_span(&te).unwrap();
        assert_eq!(span.duration_ms(), 4 * 86_400_000);
    }

    #[test]
    fn undated_entry_lands_in_backlog() {
        let rows = vec![
            row(
                "tasks/dated.md",
                "task",
                json!({"due_at": "2026-02-01", "epic": "x"}),
            ),
            row("tasks/bare.md", "task", json!({"epic": "x"})),
        ];
        let tl = build_project_timeline(&rows, LaneField::Epic);
        assert_eq!(tl.lanes.len(), 1);
        assert_eq!(tl.lanes[0].items.len(), 1);
        assert_eq!(tl.backlog.len(), 1);
        assert_eq!(tl.backlog[0].id, "tasks/bare.md");
    }

    #[test]
    fn release_entry_becomes_marker() {
        let rows = vec![row(
            "releases/v1.md",
            "release",
            json!({"event_at": "2026-06-01"}),
        )];
        let tl = build_project_timeline(&rows, LaneField::Status);
        assert_eq!(tl.markers.len(), 1);
        assert_eq!(tl.markers[0].kind, "release");
        assert!(tl.lanes.is_empty());
    }

    #[test]
    fn parse_lane_field_defaults_to_status() {
        assert_eq!(parse_lane_field(Some("epic")), LaneField::Epic);
        assert_eq!(parse_lane_field(Some("MODULE")), LaneField::Module);
        assert_eq!(parse_lane_field(Some("status")), LaneField::Status);
        assert_eq!(parse_lane_field(None), LaneField::Status);
        assert_eq!(parse_lane_field(Some("bogus")), LaneField::Status);
    }
}
