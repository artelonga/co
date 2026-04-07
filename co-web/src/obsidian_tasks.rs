//! CO-37 — Obsidian Tasks compatibility.
//!
//! Maps CO task `status` values to/from Obsidian Tasks checkbox syntax.
//!
//! | CO status   | Obsidian checkbox |
//! |-------------|-------------------|
//! | todo        | `- [ ]`           |
//! | in_progress | `- [/]`           |
//! | in_review   | `- [~]`           |
//! | done        | `- [x]`           |
//!
//! **Export** (GET /vault/\*path for task entries):
//! Body is prepended with `- [checkbox] title\n\n` when the body does not
//! already begin with a checkbox line.
//!
//! **Import** (PUT /vault/\*path):
//! If body starts with `- [c] …`, the status character is parsed and the
//! frontmatter `status` field is set (unless already present — frontmatter
//! is canonical).  The checkbox line is then stripped from the stored body so
//! it is not saved redundantly.

use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Public conversion API
// ---------------------------------------------------------------------------

/// Convert a CO task status string to an Obsidian Tasks checkbox character.
///
/// Unknown statuses fall back to `' '` (space = todo).
pub fn status_to_checkbox(status: &str) -> char {
    match status {
        "done" => 'x',
        "in_progress" => '/',
        "in_review" => '~',
        _ => ' ',
    }
}

/// Convert an Obsidian Tasks checkbox character to a CO status string.
///
/// Unknown characters fall back to `"todo"`.
pub fn checkbox_to_status(c: char) -> &'static str {
    match c {
        'x' | 'X' => "done",
        '/' => "in_progress",
        '~' => "in_review",
        _ => "todo",
    }
}

/// Return the body with an Obsidian Tasks checkbox line prepended, if
/// `entry_type` is `"task"` and the body does not already start with `- [`.
///
/// The checkbox character is derived from the `status` field in `frontmatter`.
/// Non-task entries are returned unchanged.
pub fn inject_task_checkbox(entry_type: &str, frontmatter: &JsonValue, body: &str) -> String {
    if entry_type != "task" {
        return body.to_string();
    }

    // Already has Obsidian checkbox — do not add another.
    if body.trim_start().starts_with("- [") {
        return body.to_string();
    }

    let status = frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("todo");

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");

    let checkbox = status_to_checkbox(status);
    let checkbox_line = format!("- [{checkbox}] {title}");

    if body.is_empty() {
        checkbox_line
    } else {
        format!("{checkbox_line}\n\n{body}")
    }
}

/// Parse a checkbox character from the first line of `body`.
///
/// Returns `None` when the body does not match `- [c] …`.
pub fn extract_task_status(body: &str) -> Option<&'static str> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("- [") {
        return None;
    }
    let mut chars = trimmed.chars().skip(3);
    let c = chars.next()?;
    if chars.next()? != ']' {
        return None;
    }
    Some(checkbox_to_status(c))
}

/// Apply Obsidian Tasks import semantics.
///
/// - If `body` starts with a checkbox line, the CO `status` is derived from it
///   and written to `frontmatter` (only when `status` is not already set —
///   frontmatter is the canonical source).
/// - The checkbox line is stripped from the stored body so it is not
///   persisted redundantly alongside the frontmatter `status`.
/// - Returns the updated (frontmatter, body) pair.
pub fn apply_obsidian_tasks(frontmatter: JsonValue, body: &str) -> (JsonValue, String) {
    let Some(checkbox_status) = extract_task_status(body) else {
        return (frontmatter, body.to_string());
    };

    let stripped_body = strip_checkbox_line(body);

    let mut fm = frontmatter;
    if fm.get("status").is_none()
        && let Some(obj) = fm.as_object_mut()
    {
        obj.insert(
            "status".into(),
            JsonValue::String(checkbox_status.to_string()),
        );
    }

    (fm, stripped_body)
}

