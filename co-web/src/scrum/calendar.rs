//! CO-372: Sprint calendar API.
//!
//! GET /api/v1/scrum/calendar?past=N&future=N  — JSON response
//! GET /api/v1/scrum/calendar.ics              — iCalendar subscription
//!
//! Sprint data comes from docs/scrum/sprints/_index.json (embedded at compile
//! time). DoD percentages are computed by parsing spec files in work/co/ (or
//! /app/seed-co/ in Docker).

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::atividade::{Acao, Atividade, Tipo, log_atividade};
use crate::server::AppState;

// Sprint 0 ends on this date (the going-forward anchor Thursday).
const ANCHOR_DATE: &str = "2026-06-11";
const SPRINT_DAYS: i64 = 14;

// Embedded at compile time; baked into the binary so it works without the
// docs/ tree at runtime. Update via `npm run scrum:retro` then rebuild.
static SPRINT_INDEX_JSON: &str = include_str!("../../../docs/scrum/sprints/_index.json");

// ---------------------------------------------------------------------------
// Index deserialisation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SprintIndexEntry {
    number: i32,
    #[serde(default)]
    release_tags: Vec<String>,
    #[serde(default)]
    pbis: Vec<PbiIndexEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PbiIndexEntry {
    id: String,
    #[serde(default)]
    status: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SprintEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub at: String,
}

#[derive(Debug, Serialize)]
pub struct SprintDto {
    pub number: i32,
    pub start_at: String,
    pub end_at: String,
    pub release_tag: Option<String>,
    pub goal: Option<String>,
    pub pbis_delivered: u32,
    pub pbis_carried_over: u32,
    pub dod_complete_pct: f64,
    pub events: Vec<SprintEvent>,
}

#[derive(Debug, Serialize)]
pub struct CurrentSprintInfo {
    pub number: i32,
    pub start_at: String,
    pub end_at: String,
}

#[derive(Debug, Serialize)]
pub struct CalendarResponse {
    pub anchor: String,
    pub length_days: u32,
    pub now: String,
    pub current_sprint: CurrentSprintInfo,
    pub sprints: Vec<SprintDto>,
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    pub past: Option<u32>,
    pub future: Option<u32>,
}

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

fn anchor_date() -> NaiveDate {
    NaiveDate::parse_from_str(ANCHOR_DATE, "%Y-%m-%d").expect("anchor date is valid")
}

/// Sprint N ends at: anchor + N * 14 days   (sprint 0 ends on anchor)
pub fn sprint_end_date(n: i32) -> NaiveDate {
    anchor_date() + Duration::days(n as i64 * SPRINT_DAYS)
}

/// Sprint N starts at: sprint_end(N) - 13 days
pub fn sprint_start_date(n: i32) -> NaiveDate {
    sprint_end_date(n) - Duration::days(SPRINT_DAYS - 1)
}

/// Determine the current sprint number given today's date.
pub fn current_sprint_number(today: NaiveDate) -> i32 {
    let anchor = anchor_date();
    let diff = (today - anchor).num_days();
    // Sprint 0: anchor-13 ..= anchor  (diff <= 0)
    // Sprint 1: anchor+1  ..= anchor+14
    // Sprint N: diff in [(N-1)*14+1 ..= N*14]
    if diff <= 0 {
        0
    } else {
        ((diff - 1) / SPRINT_DAYS + 1) as i32
    }
}

fn sprint_datetime(date: NaiveDate, hour: u8, minute: u8) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:00-03:00",
        date.year_ce().1,
        date.month(),
        date.day(),
        hour,
        minute,
    )
}

fn sprint_events(start: NaiveDate, end: NaiveDate) -> Vec<SprintEvent> {
    vec![
        SprintEvent {
            event_type: "planning".into(),
            at: sprint_datetime(start, 10, 0),
        },
        SprintEvent {
            event_type: "review".into(),
            at: sprint_datetime(end, 15, 0),
        },
        SprintEvent {
            event_type: "retro".into(),
            at: sprint_datetime(end, 16, 0),
        },
    ]
}

// ---------------------------------------------------------------------------
// DoD parsing
// ---------------------------------------------------------------------------

/// Try to find a spec file for a given PBI ID. Checks both dev (work/co/) and
/// Docker (/app/seed-co/) locations.
fn find_spec_content(pbi_id: &str) -> Option<String> {
    let candidates = [
        format!("work/co/{}.md", pbi_id),
        format!("/app/seed-co/{}.md", pbi_id),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Some(content);
        }
    }
    None
}

