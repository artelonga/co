//! Shared timeline layout engine (CO-396 / YG-123).
//!
//! The lens/layout math for time rendering lives **once**, here in the
//! path-dep crate both hosts consume:
//!
//! - `co-web` → `<co-time-grid>` (CO-387) → the **project-timeline** lens.
//! - `yggdrasil-core` (path-deps `game-core`) → `generators/timeline.rs`
//!   (YG-123) → the timeline-world of the `time` universe.
//!
//! Two layers:
//!
//! 1. **Per-lens conversion math** (hoisted from CO-387's
//!    `co-web/src/time/conversion.rs`): an entry's canonical value → a
//!    [`LensPosition`] on a calendar lens' axis. Mirrored in
//!    `static/shared/lib/co-time.js` — keep both in sync.
//! 2. **Project-timeline layout** (CO-396): the same entries that fill a
//!    kanban board, arranged into lanes (by epic/module/status) with gantt
//!    spans, release/milestone markers, and a "no date" backlog lane so a
//!    task without a date never disappears.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const MS_PER_DAY: f64 = 86_400_000.0;
pub const MS_PER_HOUR: f64 = 3_600_000.0;

// ===========================================================================
// Layer 1 — calendar lenses + per-lens conversion math (hoisted from CO-387).
// ===========================================================================

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

/// Which entry field a project-timeline lens groups its lanes by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneField {
    Epic,
    Module,
    Status,
}

impl LaneField {
    /// The entry frontmatter field this lane grouping reads.
    pub fn field_name(self) -> &'static str {
        match self {
            LaneField::Epic => "epic",
            LaneField::Module => "module",
            LaneField::Status => "status",
        }
    }
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
    /// CO-396: when set, this lens is a **project-timeline** lens — its
    /// entries are laid out in lanes grouped by this field (epic/module/
    /// status), with release/milestone entries lifted out as markers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane_by: Option<LaneField>,
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
        lane_by: None,
    }
}

/// Raw canonical value read from an entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanonicalValue {
    I64(i64),
    F64(f64),
}

impl CanonicalValue {
    fn as_i64(self) -> Option<i64> {
        match self {
            CanonicalValue::I64(v) => Some(v),
            CanonicalValue::F64(v) if v.fract() == 0.0 => Some(v as i64),
            CanonicalValue::F64(_) => None,
        }
    }

    fn as_f64(self) -> f64 {
        match self {
            CanonicalValue::I64(v) => v as f64,
            CanonicalValue::F64(v) => v,
        }
    }
}

/// Where an entry lands on a lens' axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LensPosition {
    /// Linear calendar grid placement (Gregorian, 4-day-week, fictional-ms).
    Linear {
        week: i64,
        day_of_week: i64,
        hour: i64,
    },
    /// Pomodoro-style work cell placement.
    Cell { cell: i64, in_break: bool },
    /// Raw linear axis placement in lens units (e.g. `shandara_year`).
    Axis { position: f64 },
    /// Log-scale placement (cosmic): log10 of years-before-present.
    Log { log_position: f64 },
}

/// Read the lens' canonical value from an entry's ms columns + frontmatter.
///
/// Column fields (`event_at_ms`, `due_at_ms`, `scheduled_at_ms`) come from the
/// caller-supplied columns; any other field name falls back to
/// `frontmatter[<field>]` (cosmic, shandara, …).
pub fn read_canonical_value(
    field: &str,
    event_at_ms: Option<i64>,
    due_at_ms: Option<i64>,
    scheduled_at_ms: Option<i64>,
    frontmatter: &JsonValue,
) -> Option<CanonicalValue> {
    match field {
        "event_at_ms" => event_at_ms.map(CanonicalValue::I64),
        "due_at_ms" => due_at_ms.map(CanonicalValue::I64),
        "scheduled_at_ms" => scheduled_at_ms.map(CanonicalValue::I64),
        _ => match frontmatter.get(field) {
            Some(JsonValue::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    Some(CanonicalValue::I64(i))
                } else {
                    n.as_f64().map(CanonicalValue::F64)
                }
            }
            _ => None,
        },
    }
}

