//! CO-387: per-universe `_calendar.yaml` loader.
//!
//! A universe may declare its available calendar lenses in `_calendar.yaml`
//! at the content root (same pattern as CO-355's `_workspace.yaml`). When the
//! file is absent or invalid, the universe falls back to the built-in
//! Gregorian lens — no breaking change for existing universes.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

// CO-396: the lens types + Gregorian default live once in the shared layout
// engine (`game_core::time_layout`), consumed by both co-web and
// yggdrasil-core. Re-export them so the rest of co-web keeps importing
// `crate::time::calendar_loader::{LensDef, …}` unchanged.
pub use game_core::time_layout::{
    CanonicalType, LabelPeriod, LaneField, LensDef, Scale, gregorian_lens,
};

/// Whole `_calendar.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarConfig {
    /// Lens id selected when the user has no stored preference.
    pub default_lens: String,
    pub lenses: Vec<LensDef>,
}

impl CalendarConfig {
    /// Default config used when a universe has no `_calendar.yaml`.
    pub fn default_gregorian() -> Self {
        CalendarConfig {
            default_lens: "gregorian".into(),
            lenses: vec![gregorian_lens()],
        }
    }

    pub fn lens_by_id(&self, id: &str) -> Option<&LensDef> {
        self.lenses.iter().find(|l| l.id == id)
    }
}

/// Load `_calendar.yaml` from a universe content root.
///
/// Falls back to [`CalendarConfig::default_gregorian`] when the file is
/// absent, unreadable, invalid YAML, or declares no lenses. A missing or
/// dangling `default_lens` is repaired to the first declared lens.
pub fn load_calendar(root: &Path) -> CalendarConfig {
    let path = root.join("_calendar.yaml");
    if !path.exists() {
        return CalendarConfig::default_gregorian();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            warn!(path = %path.display(), "calendar: cannot read _calendar.yaml: {e}");
            return CalendarConfig::default_gregorian();
        }
    };
    let mut cfg: CalendarConfig = match serde_yaml::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %path.display(), "calendar: invalid _calendar.yaml — {e}");
            return CalendarConfig::default_gregorian();
        }
    };
    if cfg.lenses.is_empty() {
        return CalendarConfig::default_gregorian();
    }
    if cfg.lens_by_id(&cfg.default_lens).is_none() {
        cfg.default_lens = cfg.lenses[0].id.clone();
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_defaults_to_gregorian() {
        let dir = tempdir().unwrap();
        let cfg = load_calendar(dir.path());
        assert_eq!(cfg.default_lens, "gregorian");
        assert_eq!(cfg.lenses.len(), 1);
        assert_eq!(cfg.lenses[0].week_length_days, Some(7));
    }

    #[test]
    fn invalid_yaml_defaults_to_gregorian() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("_calendar.yaml"), ": not yaml [").unwrap();
        let cfg = load_calendar(dir.path());
        assert_eq!(cfg.default_lens, "gregorian");
    }

    #[test]
    fn parses_spec_example_lenses() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("_calendar.yaml"),
            r#"
default_lens: gregorian
lenses:
  - id: gregorian
    name: Gregorian (canonical)
    epoch_ms: 0
    scale: linear
    week_length_days: 7
    weekday_names: [Mon, Tue, Wed, Thu, Fri, Sat, Sun]
    month_length: gregorian
    timezone: America/Sao_Paulo
  - id: 4-day-week
    name: 4-day week experiment
    epoch_ms: 1735689600000
    scale: linear
    week_length_days: 4
    weekday_names: [Um, Dois, "Três", Quatro]
    display_format: "S{week}.D{day}"
  - id: cosmic
    name: Cosmic (Big Bang -> now)
    canonical_field: cosmic_year_bp
    canonical_type: f64_years
    scale: log
    epoch: present
    display_unit: "billion years"
    label_periods:
      - { name: "Big Bang", at: 13.8e9 }
      - { name: "Common Era", at: 2024 }
  - id: shandara
    name: Shandara epoch
    epoch_ms: 0
    scale: linear
    custom_event_field: "shandara_year"
    week_length_days: 7
  - id: pomodoro
    name: Pomodoro (work cells)
    epoch_ms: 1735689600000
    scale: linear
    cell_duration_ms: 1500000
    break_duration_ms: 300000
    display_format: "Pom{cell}"
"#,
        )
        .unwrap();
        let cfg = load_calendar(dir.path());
        assert_eq!(cfg.default_lens, "gregorian");
        assert_eq!(cfg.lenses.len(), 5);

        let four = cfg.lens_by_id("4-day-week").unwrap();
        assert_eq!(four.week_length_days, Some(4));
        assert_eq!(four.resolved_canonical_type(), CanonicalType::I64Ms);
        assert_eq!(four.resolved_canonical_field(), "event_at_ms");

        let cosmic = cfg.lens_by_id("cosmic").unwrap();
        assert_eq!(cosmic.scale, Scale::Log);
        assert_eq!(cosmic.resolved_canonical_type(), CanonicalType::F64Years);
        assert_eq!(cosmic.resolved_canonical_field(), "cosmic_year_bp");
        assert_eq!(cosmic.label_periods.len(), 2);

        let shandara = cfg.lens_by_id("shandara").unwrap();
        assert_eq!(shandara.resolved_canonical_field(), "shandara_year");
        assert_eq!(shandara.resolved_canonical_type(), CanonicalType::I64Units);

        let pomodoro = cfg.lens_by_id("pomodoro").unwrap();
        assert_eq!(pomodoro.cell_duration_ms, Some(1_500_000));
    }

    /// CO-396: a `_calendar.yaml` lens may declare itself a project-timeline
    /// lens via `lane_by: epic|module|status`.
    #[test]
    fn parses_project_timeline_lane_by() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("_calendar.yaml"),
            r#"
default_lens: roadmap
lenses:
  - id: roadmap
    name: Project roadmap
    epoch_ms: 0
    scale: linear
    lane_by: epic
"#,
        )
        .unwrap();
        let cfg = load_calendar(dir.path());
        let roadmap = cfg.lens_by_id("roadmap").unwrap();
        assert_eq!(roadmap.lane_by, Some(LaneField::Epic));
        // Non-project-timeline lenses leave it unset (round-trips as absent).
        let gregorian = gregorian_lens();
        assert_eq!(gregorian.lane_by, None);
        let json = serde_json::to_value(&gregorian).unwrap();
        assert!(json.get("lane_by").is_none());
    }

    #[test]
    fn dangling_default_lens_repaired_to_first() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("_calendar.yaml"),
            "default_lens: nope\nlenses:\n  - id: fiscal\n    name: Fiscal\n",
        )
        .unwrap();
        let cfg = load_calendar(dir.path());
        assert_eq!(cfg.default_lens, "fiscal");
    }
}
