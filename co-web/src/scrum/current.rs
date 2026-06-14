//! CO-368: deterministic current-sprint computation.
//!
//! Pure function — no DB, no clock of its own. Given `now`, the first-sprint
//! `anchor`, the sprint length and the release-window width, it derives which
//! sprint is active and whether the release window is open. Because it is pure,
//! the cadence is reproducible and unit-testable without any time mocking
//! (acceptance: "current sprint computed deterministically from cadence +
//! anchor").

use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

/// The active sprint, as returned by `/sprints/current`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentSprint {
    /// 1-based sprint number (`floor((now - anchor) / length) + 1`).
    pub number: i64,
    /// Sprint start, RFC-3339 in the anchor's offset.
    pub start_at: String,
    /// Sprint end (== next sprint's start), RFC-3339 in the anchor's offset.
    pub end_at: String,
    /// True when `now` is within the final `release_window_hours` of the sprint.
    pub release_window: bool,
}

/// Compute the active sprint at `now`.
///
/// `number = floor((now - anchor) / length_days) + 1`, using Euclidean
/// division so the formula stays correct even when `now < anchor` (the
/// pre-launch sprints get number ≤ 0). `start_at`/`end_at` preserve the
/// anchor's UTC offset so the SPA can render local wall-clock times.
pub fn current_sprint(
    now: DateTime<Utc>,
    anchor: DateTime<FixedOffset>,
    length_days: i64,
    release_window_hours: i64,
) -> CurrentSprint {
    let length = Duration::days(length_days.max(1));
    let length_secs = length.num_seconds().max(1);

    let delta_secs = (now - anchor.with_timezone(&Utc)).num_seconds();
    let index = delta_secs.div_euclid(length_secs); // 0-based

    // `Duration * i32` — cast is safe for any realistic sprint index.
    let start = anchor + length * (index as i32);
    let end = anchor + length * ((index + 1) as i32);

    let remaining = end.with_timezone(&Utc) - now;
    let release_window =
        remaining > Duration::zero() && remaining <= Duration::hours(release_window_hours.max(0));

    CurrentSprint {
        number: index + 1,
        start_at: start.to_rfc3339(),
        end_at: end.to_rfc3339(),
        release_window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn anchor() -> DateTime<FixedOffset> {
        // Sprint 1 starts Thu 2026-06-11 15:00 BRT; 14-day cadence.
        DateTime::parse_from_rfc3339("2026-06-11T15:00:00-03:00").unwrap()
    }

    #[test]
    fn first_day_is_sprint_one() {
        let cur = current_sprint(at("2026-06-11T16:00:00-03:00"), anchor(), 14, 15);
        assert_eq!(cur.number, 1);
        assert_eq!(cur.start_at, "2026-06-11T15:00:00-03:00");
        assert_eq!(cur.end_at, "2026-06-25T15:00:00-03:00");
        assert!(!cur.release_window);
    }

    #[test]
    fn second_sprint_boundary_is_exclusive_at_end() {
        // Exactly at the boundary belongs to the NEXT sprint.
        let cur = current_sprint(at("2026-06-25T15:00:00-03:00"), anchor(), 14, 15);
        assert_eq!(cur.number, 2);
        assert_eq!(cur.start_at, "2026-06-25T15:00:00-03:00");
    }

    #[test]
    fn deterministic_number_progression() {
        for (when, expected) in [
            ("2026-06-12T00:00:00-03:00", 1),
            ("2026-06-24T23:59:00-03:00", 1),
            ("2026-06-26T00:00:00-03:00", 2),
            ("2026-07-10T00:00:00-03:00", 3),
        ] {
            assert_eq!(
                current_sprint(at(when), anchor(), 14, 15).number,
                expected,
                "{when}"
            );
        }
    }

    #[test]
    fn release_window_opens_in_final_hours() {
        // 10h before the Thu 15:00 end → within the 15h window.
        let cur = current_sprint(at("2026-06-25T05:00:00-03:00"), anchor(), 14, 15);
        assert_eq!(cur.number, 1);
        assert!(cur.release_window);

        // 20h before end → outside the window.
        let cur = current_sprint(at("2026-06-24T19:00:00-03:00"), anchor(), 14, 15);
        assert!(!cur.release_window);
    }

    #[test]
    fn before_anchor_yields_non_positive_number() {
        let cur = current_sprint(at("2026-06-04T15:00:00-03:00"), anchor(), 14, 15);
        assert_eq!(cur.number, 0);
    }
}
