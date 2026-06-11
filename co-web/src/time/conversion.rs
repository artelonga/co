//! CO-387: pure per-lens conversion math.
//!
//! Each lens reads its canonical field from the entry, then maps the raw
//! value to a [`LensPosition`]. Math is per-lens; the renderer is shared.
//! Mirrored in `static/shared/lib/co-time.js` — keep both in sync.

use serde_json::Value as JsonValue;

use super::calendar_loader::{CanonicalType, LensDef, Scale};

pub const MS_PER_DAY: f64 = 86_400_000.0;
pub const MS_PER_HOUR: f64 = 3_600_000.0;

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
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
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
/// caller-supplied closure over indexed columns; any other field name falls
/// back to `frontmatter[<field>]` (cosmic, shandara, …).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::calendar_loader::gregorian_lens;
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

    // AC: 4-day-week lens groups a year's entries into 91 weeks x 4 days.
    #[test]
    fn four_day_week_groups_year_into_91_weeks() {
        let lens = four_day_week_lens();
        let epoch = lens.epoch_ms.unwrap();

        // Day 0 → week 0, day 0.
        let p0 = entry_to_lens_position(CanonicalValue::I64(epoch), &lens).unwrap();
        assert_eq!(
            p0,
            LensPosition::Linear {
                week: 0,
                day_of_week: 0,
                hour: 0
            }
        );

        // Day 363 (last of 364 day-slots) → week 90, day 3 → 91 weeks total.
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
        // One hour before epoch must land on the previous day/week, not day 0.
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

    // AC: cosmic lens (log scale) places Big Bang at one edge, CE near the other.
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
        assert!(bb > ce, "Big Bang further from present than Common Era");
    }

    // AC: Pomodoro lens groups by 25-min cells with 5-min breaks.
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
        // 26 min in → cell 0's break.
        let in_break = entry_to_lens_position(CanonicalValue::I64(26 * 60 * 1000), &lens).unwrap();
        assert_eq!(
            in_break,
            LensPosition::Cell {
                cell: 0,
                in_break: true
            }
        );
        // 30 min in → cell 1's work block.
        let cell1 = entry_to_lens_position(CanonicalValue::I64(30 * 60 * 1000), &lens).unwrap();
        assert_eq!(
            cell1,
            LensPosition::Cell {
                cell: 1,
                in_break: false
            }
        );
    }

    // AC: custom shandara_year field drives the Shandara/fictional lens.
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
        let fm = json!({"event_at_ms": 999}); // must be ignored for column fields
        let v = read_canonical_value("event_at_ms", Some(123), None, None, &fm).unwrap();
        assert_eq!(v, CanonicalValue::I64(123));
        assert!(read_canonical_value("event_at_ms", None, None, None, &fm).is_none());
    }

    #[test]
    fn mismatched_scale_renders_as_orphan() {
        // Log scale + i64 ms type = mismatch → None.
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
