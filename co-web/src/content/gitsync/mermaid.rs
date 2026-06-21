//! CO-89: server-side Mermaid generation (validates CO-83).
//!
//! Two diagrams are produced from content entries:
//! * a **gantt** of in-flight `task` entries (swimlane by assignee), and
//! * a **timeline** of recent `commit` entries grouped by day.
//!
//! Both are pure string builders over [`EntryDomain`]s, so they're fully
//! unit-testable. The client renders the returned Mermaid source via CO-83.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::domain::EntryDomain;

/// Strip characters that would break a Mermaid task/section label (`:`, `#`,
/// newlines) and collapse whitespace.
fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| match c {
            ':' | '#' | ';' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract a `YYYY-MM-DD` date from an ISO-ish string (takes the first 10 chars
/// when they look like a date).
fn date_part(s: &str) -> Option<String> {
    if s.len() >= 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        Some(s[..10].to_string())
    } else {
        None
    }
}

fn fm_str<'a>(e: &'a EntryDomain, key: &str) -> Option<&'a str> {
    e.frontmatter.get(key).and_then(Value::as_str)
}

/// A short, Mermaid-safe identifier for a task (alphanumeric, lowercased).
fn task_id(e: &EntryDomain, index: usize) -> String {
    let raw = e
        .frontmatter
        .get("id")
        .and_then(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        })
        .or_else(|| fm_str(e, "parent_task").map(String::from))
        .unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        format!("t{index}")
    } else {
        format!("t{cleaned}")
    }
}

/// Build a Mermaid `gantt` of in-flight tasks, one `section` per assignee.
///
/// `start` comes from frontmatter `created`/`date_start` (falling back to the
/// entry's `created_at`); `end` from `due`/`date_end`. Status maps to Mermaid
/// tags: `done` → `done`, `doing` → `active`, anything else → untagged. Tasks
/// without a resolvable start date are skipped (a gantt bar needs a start).
pub fn tasks_to_gantt(tasks: &[EntryDomain], title: &str) -> String {
    let mut out = String::from("gantt\n");
    out.push_str(&format!("    title {}\n", sanitize(title)));
    out.push_str("    dateFormat YYYY-MM-DD\n");

    // Group by assignee, preserving deterministic (sorted) lane order.
    let mut lanes: BTreeMap<String, Vec<(usize, &EntryDomain)>> = BTreeMap::new();
    for (i, t) in tasks.iter().enumerate() {
        let assignee = fm_str(t, "assignee")
            .filter(|s| !s.is_empty())
            .unwrap_or("Unassigned")
            .to_string();
        lanes.entry(assignee).or_default().push((i, t));
    }

    for (assignee, items) in &lanes {
        let mut lane_lines = Vec::new();
        for (i, t) in items {
            let start = fm_str(t, "created")
                .or_else(|| fm_str(t, "date_start"))
                .or(t.created_at.as_deref())
                .and_then(date_part);
            let Some(start) = start else { continue };

            let end = fm_str(t, "due")
                .or_else(|| fm_str(t, "date_end"))
                .and_then(date_part);

            let status = fm_str(t, "status").unwrap_or("");
            let tag = match status {
                "done" => "done, ",
                "doing" => "active, ",
                _ => "",
            };
            let label = sanitize(t.title_or_path());
            let id = task_id(t, *i);
            let span = match end {
                Some(e) => format!("{start}, {e}"),
                None => format!("{start}, 1d"),
            };
            lane_lines.push(format!("    {label} :{tag}{id}, {span}\n"));
        }
        if !lane_lines.is_empty() {
            out.push_str(&format!("    section {}\n", sanitize(assignee)));
            for line in lane_lines {
                out.push_str(&line);
            }
        }
    }
    out
}