/// Map a canonical value to a lens position. `None` = type/scale mismatch —
/// the renderer shows the entry as an orphan.
pub fn entry_to_lens_position(raw: CanonicalValue, lens: &LensDef) -> Option<LensPosition> {
    match (lens.scale, lens.resolved_canonical_type()) {
        (Scale::Linear, CanonicalType::I64Ms) => {
            let ms = raw.as_i64()?;
            let offset = (ms - lens.epoch_ms.unwrap_or(0)) as f64;
            if let Some(cell_ms) = lens.cell_duration_ms {
                // Pomodoro-style: cycle = work cell + break.
                let break_ms = lens.break_duration_ms.unwrap_or(0).max(0);
                let cycle = (cell_ms.max(1) + break_ms) as f64;
                let cell = (offset / cycle).floor() as i64;
                let within = offset - (cell as f64) * cycle;
                return Some(LensPosition::Cell {
                    cell,
                    in_break: within >= cell_ms as f64,
                });
            }
            let day = (offset / MS_PER_DAY).floor() as i64;
            let week_len = i64::from(lens.week_length_days.unwrap_or(7).max(1));
            Some(LensPosition::Linear {
                week: day.div_euclid(week_len),
                day_of_week: day.rem_euclid(week_len),
                hour: ((offset / MS_PER_HOUR).floor() as i64).rem_euclid(24),
            })
        }
        (Scale::Linear, CanonicalType::I64Units) => {
            let units = raw.as_f64();
            Some(LensPosition::Axis {
                position: units - lens.epoch_ms.unwrap_or(0) as f64,
            })
        }
        (Scale::Log, CanonicalType::F64Years) => {
            // log10 placement on a normalized axis; closer to present = right.
            Some(LensPosition::Log {
                log_position: raw.as_f64().max(1.0).log10(),
            })
        }
        _ => None, // type/scale mismatch — render as orphan
    }
}

/// Inverse mapping for "click a cell to create an event there" — emits the
/// canonical value for the active lens.
pub fn lens_position_to_canonical(pos: LensPosition, lens: &LensDef) -> Option<CanonicalValue> {
    match (pos, lens.scale, lens.resolved_canonical_type()) {
        (
            LensPosition::Linear {
                week,
                day_of_week,
                hour,
            },
            Scale::Linear,
            CanonicalType::I64Ms,
        ) => {
            let week_len = i64::from(lens.week_length_days.unwrap_or(7).max(1));
            let day = week * week_len + day_of_week;
            let ms =
                lens.epoch_ms.unwrap_or(0) + day * MS_PER_DAY as i64 + hour * MS_PER_HOUR as i64;
            Some(CanonicalValue::I64(ms))
        }
        (LensPosition::Cell { cell, .. }, Scale::Linear, CanonicalType::I64Ms) => {
            let cell_ms = lens.cell_duration_ms?;
            let cycle = cell_ms + lens.break_duration_ms.unwrap_or(0).max(0);
            Some(CanonicalValue::I64(
                lens.epoch_ms.unwrap_or(0) + cell * cycle,
            ))
        }
        (LensPosition::Axis { position }, Scale::Linear, CanonicalType::I64Units) => Some(
            CanonicalValue::I64(position as i64 + lens.epoch_ms.unwrap_or(0)),
        ),
        (LensPosition::Log { log_position }, Scale::Log, CanonicalType::F64Years) => {
            Some(CanonicalValue::F64(10f64.powf(log_position)))
        }
        _ => None,
    }
}

// ===========================================================================
// Layer 2 — project-timeline layout (CO-396).
// ===========================================================================

