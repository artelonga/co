//! CO-368: frontmatter validation for the Scrum entry types.
//!
//! PBIs and Sprints are ordinary CO entries (`entry_type = "pbi" | "sprint"`),
//! so they are additive — no migration, no new table. What they need beyond a
//! plain note is a *shape contract* on their frontmatter, enforced here and
//! wired into the generic entry-creation path (`EntryService::validate_entry_type`)
//! so both the sugar endpoints and the raw `/entries` POST validate identically.
//!
//! Returns `Result<(), String>` (a human-readable message) rather than
//! `AppError` so the module stays free of HTTP types and is trivially
//! unit-testable; the controller maps the message onto a 422.

use serde_json::Value;

/// Allowed `status` values for a PBI.
pub const PBI_STATUSES: &[&str] = &["backlog", "ready", "in-sprint", "done"];

/// Allowed t-shirt sizes for PBI `points` (story points may also be an integer).
pub const TSHIRT_POINTS: &[&str] = &["XS", "S", "M", "L", "XL"];

/// Allowed `status` values for a Sprint.
pub const SPRINT_STATUSES: &[&str] = &["planning", "active", "review", "closed"];

/// Validate any Scrum entry type. A no-op for non-Scrum types, so it is safe to
/// call on every entry write.
pub fn validate_scrum_entry(entry_type: &str, frontmatter: &Value) -> Result<(), String> {
    match entry_type {
        "pbi" | "sbi" => validate_pbi(frontmatter),
        "sprint" => validate_sprint(frontmatter),
        _ => Ok(()),
    }
}

fn require_str<'a>(fm: &'a Value, field: &str) -> Result<&'a str, String> {
    match fm.get(field) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s),
        Some(Value::String(_)) | None => Err(format!("missing required field `{field}`")),
        Some(_) => Err(format!("field `{field}` must be a string")),
    }
}

/// Validate PBI frontmatter: `priority`, `points`, `status`, `acceptance`.
pub fn validate_pbi(fm: &Value) -> Result<(), String> {
    if !fm.is_object() {
        return Err("PBI frontmatter must be an object".into());
    }

    // priority — required, non-empty string.
    require_str(fm, "priority")?;

    // status — required, one of the known states.
    let status = require_str(fm, "status")?;
    if !PBI_STATUSES.contains(&status) {
        return Err(format!(
            "invalid PBI `status` {status:?} (expected one of {PBI_STATUSES:?})"
        ));
    }

    // points — optional, but if present must be a positive int or a t-shirt size.
    match fm.get("points") {
        None | Some(Value::Null) => {}
        Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= 0) => {}
        Some(Value::String(s)) if TSHIRT_POINTS.contains(&s.as_str()) => {}
        Some(_) => {
            return Err(format!(
                "PBI `points` must be a non-negative integer or one of {TSHIRT_POINTS:?}"
            ));
        }
    }

    // acceptance — optional, but if present must be an array of strings.
    match fm.get("acceptance") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            if !items.iter().all(Value::is_string) {
                return Err("PBI `acceptance` must be an array of strings".into());
            }
        }
        Some(_) => return Err("PBI `acceptance` must be an array of strings".into()),
    }

    Ok(())
}

/// Validate Sprint frontmatter: `number`, `start_at`, `end_at`, `goal`,
/// `release_tag`, `status`.
pub fn validate_sprint(fm: &Value) -> Result<(), String> {
    if !fm.is_object() {
        return Err("Sprint frontmatter must be an object".into());
    }

    // number — required integer.
    match fm.get("number") {
        Some(Value::Number(n)) if n.as_i64().is_some() => {}
        Some(_) => return Err("Sprint `number` must be an integer".into()),
        None => return Err("missing required field `number`".into()),
    }

    require_str(fm, "start_at")?;
    require_str(fm, "end_at")?;
    require_str(fm, "goal")?;
    require_str(fm, "release_tag")?;

    // status — optional, but if present must be a known sprint state.
    if let Some(Value::String(s)) = fm.get("status")
        && !SPRINT_STATUSES.contains(&s.as_str())
    {
        return Err(format!(
            "invalid Sprint `status` {s:?} (expected one of {SPRINT_STATUSES:?})"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_pbi_passes() {
        let fm = json!({
            "priority": "high",
            "status": "ready",
            "points": "M",
            "acceptance": ["renders", "saves"],
        });
        assert!(validate_pbi(&fm).is_ok());
        assert!(validate_scrum_entry("pbi", &fm).is_ok());
    }

    #[test]
    fn pbi_integer_points_ok() {
        let fm = json!({ "priority": "low", "status": "backlog", "points": 5 });
        assert!(validate_pbi(&fm).is_ok());
    }

    #[test]
    fn pbi_missing_status_rejected() {
        let fm = json!({ "priority": "high" });
        assert!(validate_pbi(&fm).is_err());
    }

    #[test]
    fn pbi_bad_status_rejected() {
        let fm = json!({ "priority": "high", "status": "wip" });
        let err = validate_pbi(&fm).unwrap_err();
        assert!(err.contains("status"), "{err}");
    }

    #[test]
    fn pbi_bad_points_rejected() {
        let fm = json!({ "priority": "high", "status": "ready", "points": "XXL" });
        assert!(validate_pbi(&fm).is_err());
    }

    #[test]
    fn pbi_bad_acceptance_rejected() {
        let fm = json!({ "priority": "high", "status": "ready", "acceptance": [1, 2] });
        assert!(validate_pbi(&fm).is_err());
    }

    #[test]
    fn valid_sprint_passes() {
        let fm = json!({
            "number": 12,
            "start_at": "2026-06-11T15:00:00-03:00",
            "end_at": "2026-06-25T15:00:00-03:00",
            "goal": "Ship scrum artifacts",
            "release_tag": "v3.11.0",
            "status": "active",
        });
        assert!(validate_sprint(&fm).is_ok());
        assert!(validate_scrum_entry("sprint", &fm).is_ok());
    }

    #[test]
    fn sprint_missing_number_rejected() {
        let fm = json!({
            "start_at": "x", "end_at": "y", "goal": "g", "release_tag": "v1.0.0"
        });
        assert!(validate_sprint(&fm).is_err());
    }

    #[test]
    fn sprint_missing_goal_rejected() {
        let fm = json!({
            "number": 1, "start_at": "x", "end_at": "y", "release_tag": "v1.0.0"
        });
        let err = validate_sprint(&fm).unwrap_err();
        assert!(err.contains("goal"), "{err}");
    }

    #[test]
    fn sprint_bad_status_rejected() {
        let fm = json!({
            "number": 1, "start_at": "x", "end_at": "y", "goal": "g",
            "release_tag": "v1.0.0", "status": "frozen"
        });
        assert!(validate_sprint(&fm).is_err());
    }

    #[test]
    fn non_scrum_type_is_noop() {
        assert!(validate_scrum_entry("note", &json!({})).is_ok());
        assert!(validate_scrum_entry("task", &json!({ "anything": true })).is_ok());
    }
}