/// Build a Mermaid `timeline` of recent commits grouped by calendar day
/// (author-local date). Newest day first. Each day lists its commit subjects.
pub fn commits_to_timeline(commits: &[EntryDomain], title: &str) -> String {
    let mut out = String::from("timeline\n");
    out.push_str(&format!("    title {}\n", sanitize(title)));

    // day -> subjects, sorted descending by day (newest first).
    let mut by_day: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in commits {
        let day = fm_str(c, "committed_at")
            .and_then(date_part)
            .or_else(|| c.created_at.as_deref().and_then(date_part));
        let Some(day) = day else { continue };
        by_day
            .entry(day)
            .or_default()
            .push(sanitize(c.title_or_path()));
    }

    for (day, subjects) in by_day.iter().rev() {
        let joined = subjects.join(" : ");
        out.push_str(&format!("    {day} : {joined}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(
        id: i64,
        title: &str,
        status: &str,
        assignee: &str,
        created: &str,
        due: Option<&str>,
    ) -> EntryDomain {
        let mut fm = json!({
            "type": "task",
            "id": id,
            "title": title,
            "status": status,
            "assignee": assignee,
            "created": created,
        });
        if let Some(d) = due {
            fm.as_object_mut().unwrap().insert("due".into(), json!(d));
        }
        EntryDomain {
            path: format!("tasks/{id}.md"),
            universe_key: "co-dev".into(),
            entry_type: "task".into(),
            title: Some(title.into()),
            frontmatter: fm,
            body: String::new(),
            body_hash: String::new(),
            created_at: Some(created.into()),
            updated_at: Some(created.into()),
        }
    }

    #[test]
    fn gantt_groups_by_assignee_with_status_tags() {
        let tasks = vec![
            task(
                65,
                "CO-65 visibility",
                "done",
                "yuri",
                "2026-04-26",
                Some("2026-04-27"),
            ),
            task(
                85,
                "CO-85 password-login",
                "doing",
                "yuri",
                "2026-04-27",
                Some("2026-04-29"),
            ),
            task(99, "CO-99 backlog item", "todo", "ana", "2026-04-27", None),
        ];
        let g = tasks_to_gantt(&tasks, "Active sprint");
        assert!(g.starts_with("gantt\n"));
        assert!(g.contains("title Active sprint"));
        assert!(g.contains("dateFormat YYYY-MM-DD"));
        // sections sorted: "ana" before "yuri"
        let ana_pos = g.find("section ana").unwrap();
        let yuri_pos = g.find("section yuri").unwrap();
        assert!(ana_pos < yuri_pos);
        assert!(g.contains("CO-65 visibility :done, t65, 2026-04-26, 2026-04-27"));
        assert!(g.contains("CO-85 password-login :active, t85, 2026-04-27, 2026-04-29"));
        // No due ⇒ 1d duration.
        assert!(g.contains("CO-99 backlog item :t99, 2026-04-27, 1d"));
    }

    #[test]
    fn gantt_skips_tasks_without_start_date() {
        let mut t = task(1, "no date", "todo", "yuri", "", None);
        t.created_at = None;
        t.frontmatter.as_object_mut().unwrap().remove("created");
        let g = tasks_to_gantt(&[t], "x");
        assert!(!g.contains("section")); // nothing renders
    }

    #[test]
    fn sanitize_strips_colons() {
        let t = task(
            1,
            "feat: a thing",
            "done",
            "yuri",
            "2026-04-26",
            Some("2026-04-27"),
        );
        let g = tasks_to_gantt(&[t], "T");
        // The colon in the title must be gone (only the mermaid field colon remains).
        assert!(g.contains("feat a thing :done"));
    }

    #[test]
    fn timeline_groups_by_day_newest_first() {
        let c1 = {
            let mut e = task(0, "feat: CO-1 x", "", "", "2026-04-26", None);
            e.entry_type = "commit".into();
            e.frontmatter = json!({"type":"commit","title":"feat: CO-1 x","committed_at":"2026-04-26T10:00:00-03:00"});
            e
        };
        let c2 = {
            let mut e = task(0, "fix: CO-2 y", "", "", "2026-04-27", None);
            e.entry_type = "commit".into();
            e.frontmatter = json!({"type":"commit","title":"fix: CO-2 y","committed_at":"2026-04-27T10:00:00-03:00"});
            e
        };
        let tl = commits_to_timeline(&[c1, c2], "Recent commits");
        assert!(tl.starts_with("timeline\n"));
        let pos_27 = tl.find("2026-04-27").unwrap();
        let pos_26 = tl.find("2026-04-26").unwrap();
        assert!(pos_27 < pos_26, "newest day first");
    }
}