/// One entry feeding the project-timeline layout. The host (co-web or
/// yggdrasil-core) maps its own row/instance type onto this neutral shape —
/// the layout engine has no knowledge of either host's storage.
///
/// Dates are Unix ms, all optional. `created_ms`/`done_ms` are board
/// lifecycle stamps; `scheduled_ms`/`due_ms`/`event_ms` are explicit schedule
/// fields. `entry_type` of `release`/`milestone` lifts the entry to a marker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub entry_type: String,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_ms: Option<i64>,
    #[serde(default)]
    pub scheduled_ms: Option<i64>,
    #[serde(default)]
    pub due_ms: Option<i64>,
    #[serde(default)]
    pub event_ms: Option<i64>,
    #[serde(default)]
    pub done_ms: Option<i64>,
}

impl TimelineEntry {
    /// The lane-grouping value for the chosen field; `None` (or empty) means
    /// "no group" — the entry lands in the catch-all lane.
    fn lane_value(&self, field: LaneField) -> Option<&str> {
        let raw = match field {
            LaneField::Epic => self.epic.as_deref(),
            LaneField::Module => self.module.as_deref(),
            LaneField::Status => self.status.as_deref(),
        };
        raw.map(str::trim).filter(|s| !s.is_empty())
    }

    /// True when the entry is a release/milestone — laid out as a marker.
    fn is_marker(&self) -> bool {
        matches!(self.entry_type.as_str(), "release" | "milestone")
    }

    /// The single anchor instant for a marker / point placement, in priority
    /// order. `created_ms` is deliberately excluded — a task that was merely
    /// filed (created) but never scheduled has no roadmap date and belongs in
    /// the backlog, not pinned to its creation day.
    fn anchor_ms(&self) -> Option<i64> {
        self.due_ms
            .or(self.scheduled_ms)
            .or(self.event_ms)
            .or(self.done_ms)
    }
}

/// A gantt bar's extent on the time axis. A zero-width span (`start == end`)
/// is a point — a single dated entry with no measurable duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GanttSpan {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl GanttSpan {
    pub fn duration_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }
    pub fn is_point(&self) -> bool {
        self.start_ms == self.end_ms
    }
}

/// An entry placed onto the timeline (gantt bar or point).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedItem {
    pub id: String,
    pub title: String,
    pub entry_type: String,
    pub span: GanttSpan,
}

/// A release/milestone lifted out of the lanes as a vertical marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub at_ms: i64,
}

/// One horizontal lane (a group of items sharing a lane value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lane {
    pub key: String,
    pub label: String,
    pub items: Vec<PlacedItem>,
}

/// An undated entry parked in the "no date" backlog lane on the margin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacklogItem {
    pub id: String,
    pub title: String,
    pub entry_type: String,
}

/// The fully laid-out project timeline: dated entries in lanes, release/
/// milestone markers, and a backlog of undated entries that never vanish.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTimeline {
    pub group_by: LaneField,
    pub lanes: Vec<Lane>,
    pub markers: Vec<Marker>,
    pub backlog: Vec<BacklogItem>,
}

/// Lane label for entries that carry no value for the grouping field.
pub const UNGROUPED_LANE_KEY: &str = "";
pub const UNGROUPED_LANE_LABEL: &str = "(ungrouped)";

/// Compute the gantt span for an entry, `None` when it has no timeline date.
///
/// Duration precedence (CO-396):
/// 1. `scheduled_ms → due_ms` when **both** present (the explicit schedule).
/// 2. `created_ms → done_ms` when **both** present (the lifecycle span).
/// 3. otherwise a zero-width point at the entry's anchor instant
///    (`due → scheduled → event → done`), if any.
///
/// `created_ms` alone never produces a span — see [`TimelineEntry::anchor_ms`].
/// A span is always normalized so `start_ms <= end_ms`.
pub fn gantt_span(entry: &TimelineEntry) -> Option<GanttSpan> {
    let span = |a: i64, b: i64| GanttSpan {
        start_ms: a.min(b),
        end_ms: a.max(b),
    };
    if let (Some(s), Some(d)) = (entry.scheduled_ms, entry.due_ms) {
        return Some(span(s, d));
    }
    if let (Some(c), Some(done)) = (entry.created_ms, entry.done_ms) {
        return Some(span(c, done));
    }
    entry.anchor_ms().map(|a| span(a, a))
}

