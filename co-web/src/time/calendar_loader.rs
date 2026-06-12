//! CO-387: per-universe `_calendar.yaml` loader.
//!
//! A universe may declare its available calendar lenses in `_calendar.yaml`
//! at the content root (same pattern as CO-355's `_workspace.yaml`). When the
//! file is absent or invalid, the universe falls back to the built-in
//! Gregorian lens — no breaking change for existing universes.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Whole `_calendar.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarConfig {
    /// Lens id selected when the user has no stored preference.
    pub default_lens: String,
    pub lenses: Vec<LensDef>,
}

/// Scale of a lens' axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    #[default]
    Linear,
    Log,
}

/// Type of the canonical field a lens reads.
///
/// `I64Ms` (the default) reads Unix milliseconds — covers ±292 Myr from
/// epoch. `F64Years` reads fractional years-before-present (cosmic scale —
/// 13.8 Gyr overflows i64 ms by ~47×). `I64Units` reads a raw linear unit
/// (e.g. a fictional calendar's year number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalType {
    I64Ms,
    F64Years,
    I64Units,
}

/// A named label on the lens axis (e.g. "Big Bang" at 13.8e9 years bp).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelPeriod {
    pub name: String,
    pub at: f64,
}

/// One calendar lens definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensDef {
    pub id: String,
    pub name: String,
    /// Epoch in Unix ms for i64-ms lenses. `None` = Unix epoch (0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch_ms: Option<i64>,
    #[serde(default)]
    pub scale: Scale,
    /// Canonical field this lens reads. Defaults to `event_at_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_field: Option<String>,
    /// Explicit canonical type; see [`LensDef::resolved_canonical_type`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_type: Option<CanonicalType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week_length_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekday_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month_length: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_unit: Option<String>,
    /// Named epoch for non-ms lenses (e.g. `present` for cosmic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    /// Frontmatter field that drives position for custom-unit lenses
    /// (e.g. `shandara_year`). Implies `CanonicalType::I64Units` unless
    /// `canonical_type` says otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_event_field: Option<String>,
    /// Pomodoro-style work cell duration. Presence switches the lens to
    /// cell math (25-min cells + breaks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_periods: Vec<LabelPeriod>,
}

impl LensDef {
    /// The entry field this lens reads its canonical value from.
    ///
    /// Resolution: `custom_event_field` → `canonical_field` → `event_at_ms`.
    pub fn resolved_canonical_field(&self) -> &str {
        self.custom_event_field
            .as_deref()
            .or(self.canonical_field.as_deref())
            .unwrap_or("event_at_ms")
    }

    /// The canonical type driving this lens' math.
    ///
    /// Explicit `canonical_type` wins; a `custom_event_field` without an
    /// explicit type implies raw linear units; otherwise Unix ms.
    pub fn resolved_canonical_type(&self) -> CanonicalType {
        if let Some(t) = self.canonical_type {
            return t;
        }
        if self.custom_event_field.is_some() {
            return CanonicalType::I64Units;
        }
        CanonicalType::I64Ms
    }
}

/// Built-in Gregorian lens — the canonical default.
pub fn gregorian_lens() -> LensDef {
    LensDef {
        id: "gregorian".into(),
        name: "Gregorian (canonical)".into(),
        epoch_ms: Some(0),
        scale: Scale::Linear,
        canonical_field: Some("event_at_ms".into()),
        canonical_type: Some(CanonicalType::I64Ms),
        week_length_days: Some(7),
        weekday_names: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        month_length: Some("gregorian".into()),
        timezone: None,
        display_format: None,
        display_unit: None,
        epoch: None,
        custom_event_field: None,
        cell_duration_ms: None,
        break_duration_ms: None,
        label_periods: Vec::new(),
    }
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