/// Parse `## Acceptance` section and count checked vs total checkboxes.
/// Returns (checked, total).
pub fn parse_acceptance_checkboxes(content: &str) -> (u32, u32) {
    let mut in_acceptance = false;
    let mut checked = 0u32;
    let mut total = 0u32;

    for line in content.lines() {
        if line.starts_with("## ") {
            in_acceptance = line == "## Acceptance";
        }
        if in_acceptance {
            let trimmed = line.trim_start_matches(' ').trim_start_matches('\t');
            if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
                checked += 1;
                total += 1;
            } else if trimmed.starts_with("- [ ]") {
                total += 1;
            }
        }
    }
    (checked, total)
}

/// Compute DoD percentage for a sprint's delivered PBIs.
pub(crate) fn compute_dod_pct(pbis: &[PbiIndexEntry]) -> f64 {
    let mut total_checked = 0u32;
    let mut total_items = 0u32;

    for pbi in pbis {
        if pbi.status != "done" {
            continue;
        }
        if let Some(content) = find_spec_content(&pbi.id) {
            let (checked, items) = parse_acceptance_checkboxes(&content);
            total_checked += checked;
            total_items += items;
        }
    }

    if total_items == 0 {
        return 0.0;
    }
    (total_checked as f64 / total_items as f64) * 100.0
}

// ---------------------------------------------------------------------------
// Core computation (testable without AppState)
// ---------------------------------------------------------------------------

fn build_sprint_dto(n: i32, entry: Option<&SprintIndexEntry>) -> SprintDto {
    let start = sprint_start_date(n);
    let end = sprint_end_date(n);

    let release_tag = entry.and_then(|e| e.release_tags.first()).cloned();
    let pbis_delivered = entry
        .map(|e| e.pbis.iter().filter(|p| p.status == "done").count() as u32)
        .unwrap_or(0);
    let dod_pct = entry.map(|e| compute_dod_pct(&e.pbis)).unwrap_or(0.0);

    SprintDto {
        number: n,
        start_at: format!("{}T15:00:00-03:00", start),
        end_at: format!("{}T15:00:00-03:00", end),
        release_tag,
        goal: None,
        pbis_delivered,
        pbis_carried_over: 0,
        dod_complete_pct: (dod_pct * 10.0).round() / 10.0,
        events: sprint_events(start, end),
    }
}

pub(crate) fn compute_calendar(
    today: NaiveDate,
    past: u32,
    future: u32,
    index: &[SprintIndexEntry],
) -> CalendarResponse {
    let past = past.min(24);
    let future = future.min(24);
    let current_n = current_sprint_number(today);

    let mut sprints = Vec::with_capacity((past + future) as usize);

    // Past sprints (oldest first)
    for offset in (1..=past).rev() {
        let n = current_n - offset as i32;
        let entry = index.iter().find(|e| e.number == n);
        sprints.push(build_sprint_dto(n, entry));
    }

    // Future sprints
    for offset in 1..=future {
        let n = current_n + offset as i32;
        let entry = index.iter().find(|e| e.number == n);
        sprints.push(build_sprint_dto(n, entry));
    }

    let current_start = sprint_start_date(current_n);
    let current_end = sprint_end_date(current_n);

    CalendarResponse {
        anchor: format!("{}T15:00:00-03:00", ANCHOR_DATE),
        length_days: SPRINT_DAYS as u32,
        now: chrono::Utc::now().to_rfc3339(),
        current_sprint: CurrentSprintInfo {
            number: current_n,
            start_at: format!("{}T15:00:00-03:00", current_start),
            end_at: format!("{}T15:00:00-03:00", current_end),
        },
        sprints,
    }
}

// ---------------------------------------------------------------------------
// ICS generation
// ---------------------------------------------------------------------------

fn ics_utc(date: NaiveDate, brt_hour: u8, brt_min: u8) -> String {
    // BRT = UTC-3; add 3 hours to get UTC
    let utc_hour = brt_hour + 3;
    format!(
        "{:04}{:02}{:02}T{:02}{:02}00Z",
        date.year_ce().1,
        date.month(),
        date.day(),
        utc_hour,
        brt_min,
    )
}