/// Remove the leading `- [c] …` line (and the immediately following blank
/// line, if any) from `body`.
fn strip_checkbox_line(body: &str) -> String {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("- [") {
        return body.to_string();
    }
    let after_first_line = trimmed.find('\n').map(|i| &trimmed[i + 1..]).unwrap_or("");
    // Strip a single leading blank line that Obsidian conventionally adds.
    after_first_line
        .strip_prefix('\n')
        .unwrap_or(after_first_line)
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_to_checkbox_all_variants() {
        assert_eq!(status_to_checkbox("todo"), ' ');
        assert_eq!(status_to_checkbox("in_progress"), '/');
        assert_eq!(status_to_checkbox("in_review"), '~');
        assert_eq!(status_to_checkbox("done"), 'x');
        assert_eq!(status_to_checkbox("unknown"), ' ');
    }

    #[test]
    fn checkbox_to_status_all_variants() {
        assert_eq!(checkbox_to_status(' '), "todo");
        assert_eq!(checkbox_to_status('/'), "in_progress");
        assert_eq!(checkbox_to_status('~'), "in_review");
        assert_eq!(checkbox_to_status('x'), "done");
        assert_eq!(checkbox_to_status('X'), "done");
        assert_eq!(checkbox_to_status('-'), "todo");
    }

    #[test]
    fn inject_todo_empty_body() {
        let fm = json!({"status": "todo", "title": "Build feature"});
        assert_eq!(inject_task_checkbox("task", &fm, ""), "- [ ] Build feature");
    }

    #[test]
    fn inject_done_with_body() {
        let fm = json!({"status": "done", "title": "Ship it"});
        let result = inject_task_checkbox("task", &fm, "Some description");
        assert_eq!(result, "- [x] Ship it\n\nSome description");
    }

    #[test]
    fn inject_in_progress() {
        let fm = json!({"status": "in_progress", "title": "WIP"});
        assert_eq!(inject_task_checkbox("task", &fm, ""), "- [/] WIP");
    }

    #[test]
    fn inject_non_task_unchanged() {
        let fm = json!({"title": "A note"});
        assert_eq!(
            inject_task_checkbox("note", &fm, "Content here"),
            "Content here"
        );
    }

    #[test]
    fn inject_skips_existing_checkbox() {
        let fm = json!({"status": "todo", "title": "T"});
        let body = "- [x] T\n\nExisting body";
        assert_eq!(inject_task_checkbox("task", &fm, body), body);
    }

    #[test]
    fn extract_todo() {
        assert_eq!(extract_task_status("- [ ] Do something"), Some("todo"));
    }

    #[test]
    fn extract_done() {
        assert_eq!(extract_task_status("- [x] Finished"), Some("done"));
    }

    #[test]
    fn extract_in_progress() {
        assert_eq!(
            extract_task_status("- [/] In progress"),
            Some("in_progress")
        );
    }

    #[test]
    fn extract_no_match() {
        assert_eq!(extract_task_status("No checkbox here"), None);
        assert_eq!(extract_task_status(""), None);
    }

    #[test]
    fn apply_sets_status_from_checkbox() {
        let fm = json!({"title": "Task"});
        let body = "- [x] Task done\n\nDetails here";
        let (new_fm, new_body) = apply_obsidian_tasks(fm, body);
        assert_eq!(new_fm["status"], "done");
        assert_eq!(new_body.trim(), "Details here");
    }

    #[test]
    fn apply_preserves_frontmatter_status() {
        // Frontmatter wins over body checkbox.
        let fm = json!({"status": "in_progress", "title": "Task"});
        let body = "- [x] Task done\n\nDetails";
        let (new_fm, _) = apply_obsidian_tasks(fm, body);
        assert_eq!(new_fm["status"], "in_progress");
    }

    #[test]
    fn apply_no_checkbox_unchanged() {
        let fm = json!({"status": "todo", "title": "Task"});
        let body = "Just a regular body";
        let (new_fm, new_body) = apply_obsidian_tasks(fm, body);
        assert_eq!(new_fm["status"], "todo");
        assert_eq!(new_body, body);
    }

    #[test]
    fn strip_checkbox_line_with_blank_line() {
        let body = "- [x] Title\n\nBody text";
        assert_eq!(strip_checkbox_line(body), "Body text");
    }

    #[test]
    fn strip_checkbox_line_no_trailing_blank() {
        let body = "- [x] Title\nBody text";
        assert_eq!(strip_checkbox_line(body), "Body text");
    }
}
