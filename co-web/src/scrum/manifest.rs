//! CO-368: per-universe `_scrum.yaml` loader.
//!
//! A universe may declare a Scrum cadence + roles + Definition-of-Done in
//! `_scrum.yaml` at its content root (same pattern as CO-387's `_calendar.yaml`
//! and CO-355's `_workspace.yaml`). When the file is absent or invalid the
//! universe behaves exactly as before — `enabled: false`, no Scrum tab — so
//! existing universes see no change (acceptance: "universe WITHOUT `_scrum.yaml`
//! is unchanged").

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::current::{CurrentSprint, current_sprint};

/// Default sprint length when `_scrum.yaml` omits `sprint_length_days`.
pub const DEFAULT_SPRINT_LENGTH_DAYS: i64 = 14;

/// Default release-window width, in hours. The window from the PR cutoff
/// (Wed 23:59 BRT) to ship (Thu 15:00 BRT) is ~15h, so a sprint reports
/// `release_window: true` during its final 15 hours.
pub const DEFAULT_RELEASE_WINDOW_HOURS: i64 = 15;

/// Roles block of `_scrum.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScrumRoles {
    #[serde(default)]
    pub product_owner: String,
    #[serde(default)]
    pub scrum_master: String,
    #[serde(default)]
    pub developers: Vec<String>,
}

/// Whole `_scrum.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrumManifest {
    /// When false (the default), the universe has no Scrum surface.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sprint_length")]
    pub sprint_length_days: i64,
    #[serde(default)]
    pub sprint_start_day: Option<String>,
    #[serde(default)]
    pub sprint_start_time: Option<String>,
    /// First-sprint anchor (RFC-3339) used to compute the current sprint.
    #[serde(default)]
    pub sprint_start_anchor: Option<String>,
    #[serde(default)]
    pub release_tag_pattern: Option<String>,
    /// How many hours before a sprint's end the release window opens.
    #[serde(default = "default_release_window_hours")]
    pub release_window_hours: i64,
    #[serde(default)]
    pub roles: ScrumRoles,
    #[serde(default)]
    pub default_dod: Vec<String>,
}

fn default_sprint_length() -> i64 {
    DEFAULT_SPRINT_LENGTH_DAYS
}

fn default_release_window_hours() -> i64 {
    DEFAULT_RELEASE_WINDOW_HOURS
}

impl Default for ScrumManifest {
    fn default() -> Self {
        ScrumManifest {
            enabled: false,
            sprint_length_days: DEFAULT_SPRINT_LENGTH_DAYS,
            sprint_start_day: None,
            sprint_start_time: None,
            sprint_start_anchor: None,
            release_tag_pattern: None,
            release_window_hours: DEFAULT_RELEASE_WINDOW_HOURS,
            roles: ScrumRoles::default(),
            default_dod: Vec::new(),
        }
    }
}

impl ScrumManifest {
    /// The disabled default returned for a universe with no `_scrum.yaml`.
    pub fn disabled() -> Self {
        ScrumManifest::default()
    }

    /// Compute the current sprint from the cadence + anchor, evaluated at `now`.
    ///
    /// Returns `None` when Scrum is disabled or the anchor is missing/unparsable
    /// — the cadence cannot be derived without a starting point.
    pub fn current_at(&self, now: DateTime<Utc>) -> Option<CurrentSprint> {
        if !self.enabled {
            return None;
        }
        let anchor_raw = self.sprint_start_anchor.as_deref()?;
        let anchor = DateTime::parse_from_rfc3339(anchor_raw)
            .map_err(|e| warn!("scrum: invalid sprint_start_anchor {anchor_raw:?}: {e}"))
            .ok()?;
        Some(current_sprint(
            now,
            anchor,
            self.sprint_length_days,
            self.release_window_hours,
        ))
    }
}

/// Load `_scrum.yaml` from a universe content root.
///
/// Falls back to [`ScrumManifest::disabled`] when the file is absent,
/// unreadable, or invalid YAML — never an error, so a malformed manifest can
/// never take a universe offline.
pub fn load_scrum(root: &Path) -> ScrumManifest {
    let path = root.join("_scrum.yaml");
    if !path.exists() {
        return ScrumManifest::disabled();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            warn!(path = %path.display(), "scrum: cannot read _scrum.yaml: {e}");
            return ScrumManifest::disabled();
        }
    };
    match serde_yaml::from_str::<ScrumManifest>(&raw) {
        Ok(m) => m,
        Err(e) => {
            warn!(path = %path.display(), "scrum: invalid _scrum.yaml — {e}");
            ScrumManifest::disabled()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_is_disabled_default() {
        let dir = tempdir().unwrap();
        let m = load_scrum(dir.path());
        assert!(!m.enabled);
        assert_eq!(m.sprint_length_days, DEFAULT_SPRINT_LENGTH_DAYS);
        assert!(m.default_dod.is_empty());
        // A disabled manifest never computes a sprint.
        assert!(m.current_at(Utc::now()).is_none());
    }

    #[test]
    fn invalid_yaml_falls_back_to_disabled() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("_scrum.yaml"), ": not yaml [").unwrap();
        let m = load_scrum(dir.path());
        assert!(!m.enabled);
    }

    #[test]
    fn parses_spec_example() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("_scrum.yaml"),
            r#"
enabled: true
sprint_length_days: 14
sprint_start_day: thursday
sprint_start_time: "15:00"
sprint_start_anchor: "2026-06-11T15:00:00-03:00"
release_tag_pattern: "v{major}.{minor}.0"
roles:
  product_owner: yuri@artelonga.com.br
  scrum_master: ""
  developers:
    - claude
    - yuri@artelonga.com.br
default_dod:
  - "Acceptance criteria checkboxes all green"
  - "cargo test + cargo clippy -D warnings clean"
  - "PR merged to main"
"#,
        )
        .unwrap();
        let m = load_scrum(dir.path());
        assert!(m.enabled);
        assert_eq!(m.sprint_length_days, 14);
        assert_eq!(m.sprint_start_day.as_deref(), Some("thursday"));
        assert_eq!(m.roles.product_owner, "yuri@artelonga.com.br");
        assert_eq!(m.roles.developers.len(), 2);
        assert_eq!(m.default_dod.len(), 3);
        assert_eq!(m.release_window_hours, DEFAULT_RELEASE_WINDOW_HOURS);

        // With an anchor + enabled, the current sprint is computable.
        let now = DateTime::parse_from_rfc3339("2026-06-20T12:00:00-03:00")
            .unwrap()
            .with_timezone(&Utc);
        let cur = m.current_at(now).unwrap();
        assert_eq!(cur.number, 1);
    }

    #[test]
    fn enabled_without_anchor_has_no_current_sprint() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("_scrum.yaml"), "enabled: true\n").unwrap();
        let m = load_scrum(dir.path());
        assert!(m.enabled);
        assert!(m.current_at(Utc::now()).is_none());
    }
}