pub(crate) fn generate_ics(
    today: NaiveDate,
    past: u32,
    future: u32,
    index: &[SprintIndexEntry],
) -> String {
    let past = past.min(24);
    let future = future.min(24);
    let current_n = current_sprint_number(today);

    let mut out = String::new();
    out.push_str("BEGIN:VCALENDAR\r\n");
    out.push_str("VERSION:2.0\r\n");
    out.push_str("PRODID:-//CO Platform//Sprint Calendar//PT\r\n");
    out.push_str("CALSCALE:GREGORIAN\r\n");
    out.push_str("METHOD:PUBLISH\r\n");
    out.push_str("X-WR-CALNAME:CO Sprints\r\n");
    out.push_str("X-WR-TIMEZONE:America/Sao_Paulo\r\n");

    for n in (current_n - past as i32)..=(current_n + future as i32) {
        let start = sprint_start_date(n);
        let end = sprint_end_date(n);
        let goal = index
            .iter()
            .find(|e| e.number == n)
            .and_then(|e| e.pbis.first())
            .map(|_| "")
            .unwrap_or("");

        let events: &[(&str, NaiveDate, u8, u8, u8, u8, &str)] = &[
            ("planning", start, 10, 0, 11, 0, "Planning"),
            ("review", end, 15, 0, 16, 0, "Review"),
            ("retro", end, 16, 0, 17, 0, "Retrospective"),
        ];

        for (event_type, date, sh, sm, eh, em, label) in events {
            let uid = format!("sprint-{}-{}@co.artelonga.com.br", n, event_type);
            out.push_str("BEGIN:VEVENT\r\n");
            out.push_str(&format!("UID:{}\r\n", uid));
            out.push_str(&format!("DTSTART:{}\r\n", ics_utc(*date, *sh, *sm)));
            out.push_str(&format!("DTEND:{}\r\n", ics_utc(*date, *eh, *em)));
            out.push_str(&format!("SUMMARY:Sprint {} — {}\r\n", n, label));
            if !goal.is_empty() {
                out.push_str(&format!("DESCRIPTION:{}\r\n", goal));
            }
            out.push_str("BEGIN:VALARM\r\n");
            out.push_str("TRIGGER:-PT15M\r\n");
            out.push_str("ACTION:DISPLAY\r\n");
            out.push_str("DESCRIPTION:Lembrete\r\n");
            out.push_str("END:VALARM\r\n");
            out.push_str("END:VEVENT\r\n");
        }
    }

    out.push_str("END:VCALENDAR\r\n");
    out
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn load_index() -> Vec<SprintIndexEntry> {
    serde_json::from_str(SPRINT_INDEX_JSON).unwrap_or_default()
}

pub async fn calendar_handler(
    State(state): State<AppState>,
    Query(params): Query<CalendarQuery>,
) -> impl IntoResponse {
    let index = load_index();
    let today = chrono::Local::now().date_naive();
    let past = params.past.unwrap_or(6);
    let future = params.future.unwrap_or(4);

    let response = compute_calendar(today, past, future, &index);

    log_atividade(
        state,
        Atividade {
            acao: Acao::Ler,
            entidade: "scrum.calendar".into(),
            entidade_id: None,
            before: None,
            after: Some(serde_json::json!({
                "event_type": "scrum.calendar.viewed",
                "past": past,
                "future": future,
            })),
            tipo: Tipo::Navegacao,
            user_id: None,
            ip: None,
            user_agent: None,
        },
    );

    Json(response)
}

pub async fn calendar_ics_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let index = load_index();
    let today = chrono::Local::now().date_naive();
    let ics = generate_ics(today, 12, 4, &index);

    // Detect calendar app subscriptions by User-Agent
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let is_calendar_client = ua.contains("Google Calendar")
        || ua.contains("CalDAV")
        || ua.contains("Apple-PubSub")
        || ua.contains("Thunderbird")
        || ua.contains("Evolution")
        || ua.contains("Outlook");

    if is_calendar_client {
        log_atividade(
            state,
            Atividade {
                acao: Acao::Ler,
                entidade: "scrum.ics".into(),
                entidade_id: None,
                before: None,
                after: Some(serde_json::json!({
                    "event_type": "scrum.ics.subscribed",
                    "user_agent": ua,
                })),
                tipo: Tipo::Navegacao,
                user_id: None,
                ip: None,
                user_agent: Some(ua.to_string()),
            },
        );
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        ics,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// SPA page handler
// ---------------------------------------------------------------------------

static CALENDAR_PAGE_HTML: &str = include_str!("../../static/variants/a/scrum-calendar.html");

pub async fn serve_calendar_page() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        CALENDAR_PAGE_HTML,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/calendar", get(calendar_handler))
        .route("/calendar.ics", get(calendar_ics_handler))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_index() -> Vec<SprintIndexEntry> {
        serde_json::from_str(
            r#"[
              {
                "number": -1,
                "start": "2026-05-15",
                "end": "2026-05-28",
                "release_tags": ["v2.31.0"],
                "pbi_count": 2,
                "pbis": [
                  {"id": "CO-X1", "status": "done", "release_tag": "v2.31.0"},
                  {"id": "CO-X2", "status": "done", "release_tag": "v2.31.0"}
                ]
              },
              {
                "number": 0,
                "start": "2026-05-29",
                "end": "2026-06-11",
                "release_tags": ["v2.40.0"],
                "pbi_count": 1,
                "pbis": [
                  {"id": "CO-X3", "status": "done", "release_tag": "v2.40.0"}
                ]
              },
              {
                "number": 1,
                "start": "2026-06-12",
                "end": "2026-06-25",
                "release_tags": [],
                "pbi_count": 0,
                "pbis": []
              }
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn test_current_sprint_number_in_sprint_zero() {
        // Today is 2026-06-10, sprint 0 ends 2026-06-11
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        assert_eq!(current_sprint_number(today), 0);
    }

    #[test]
    fn test_current_sprint_number_on_anchor() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        assert_eq!(current_sprint_number(today), 0);
    }

    #[test]
    fn test_current_sprint_number_in_sprint_one() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        assert_eq!(current_sprint_number(today), 1);
    }

    #[test]
    fn test_sprint_start_end_dates() {
        // Sprint 0 ends 2026-06-11, starts 2026-05-29
        let end0 = sprint_end_date(0);
        let start0 = sprint_start_date(0);
        assert_eq!(end0.to_string(), "2026-06-11");
        assert_eq!(start0.to_string(), "2026-05-29");

        // Sprint 1 ends 2026-06-25, starts 2026-06-12
        let end1 = sprint_end_date(1);
        let start1 = sprint_start_date(1);
        assert_eq!(end1.to_string(), "2026-06-25");
        assert_eq!(start1.to_string(), "2026-06-12");
    }

    #[test]
    fn test_compute_calendar_returns_correct_count() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let cal = compute_calendar(today, 6, 4, &index);
        assert_eq!(cal.sprints.len(), 10, "past=6 + future=4 = 10 sprints");
        assert_eq!(cal.current_sprint.number, 0);
    }

    #[test]
    fn test_compute_calendar_sprint_ordering() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let cal = compute_calendar(today, 2, 2, &index);
        // First 2 are past (oldest first), last 2 are future
        assert_eq!(cal.sprints[0].number, -2);
        assert_eq!(cal.sprints[1].number, -1);
        assert_eq!(cal.sprints[2].number, 1);
        assert_eq!(cal.sprints[3].number, 2);
    }

    #[test]
    fn test_sprint_dto_has_events() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let cal = compute_calendar(today, 1, 0, &index);
        let sprint = &cal.sprints[0];
        assert_eq!(sprint.events.len(), 3);
        assert_eq!(sprint.events[0].event_type, "planning");
        assert_eq!(sprint.events[1].event_type, "review");
        assert_eq!(sprint.events[2].event_type, "retro");
    }

    #[test]
    fn test_parse_acceptance_checkboxes_basic() {
        let content = "## Acceptance\n- [x] First criterion\n- [ ] Second criterion\n- [x] Third\n## Other\n- [ ] Not counted\n";
        let (checked, total) = parse_acceptance_checkboxes(content);
        assert_eq!(checked, 2);
        assert_eq!(total, 3);
    }

    #[test]
    fn test_parse_acceptance_checkboxes_empty() {
        let (checked, total) = parse_acceptance_checkboxes("No acceptance section here");
        assert_eq!(checked, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_generate_ics_contains_vcalendar() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let ics = generate_ics(today, 1, 1, &index);
        assert!(ics.contains("BEGIN:VCALENDAR"));
        assert!(ics.contains("END:VCALENDAR"));
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("END:VEVENT"));
    }

    #[test]
    fn test_generate_ics_uid_pattern() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let ics = generate_ics(today, 0, 1, &index);
        assert!(ics.contains("sprint-1-planning@co.artelonga.com.br"));
        assert!(ics.contains("sprint-1-review@co.artelonga.com.br"));
        assert!(ics.contains("sprint-1-retro@co.artelonga.com.br"));
    }

    #[test]
    fn test_generate_ics_has_alarm() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let index = test_index();
        let ics = generate_ics(today, 0, 1, &index);
        assert!(ics.contains("BEGIN:VALARM"));
        assert!(ics.contains("TRIGGER:-PT15M"));
        assert!(ics.contains("END:VALARM"));
    }

    #[test]
    fn test_ics_utc_conversion() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        // 10:00 BRT = 13:00 UTC
        let result = ics_utc(date, 10, 0);
        assert_eq!(result, "20260612T130000Z");
        // 15:00 BRT = 18:00 UTC
        let result2 = ics_utc(date, 15, 0);
        assert_eq!(result2, "20260612T180000Z");
    }

    #[test]
    fn test_sprint_index_json_is_valid() {
        let result: Result<Vec<SprintIndexEntry>, _> = serde_json::from_str(SPRINT_INDEX_JSON);
        assert!(result.is_ok(), "SPRINT_INDEX_JSON must be valid JSON");
        let index = result.unwrap();
        assert!(!index.is_empty(), "sprint index must not be empty");
    }
}