/// Lay out entries into lanes + markers + backlog for a project timeline.
///
/// - release/milestone entries with a date → [`Marker`]; without a date →
///   backlog (a marker never silently disappears either).
/// - every other dated entry → a lane keyed by its `group_by` value
///   (empty/missing value → the ungrouped lane).
/// - every undated entry → the backlog.
///
/// Lane and item order is the first-seen input order — deterministic, no
/// clock or hashing — so two hosts laying out the same entries agree.
pub fn layout_project_timeline(entries: &[TimelineEntry], group_by: LaneField) -> ProjectTimeline {
    let mut lanes: Vec<Lane> = Vec::new();
    let mut markers: Vec<Marker> = Vec::new();
    let mut backlog: Vec<BacklogItem> = Vec::new();

    for entry in entries {
        let span = gantt_span(entry);

        if entry.is_marker() {
            match entry.anchor_ms() {
                Some(at_ms) => markers.push(Marker {
                    id: entry.id.clone(),
                    title: entry.title.clone(),
                    kind: entry.entry_type.clone(),
                    at_ms,
                }),
                None => backlog.push(BacklogItem {
                    id: entry.id.clone(),
                    title: entry.title.clone(),
                    entry_type: entry.entry_type.clone(),
                }),
            }
            continue;
        }

        let Some(span) = span else {
            backlog.push(BacklogItem {
                id: entry.id.clone(),
                title: entry.title.clone(),
                entry_type: entry.entry_type.clone(),
            });
            continue;
        };

        let (key, label) = match entry.lane_value(group_by) {
            Some(v) => (v.to_string(), v.to_string()),
            None => (
                UNGROUPED_LANE_KEY.to_string(),
                UNGROUPED_LANE_LABEL.to_string(),
            ),
        };
        let item = PlacedItem {
            id: entry.id.clone(),
            title: entry.title.clone(),
            entry_type: entry.entry_type.clone(),
            span,
        };
        match lanes.iter_mut().find(|l| l.key == key) {
            Some(lane) => lane.items.push(item),
            None => lanes.push(Lane {
                key,
                label,
                items: vec![item],
            }),
        }
    }

    ProjectTimeline {
        group_by,
        lanes,
        markers,
        backlog,
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    use serde_json::json;

    fn four_day_week_lens() -> LensDef {
        LensDef {
            id: "4-day-week".into(),
            name: "4-day week".into(),
            epoch_ms: Some(1_735_689_600_000), // 2025-01-01T00:00:00Z
            week_length_days: Some(4),
            ..gregorian_lens()
        }
    }

    fn cosmic_lens() -> LensDef {
        LensDef {
            id: "cosmic".into(),
            name: "Cosmic".into(),
            scale: Scale::Log,
            canonical_field: Some("cosmic_year_bp".into()),
            canonical_type: Some(CanonicalType::F64Years),
            epoch_ms: None,
            week_length_days: None,
            ..gregorian_lens()
        }
    }

    fn pomodoro_lens() -> LensDef {
        LensDef {
            id: "pomodoro".into(),
            name: "Pomodoro".into(),
            epoch_ms: Some(0),
            cell_duration_ms: Some(1_500_000), // 25 min
            break_duration_ms: Some(300_000),  // 5 min
            ..gregorian_lens()
        }
    }

    fn shandara_lens() -> LensDef {
        LensDef {
            id: "shandara".into(),
            name: "Shandara".into(),
            epoch_ms: Some(0),
            canonical_type: None,
            custom_event_field: Some("shandara_year".into()),
            ..gregorian_lens()
        }
    }

    #[test]
    fn four_day_week_groups_year_into_91_weeks() {
        let lens = four_day_week_lens();
        let epoch = lens.epoch_ms.unwrap();
        let p0 = entry_to_lens_position(CanonicalValue::I64(epoch), &lens).unwrap();
        assert_eq!(
            p0,
            LensPosition::Linear {
                week: 0,
                day_of_week: 0,
                hour: 0
            }
        );
        let day363 = epoch + 363 * MS_PER_DAY as i64;
        let p = entry_to_lens_position(CanonicalValue::I64(day363), &lens).unwrap();
        assert_eq!(
            p,
            LensPosition::Linear {
                week: 90,
                day_of_week: 3,
                hour: 0
            }
        );
    }

    #[test]
    fn gregorian_defaults_to_seven_day_weeks() {
        let lens = gregorian_lens();
        let p = entry_to_lens_position(CanonicalValue::I64(8 * MS_PER_DAY as i64), &lens).unwrap();
        assert_eq!(
            p,
            LensPosition::Linear {
                week: 1,
                day_of_week: 1,
                hour: 0
            }
        );
    }

    #[test]
    fn pre_epoch_dates_use_euclidean_week_math() {
        let lens = gregorian_lens();
        let p = entry_to_lens_position(CanonicalValue::I64(-(MS_PER_HOUR as i64)), &lens).unwrap();
        assert_eq!(
            p,
            LensPosition::Linear {
                week: -1,
                day_of_week: 6,
                hour: 23
            }
        );
    }

    #[test]
    fn cosmic_log_scale_places_big_bang_and_common_era() {
        let lens = cosmic_lens();
        let big_bang = entry_to_lens_position(CanonicalValue::F64(13.8e9), &lens).unwrap();
        let common_era = entry_to_lens_position(CanonicalValue::F64(2024.0), &lens).unwrap();
        let (LensPosition::Log { log_position: bb }, LensPosition::Log { log_position: ce }) =
            (big_bang, common_era)
        else {
            panic!("cosmic lens must produce Log positions");
        };
        assert!((bb - 10.1399).abs() < 0.001, "Big Bang log10 ≈ 10.14: {bb}");
        assert!((ce - 3.3062).abs() < 0.001, "CE log10 ≈ 3.31: {ce}");
        assert!(bb > ce);
    }

    #[test]
    fn pomodoro_groups_into_cells_and_breaks() {
        let lens = pomodoro_lens();
        let cell0 = entry_to_lens_position(CanonicalValue::I64(0), &lens).unwrap();
        assert_eq!(
            cell0,
            LensPosition::Cell {
                cell: 0,
                in_break: false
            }
        );
        let in_break = entry_to_lens_position(CanonicalValue::I64(26 * 60 * 1000), &lens).unwrap();
        assert_eq!(
            in_break,
            LensPosition::Cell {
                cell: 0,
                in_break: true
            }
        );
        let cell1 = entry_to_lens_position(CanonicalValue::I64(30 * 60 * 1000), &lens).unwrap();
        assert_eq!(
            cell1,
            LensPosition::Cell {
                cell: 1,
                in_break: false
            }
        );
    }

    #[test]
    fn shandara_custom_field_drives_axis_position() {
        let lens = shandara_lens();
        let fm = json!({"shandara_year": 412});
        let raw =
            read_canonical_value(lens.resolved_canonical_field(), None, None, None, &fm).unwrap();
        let p = entry_to_lens_position(raw, &lens).unwrap();
        assert_eq!(p, LensPosition::Axis { position: 412.0 });
    }

    #[test]
    fn read_canonical_value_prefers_ms_columns() {
        let fm = json!({"event_at_ms": 999});
        let v = read_canonical_value("event_at_ms", Some(123), None, None, &fm).unwrap();
        assert_eq!(v, CanonicalValue::I64(123));
        assert!(read_canonical_value("event_at_ms", None, None, None, &fm).is_none());
    }

    #[test]
    fn mismatched_scale_renders_as_orphan() {
        let mut lens = cosmic_lens();
        lens.canonical_type = Some(CanonicalType::I64Ms);
        assert!(entry_to_lens_position(CanonicalValue::I64(0), &lens).is_none());
    }

    #[test]
    fn lens_position_round_trips() {
        let lens = four_day_week_lens();
        let original = CanonicalValue::I64(lens.epoch_ms.unwrap() + 47 * MS_PER_DAY as i64);
        let pos = entry_to_lens_position(original, &lens).unwrap();
        let back = lens_position_to_canonical(pos, &lens).unwrap();
        assert_eq!(back, original);

        let pomo = pomodoro_lens();
        let pos = entry_to_lens_position(CanonicalValue::I64(3 * 1_800_000), &pomo).unwrap();
        assert_eq!(
            lens_position_to_canonical(pos, &pomo),
            Some(CanonicalValue::I64(3 * 1_800_000))
        );
    }
}

#[cfg(test)]
mod project_timeline_tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    fn task(id: &str) -> TimelineEntry {
        TimelineEntry {
            id: id.into(),
            title: id.into(),
            entry_type: "task".into(),
            ..Default::default()
        }
    }

    // AC: gantt duration uses scheduled→due when both present.
    #[test]
    fn gantt_span_prefers_scheduled_to_due() {
        let e = TimelineEntry {
            scheduled_ms: Some(10 * DAY),
            due_ms: Some(14 * DAY),
            created_ms: Some(DAY),
            done_ms: Some(20 * DAY),
            ..task("t")
        };
        let span = gantt_span(&e).unwrap();
        assert_eq!(span.start_ms, 10 * DAY);
        assert_eq!(span.end_ms, 14 * DAY);
        assert_eq!(span.duration_ms(), 4 * DAY);
    }

    // AC: gantt duration falls back to created→done.
    #[test]
    fn gantt_span_falls_back_to_created_to_done() {
        let e = TimelineEntry {
            created_ms: Some(2 * DAY),
            done_ms: Some(9 * DAY),
            ..task("t")
        };
        let span = gantt_span(&e).unwrap();
        assert_eq!(span.start_ms, 2 * DAY);
        assert_eq!(span.end_ms, 9 * DAY);
    }

    #[test]
    fn gantt_span_is_normalized_when_reversed() {
        let e = TimelineEntry {
            scheduled_ms: Some(14 * DAY),
            due_ms: Some(10 * DAY),
            ..task("t")
        };
        let span = gantt_span(&e).unwrap();
        assert_eq!(span.start_ms, 10 * DAY);
        assert_eq!(span.end_ms, 14 * DAY);
    }

    #[test]
    fn single_date_degrades_to_a_point() {
        let e = TimelineEntry {
            due_ms: Some(7 * DAY),
            ..task("t")
        };
        let span = gantt_span(&e).unwrap();
        assert!(span.is_point());
        assert_eq!(span.start_ms, 7 * DAY);
    }

    // AC: created-only entry has no roadmap date → no span (→ backlog).
    #[test]
    fn created_only_has_no_span() {
        let e = TimelineEntry {
            created_ms: Some(3 * DAY),
            ..task("t")
        };
        assert!(gantt_span(&e).is_none());
    }

    // AC: lanes group by the chosen field.
    #[test]
    fn lanes_group_by_field() {
        let entries = vec![
            TimelineEntry {
                epic: Some("auth".into()),
                due_ms: Some(DAY),
                ..task("a")
            },
            TimelineEntry {
                epic: Some("billing".into()),
                due_ms: Some(2 * DAY),
                ..task("b")
            },
            TimelineEntry {
                epic: Some("auth".into()),
                due_ms: Some(3 * DAY),
                ..task("c")
            },
        ];
        let tl = layout_project_timeline(&entries, LaneField::Epic);
        assert_eq!(tl.lanes.len(), 2);
        assert_eq!(tl.lanes[0].key, "auth");
        assert_eq!(tl.lanes[0].items.len(), 2); // a, c — first-seen order
        assert_eq!(tl.lanes[0].items[0].id, "a");
        assert_eq!(tl.lanes[0].items[1].id, "c");
        assert_eq!(tl.lanes[1].key, "billing");
    }

    #[test]
    fn lane_grouping_by_status_and_module() {
        let entries = vec![
            TimelineEntry {
                status: Some("doing".into()),
                module: Some("web".into()),
                due_ms: Some(DAY),
                ..task("a")
            },
            TimelineEntry {
                status: Some("done".into()),
                module: Some("web".into()),
                due_ms: Some(DAY),
                ..task("b")
            },
        ];
        let by_status = layout_project_timeline(&entries, LaneField::Status);
        assert_eq!(by_status.lanes.len(), 2);
        let by_module = layout_project_timeline(&entries, LaneField::Module);
        assert_eq!(by_module.lanes.len(), 1);
        assert_eq!(by_module.lanes[0].key, "web");
    }

    // AC: a task without a date lands in the backlog, never disappears.
    #[test]
    fn undated_task_goes_to_backlog() {
        let entries = vec![
            TimelineEntry {
                epic: Some("auth".into()),
                due_ms: Some(DAY),
                ..task("dated")
            },
            TimelineEntry {
                epic: Some("auth".into()),
                created_ms: Some(DAY), // created-only ⇒ no roadmap date
                ..task("undated")
            },
            task("bare"),
        ];
        let tl = layout_project_timeline(&entries, LaneField::Epic);
        assert_eq!(tl.lanes.len(), 1);
        assert_eq!(tl.lanes[0].items.len(), 1);
        assert_eq!(tl.backlog.len(), 2);
        let ids: Vec<&str> = tl.backlog.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["undated", "bare"]);
        // No entry was dropped.
        assert_eq!(
            tl.lanes.iter().map(|l| l.items.len()).sum::<usize>() + tl.backlog.len(),
            3
        );
    }

    // AC: release/milestone become markers, not lane items.
    #[test]
    fn releases_and_milestones_become_markers() {
        let entries = vec![
            TimelineEntry {
                entry_type: "release".into(),
                event_ms: Some(30 * DAY),
                ..task("v1.0")
            },
            TimelineEntry {
                entry_type: "milestone".into(),
                due_ms: Some(15 * DAY),
                ..task("beta")
            },
            TimelineEntry {
                epic: Some("auth".into()),
                due_ms: Some(DAY),
                ..task("task")
            },
        ];
        let tl = layout_project_timeline(&entries, LaneField::Epic);
        assert_eq!(tl.markers.len(), 2);
        assert_eq!(tl.markers[0].kind, "release");
        assert_eq!(tl.markers[0].at_ms, 30 * DAY);
        assert_eq!(tl.markers[1].kind, "milestone");
        assert_eq!(tl.markers[1].at_ms, 15 * DAY);
        // The marker entries are NOT in any lane.
        assert_eq!(tl.lanes.len(), 1);
        assert_eq!(tl.lanes[0].items.len(), 1);
        assert_eq!(tl.lanes[0].items[0].id, "task");
    }

    #[test]
    fn undated_marker_goes_to_backlog_not_dropped() {
        let entries = vec![TimelineEntry {
            entry_type: "release".into(),
            ..task("someday")
        }];
        let tl = layout_project_timeline(&entries, LaneField::Status);
        assert!(tl.markers.is_empty());
        assert_eq!(tl.backlog.len(), 1);
        assert_eq!(tl.backlog[0].id, "someday");
    }

    #[test]
    fn missing_lane_value_uses_ungrouped_lane() {
        let entries = vec![TimelineEntry {
            epic: Some("   ".into()), // whitespace ⇒ treated as missing
            due_ms: Some(DAY),
            ..task("x")
        }];
        let tl = layout_project_timeline(&entries, LaneField::Epic);
        assert_eq!(tl.lanes.len(), 1);
        assert_eq!(tl.lanes[0].key, UNGROUPED_LANE_KEY);
        assert_eq!(tl.lanes[0].label, UNGROUPED_LANE_LABEL);
    }

    #[test]
    fn layout_is_deterministic_and_serializable() {
        let entries = vec![
            TimelineEntry {
                status: Some("doing".into()),
                scheduled_ms: Some(DAY),
                due_ms: Some(3 * DAY),
                ..task("a")
            },
            task("backlog-task"),
        ];
        let tl = layout_project_timeline(&entries, LaneField::Status);
        // Round-trips through JSON (the wire contract for both hosts).
        let json = serde_json::to_string(&tl).unwrap();
        let back: ProjectTimeline = serde_json::from_str(&json).unwrap();
        assert_eq!(tl, back);
    }
}
