//! CO-499 — lead-temperature digest (output **B** of the input→síntese→3-outputs
//! loop). Turns raw web telemetry (`telemetry_events`) into a read-only
//! cold/warm/hot lead digest: per-subject *temperature* (from recency +
//! frequency + dwell), *trend* (subindo/caindo/estável, window-over-window) and
//! the *why* (top pages, return, language).
//!
//! ## PRIVACY INVARIANT (LGPD) — hard, unit-tested guarantee
//!
//! A subject is **NAMED** only when there is an identified/consented user
//! identity (`telemetry_events.user_id`, set after the CO-490 companion-app link
//! via `users.whatsapp` / a logged-in session). **ANONYMOUS** visitors — an
//! `al_vid` visitor token with no linked `user_id` — are reported ONLY inside an
//! [`Aggregate`] bucket and are NEVER emitted as a nominal lead. The partition
//! lives in [`compute_digest`]: a row whose `user_id` is `None`/empty can never
//! contribute to a [`NamedLead`]. See the `anonymous_*`/`mixed_*` tests.
//!
//! ## SACRED BOUNDARY
//!
//! This endpoint only *reads/organizes* telemetry into a digest. It never
//! publishes or sends anything — no WhatsApp message is dispatched here.
//!
//! ## Gating & multi-tenant scope (CO-499 fix)
//!
//! `GET /api/v1/whatsapp/telemetry/digest?universe=<key>` self-gates via
//! [`Scoped<TelemetryRead>`](crate::auth::extractors::Scoped) (`telemetry:read`,
//! CO-448), mirroring [`crate::telemetry_api`]: a full JWT/session passes, and a
//! least-privilege token carrying `telemetry:read` passes — a token lacking it
//! (e.g. `entries:read`-only) gets 403, an unauthenticated caller 401.
//!
//! **The digest is ALWAYS scoped to a single universe the CALLER can read.** The
//! `universe` query param is **required** (`400` when absent), and the caller —
//! the identity resolved by `Scoped<TelemetryRead>` — must hold READ access to
//! it under the deterministic CO-49 check ([`Storage::check_universe_access`]),
//! exactly as every per-universe read route authorizes; a caller without read
//! access gets `403`. The SQL then filters on `telemetry_events.universe_key`, so
//! a `telemetry:read` holder can never read the names/browsing of leads in a
//! universe they cannot read (the multi-tenant leak this fix closes). No new
//! schema and no new migration: every field is read from `telemetry_events`,
//! whose `universe_key` column is already present and indexed.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::auth::extractors::{Scoped, TelemetryRead};
use crate::content::models::UniverseAccess;
use crate::platform::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Tunables (pure scoring thresholds)
// ---------------------------------------------------------------------------

const SECS_PER_DAY: i64 = 86_400;
/// Default look-back window when `?days=` is absent.
const DEFAULT_WINDOW_DAYS: i64 = 14;
/// Clamp on the caller-supplied window so a digest never scans an unbounded span.
const MAX_WINDOW_DAYS: i64 = 90;
/// Beyond this recency a subject is cold regardless of past frequency/dwell — a
/// lead that has not returned in over a month is not "warm/hot" today.
const STALE_DAYS: i64 = 30;
/// How many top pages to surface in the `why`.
const TOP_PAGES: usize = 3;

// ---------------------------------------------------------------------------
// Input row
// ---------------------------------------------------------------------------

/// One telemetry observation relevant to lead scoring, projected from a
/// `telemetry_events` row. `user_id` is the linked identity (`None`/empty ⇒
/// anonymous); `visitor_token` is the `al_vid` marketing token.
#[derive(Debug, Clone)]
pub struct TelemetryRow {
    /// Event time, epoch seconds.
    pub ts: i64,
    pub visitor_token: Option<String>,
    /// Linked user identity. `None`/empty ⇒ ANONYMOUS (aggregate-only).
    pub user_id: Option<String>,
    pub path: Option<String>,
    /// Server-measured dwell (`telemetry_events.duration_ms`).
    pub dwell_ms: Option<i64>,
    pub referrer: Option<String>,
    pub language: Option<String>,
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Temperature {
    Cold,
    Warm,
    Hot,
}

/// Window-over-window direction. Portuguese domain terms (subindo/caindo) per
/// the project glossary; `estavel` when the two halves tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trend {
    Subindo,
    Caindo,
    Estavel,
}

/// The "why" behind a subject's temperature: the evidence the brain can quote.
#[derive(Debug, Clone, Serialize)]
pub struct Why {
    /// Total pageviews in the window.
    pub pageviews: usize,
    /// Distinct active calendar days (UTC) — the basis of `returned`.
    pub active_days: usize,
    /// Returned across more than one day.
    pub returned: bool,
    /// Most-viewed paths, most-frequent first (up to [`TOP_PAGES`]).
    pub top_pages: Vec<String>,
    /// Dominant language seen for the subject, if any.
    pub language: Option<String>,
    /// Most recent referrer seen, if any.
    pub referrer: Option<String>,
    /// Last-seen time, epoch seconds.
    pub last_seen_ts: i64,
    /// Total server-measured dwell across the window, milliseconds.
    pub total_dwell_ms: i64,
}

/// A NAMED lead. Only ever produced for an identified/consented `user_id`.
#[derive(Debug, Clone, Serialize)]
pub struct NamedLead {
    /// The identified subject. The pure layer emits the `user_id`; the HTTP
    /// handler best-effort rewrites it to the user's display name.
    pub subject: String,
    pub temperature: Temperature,
    pub trend: Trend,
    pub why: Why,
}

/// Count of anonymous visitors per temperature (the aggregate breakdown).
#[derive(Debug, Clone, Default, Serialize)]
pub struct TemperatureMix {
    pub cold: usize,
    pub warm: usize,
    pub hot: usize,
}

/// The aggregate (anonymous-only) bucket. Never names anyone — LGPD invariant.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Aggregate {
    /// Distinct anonymous visitors (`al_vid` tokens with no linked identity).
    pub count: usize,
    pub temperature_mix: TemperatureMix,
}

#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    pub window_days: i64,
    /// NAMED leads — identified/consented subjects only.
    pub named: Vec<NamedLead>,
    /// Everyone else, aggregated. Never nominal.
    pub aggregate: Aggregate,
}

// ---------------------------------------------------------------------------
// Pure classification
// ---------------------------------------------------------------------------

/// Cold/warm/hot from recency + frequency + dwell. Pure → unit-tested.
///
/// A subject not seen for more than [`STALE_DAYS`] is cold outright. Otherwise
/// recency, pageview frequency and total dwell each score 0–2 and the sum maps
/// to a band (`>=5` hot, `>=2` warm, else cold).
pub fn classify_temperature(
    now: i64,
    last_seen_ts: i64,
    pageviews: usize,
    total_dwell_ms: i64,
) -> Temperature {
    let days = (now - last_seen_ts).max(0) / SECS_PER_DAY;
    if days > STALE_DAYS {
        return Temperature::Cold;
    }
    let recency = if days <= 2 {
        2
    } else if days <= 7 {
        1
    } else {
        0
    };
    let frequency = if pageviews >= 8 {
        2
    } else if pageviews >= 3 {
        1
    } else {
        0
    };
    let dwell = if total_dwell_ms >= 120_000 {
        2
    } else if total_dwell_ms >= 30_000 {
        1
    } else {
        0
    };
    match recency + frequency + dwell {
        5..=6 => Temperature::Hot,
        2..=4 => Temperature::Warm,
        _ => Temperature::Cold,
    }
}

/// Window-over-window trend: split the window at `split` (epoch seconds) and
/// compare activity in the recent half vs the older half. Pure → unit-tested.
pub fn classify_trend(timestamps: &[i64], split: i64) -> Trend {
    let recent = timestamps.iter().filter(|&&t| t >= split).count();
    let older = timestamps.iter().filter(|&&t| t < split).count();
    match recent.cmp(&older) {
        std::cmp::Ordering::Greater => Trend::Subindo,
        std::cmp::Ordering::Less => Trend::Caindo,
        std::cmp::Ordering::Equal => Trend::Estavel,
    }
}

fn temp_rank(t: Temperature) -> u8 {
    match t {
        Temperature::Hot => 2,
        Temperature::Warm => 1,
        Temperature::Cold => 0,
    }
}

/// Reduce a subject's rows to its [`Why`] evidence.
fn summarize(rows: &[&TelemetryRow]) -> Why {
    let pageviews = rows.len();
    let last_seen_ts = rows.iter().map(|r| r.ts).max().unwrap_or(0);
    let total_dwell_ms = rows.iter().filter_map(|r| r.dwell_ms).sum();

    // Distinct active UTC days.
    let mut days: Vec<i64> = rows.iter().map(|r| r.ts / SECS_PER_DAY).collect();
    days.sort_unstable();
    days.dedup();
    let active_days = days.len();

    // Top pages by frequency (ties broken by path for determinism).
    let mut page_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        if let Some(p) = r.path.as_deref() {
            *page_counts.entry(p).or_default() += 1;
        }
    }
    let mut pages: Vec<(&str, usize)> = page_counts.into_iter().collect();
    pages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let top_pages: Vec<String> = pages
        .into_iter()
        .take(TOP_PAGES)
        .map(|(p, _)| p.to_string())
        .collect();

    // Dominant language (most frequent non-empty; ties broken lexically).
    let mut lang_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        if let Some(l) = r.language.as_deref().filter(|s| !s.is_empty()) {
            *lang_counts.entry(l).or_default() += 1;
        }
    }
    let language = lang_counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(l, _)| l.to_string());

    // Most recent non-empty referrer.
    let referrer = rows
        .iter()
        .filter(|r| r.referrer.as_deref().is_some_and(|s| !s.is_empty()))
        .max_by_key(|r| r.ts)
        .and_then(|r| r.referrer.clone());

    Why {
        pageviews,
        active_days,
        returned: active_days > 1,
        top_pages,
        language,
        referrer,
        last_seen_ts,
        total_dwell_ms,
    }
}

/// Build the lead-temperature digest from a window of telemetry rows.
///
/// PRIVACY INVARIANT: rows are partitioned by identity first — a row with a
/// non-empty `user_id` joins the NAMED set keyed by that id; every other row is
/// ANONYMOUS and can only ever land in the [`Aggregate`] bucket. There is no
/// code path from a `user_id == None` row to a [`NamedLead`].
pub fn compute_digest(rows: &[TelemetryRow], now: i64, window_seconds: i64) -> Digest {
    let split = now - window_seconds / 2;

    let mut by_user: BTreeMap<String, Vec<&TelemetryRow>> = BTreeMap::new();
    let mut by_visitor: BTreeMap<String, Vec<&TelemetryRow>> = BTreeMap::new();

    for r in rows {
        match r.user_id.as_deref().filter(|s| !s.is_empty()) {
            // Identified/consented → NAMED, keyed by the user identity.
            Some(uid) => by_user.entry(uid.to_string()).or_default().push(r),
            // Anonymous → AGGREGATE only, keyed by the al_vid visitor token.
            // A row with neither identity nor token is unattributable noise.
            None => {
                if let Some(vid) = r.visitor_token.as_deref().filter(|s| !s.is_empty()) {
                    by_visitor.entry(vid.to_string()).or_default().push(r);
                }
            }
        }
    }

    let mut named: Vec<NamedLead> = by_user
        .into_iter()
        .map(|(uid, group)| {
            let why = summarize(&group);
            let temperature =
                classify_temperature(now, why.last_seen_ts, why.pageviews, why.total_dwell_ms);
            let ts: Vec<i64> = group.iter().map(|r| r.ts).collect();
            let trend = classify_trend(&ts, split);
            NamedLead {
                subject: uid,
                temperature,
                trend,
                why,
            }
        })
        .collect();

    // Hottest first, then most-recent, then subject for a stable order.
    named.sort_by(|a, b| {
        temp_rank(b.temperature)
            .cmp(&temp_rank(a.temperature))
            .then(b.why.last_seen_ts.cmp(&a.why.last_seen_ts))
            .then(a.subject.cmp(&b.subject))
    });

    let mut mix = TemperatureMix::default();
    for group in by_visitor.values() {
        let why = summarize(group);
        match classify_temperature(now, why.last_seen_ts, why.pageviews, why.total_dwell_ms) {
            Temperature::Hot => mix.hot += 1,
            Temperature::Warm => mix.warm += 1,
            Temperature::Cold => mix.cold += 1,
        }
    }
    let aggregate = Aggregate {
        count: by_visitor.len(),
        temperature_mix: mix,
    };

    Digest {
        window_days: window_seconds / SECS_PER_DAY,
        named,
        aggregate,
    }
}

// ---------------------------------------------------------------------------
// HTTP endpoint — GET /api/v1/whatsapp/telemetry/digest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DigestQuery {
    /// Look-back window in days. Defaults to [`DEFAULT_WINDOW_DAYS`], clamped to
    /// `[1, MAX_WINDOW_DAYS]`.
    pub days: Option<i64>,
    /// CO-518 alias for [`days`](Self::days): the look-back window in the bot's
    /// `"<n>d"` form (e.g. `"7d"`, `"30d"`; a bare `"7"` also parses). The bot
    /// (`co_tools.telemetry_digest`) sends THIS param, so without it the window was
    /// silently ignored. [`days`](Self::days) wins when both are present.
    pub window: Option<String>,
    /// REQUIRED — the universe to scope the digest to. The caller must hold READ
    /// access to it; the telemetry query is filtered by this `universe_key`.
    /// Absent/blank ⇒ `400` (CO-499 multi-tenant fix).
    pub universe: Option<String>,
}

/// Resolve the look-back window in days from the two accepted params. `days` (the
/// legacy integer) wins when positive; otherwise the bot's `window` form (`"7d"`,
/// `"30d"`, or a bare `"7"`; trailing `d`/`D` optional) is parsed. Returns `None`
/// when neither yields a positive value → caller applies [`DEFAULT_WINDOW_DAYS`].
/// Pure → unit-testable.
pub fn parse_window_days(days: Option<i64>, window: Option<&str>) -> Option<i64> {
    if let Some(d) = days.filter(|&d| d > 0) {
        return Some(d);
    }
    let w = window?.trim();
    let digits = w
        .strip_suffix(|c: char| c == 'd' || c == 'D')
        .unwrap_or(w)
        .trim();
    digits.parse::<i64>().ok().filter(|&d| d > 0)
}

/// Authorize the caller's resolved [`UniverseAccess`] for a digest read: a digest
/// exposes per-subject names + browsing behavior, so it requires real READ access
/// (owner / member / subscriber / public-static). `MetadataOnly`, `LoginRequired`
/// and `Denied` are NOT read access → `403`. Mirrors the per-universe read gate.
fn ensure_read_access(access: UniverseAccess) -> Result<(), AppError> {
    match access {
        UniverseAccess::ReadOnly | UniverseAccess::ReadWrite => Ok(()),
        UniverseAccess::MetadataOnly | UniverseAccess::LoginRequired | UniverseAccess::Denied => {
            Err(AppError::Forbidden(
                "you do not have read access to this universe".into(),
            ))
        }
    }
}

/// Project the relevant `telemetry_events` columns for a window into rows. Reads
/// only — pageviews carry the dwell (`duration_ms`); referrer/language are
/// lifted from the JSON `properties` (`referrer`/`ref`, `lang`).
fn load_rows(conn: &rusqlite::Connection, since_ts: i64, universe: &str) -> Vec<TelemetryRow> {
    let since_rfc3339 = chrono::DateTime::<chrono::Utc>::from_timestamp(since_ts, 0)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    // CO-499: scope to ONE universe (the caller-authorized `universe_key`). The
    // (universe_key, timestamp) index backs this filter.
    let Ok(mut stmt) = conn.prepare(
        "SELECT timestamp, visitor_token, user_id, path, duration_ms, properties \
         FROM telemetry_events \
         WHERE event_type = 'pageview' AND timestamp >= ?1 AND universe_key = ?2",
    ) else {
        return Vec::new();
    };
    stmt.query_map(rusqlite::params![since_rfc3339, universe], |r| {
        let timestamp: String = r.get(0)?;
        let visitor_token: Option<String> = r.get(1)?;
        let user_id: Option<String> = r.get(2)?;
        let path: Option<String> = r.get(3)?;
        let dwell_ms: Option<i64> = r.get(4)?;
        let properties: Option<String> = r.get(5)?;
        Ok((
            timestamp,
            visitor_token,
            user_id,
            path,
            dwell_ms,
            properties,
        ))
    })
    .map(|mapped| {
        mapped
            .filter_map(|row| row.ok())
            .filter_map(
                |(timestamp, visitor_token, user_id, path, dwell_ms, properties)| {
                    let ts = chrono::DateTime::parse_from_rfc3339(&timestamp)
                        .ok()?
                        .timestamp();
                    let (referrer, language) = parse_props(properties.as_deref());
                    Some(TelemetryRow {
                        ts,
                        visitor_token,
                        user_id,
                        path,
                        dwell_ms,
                        referrer,
                        language,
                    })
                },
            )
            .collect()
    })
    .unwrap_or_default()
}

/// Lift `(referrer, language)` out of the stored `properties` JSON, tolerating
/// both the server-side (`referrer`) and marketing (`ref`/`lang`) shapes.
fn parse_props(properties: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = properties else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (None, None);
    };
    let referrer = v
        .get("referrer")
        .or_else(|| v.get("ref"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let language = v
        .get("lang")
        .or_else(|| v.get("language"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    (referrer, language)
}

/// `GET /api/v1/whatsapp/telemetry/digest?universe=<key>` — read-only
/// lead-temperature digest, scoped to ONE universe the caller can read.
async fn digest_handler(
    cap: Scoped<TelemetryRead>,
    State(state): State<AppState>,
    Query(q): Query<DigestQuery>,
) -> Result<Json<Digest>, AppError> {
    // CO-499: the universe to scope to is REQUIRED — a digest is never global.
    let universe = q
        .universe
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("universe is required".into()))?
        .to_string();

    // CO-518: accept both the legacy `days` integer and the bot's `window="7d"`.
    let window_days = parse_window_days(q.days, q.window.as_deref())
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);
    let window_seconds = window_days * SECS_PER_DAY;
    let now = chrono::Utc::now().timestamp();
    let since = now - window_seconds;

    // One lock: authorize the caller for THIS universe, then pull the window's
    // rows AND resolve the named subjects' display names; drop the lock before
    // the (pure) compute.
    let (rows, name_map) = {
        let storage = state.core.storage.lock();
        // CO-499 multi-tenant gate: the caller resolved by `Scoped<TelemetryRead>`
        // must hold READ access to `universe` (same deterministic CO-49 check
        // every per-universe read route uses). Without it, a `telemetry:read`
        // holder could read names + browsing behavior of leads across tenants.
        ensure_read_access(storage.check_universe_access(Some(&cap.user_id), &universe))?;
        let rows = load_rows(storage.conn(), since, &universe);
        let mut name_map: BTreeMap<String, String> = BTreeMap::new();
        for r in &rows {
            if let Some(uid) = r.user_id.as_deref().filter(|s| !s.is_empty())
                && !name_map.contains_key(uid)
                && let Some((display_name, _usuario)) = storage.get_user_display_info(uid)
            {
                name_map.insert(uid.to_string(), display_name);
            }
        }
        (rows, name_map)
    };

    let mut digest = compute_digest(&rows, now, window_seconds);
    // Best-effort: present the human display name instead of the raw user id.
    // (The privacy invariant already holds — these are all identified users.)
    for lead in &mut digest.named {
        if let Some(name) = name_map.get(&lead.subject) {
            lead.subject = name.clone();
        }
    }
    Ok(Json(digest))
}

/// Router for the lead-temperature digest, nested at `/api/v1` (full path
/// `/api/v1/whatsapp/telemetry/digest`). Self-gating via `Scoped<TelemetryRead>`,
/// so no auth middleware layer is needed at the mount site.
pub fn router() -> Router<AppState> {
    Router::new().route("/whatsapp/telemetry/digest", get(digest_handler))
}

// ---------------------------------------------------------------------------
// Unit tests (pure — no server)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = SECS_PER_DAY;

    fn row(ts: i64, visitor: Option<&str>, user: Option<&str>) -> TelemetryRow {
        TelemetryRow {
            ts,
            visitor_token: visitor.map(String::from),
            user_id: user.map(String::from),
            path: Some("/precos".into()),
            dwell_ms: Some(40_000),
            referrer: Some("https://google.com".into()),
            language: Some("pt".into()),
        }
    }

    // ---- temperature classification ----

    #[test]
    fn temperature_hot_when_recent_frequent_and_engaged() {
        let now = 100 * DAY;
        // seen today, 10 pageviews, >2min dwell → 2+2+2 = 6 → hot.
        assert_eq!(
            classify_temperature(now, now, 10, 130_000),
            Temperature::Hot
        );
    }

    #[test]
    fn temperature_warm_in_the_middle_band() {
        let now = 100 * DAY;
        // 5 days ago (recency 1), 4 pageviews (freq 1), 40s dwell (dwell 1) = 3 → warm.
        assert_eq!(
            classify_temperature(now, now - 5 * DAY, 4, 40_000),
            Temperature::Warm
        );
    }

    #[test]
    fn temperature_cold_when_sparse() {
        let now = 100 * DAY;
        // 10 days ago (recency 0), 1 pageview (freq 0), 5s dwell (dwell 0) = 0 → cold.
        assert_eq!(
            classify_temperature(now, now - 10 * DAY, 1, 5_000),
            Temperature::Cold
        );
    }

    #[test]
    fn temperature_stale_is_cold_regardless_of_past_intensity() {
        let now = 100 * DAY;
        // Heavy past engagement but not seen for 40 days → cold (staleness gate).
        assert_eq!(
            classify_temperature(now, now - 40 * DAY, 50, 999_000),
            Temperature::Cold
        );
    }

    // ---- trend classification ----

    #[test]
    fn trend_rising_falling_and_flat() {
        let split = 50;
        // More activity in the recent half → subindo.
        assert_eq!(classify_trend(&[10, 60, 70, 80], split), Trend::Subindo);
        // More in the older half → caindo.
        assert_eq!(classify_trend(&[10, 20, 30, 60], split), Trend::Caindo);
        // Tie → estavel.
        assert_eq!(classify_trend(&[10, 20, 60, 70], split), Trend::Estavel);
        // Empty → estavel (0 == 0).
        assert_eq!(classify_trend(&[], split), Trend::Estavel);
    }

    // ---- PRIVACY INVARIANT (LGPD) ----

    #[test]
    fn anonymous_only_dataset_yields_zero_named_all_aggregate() {
        let now = 100 * DAY;
        let rows = vec![
            row(now, Some("al_v1"), None),
            row(now - DAY, Some("al_v1"), None),
            row(now, Some("al_v2"), None),
            // A row with NO identity AND no token: unattributable, must not name anyone.
            row(now, None, None),
        ];
        let digest = compute_digest(&rows, now, 14 * DAY);

        // HARD GUARANTEE: not a single named lead from anonymous telemetry.
        assert!(
            digest.named.is_empty(),
            "anonymous telemetry must NEVER produce a named lead"
        );
        // Two distinct al_vid visitors land in the aggregate bucket.
        assert_eq!(digest.aggregate.count, 2);
        let mix = &digest.aggregate.temperature_mix;
        assert_eq!(
            mix.cold + mix.warm + mix.hot,
            2,
            "every anonymous visitor is counted in the temperature mix"
        );
    }

    #[test]
    fn mixed_dataset_names_only_identified_subjects() {
        let now = 100 * DAY;
        let rows = vec![
            // Identified user "usr_alice" (via CO-490 link) → NAMED.
            row(now, Some("al_v1"), Some("usr_alice")),
            row(now - DAY, Some("al_v1"), Some("usr_alice")),
            // Identified user "usr_bob" → NAMED.
            row(now, Some("al_v9"), Some("usr_bob")),
            // Anonymous visitor → aggregate only, never named.
            row(now, Some("al_v2"), None),
            row(now - DAY, Some("al_v2"), None),
            // Empty-string user_id must be treated as anonymous, not named.
            row(now, Some("al_v3"), Some("")),
        ];
        let digest = compute_digest(&rows, now, 14 * DAY);

        let named: Vec<&str> = digest.named.iter().map(|l| l.subject.as_str()).collect();
        assert!(named.contains(&"usr_alice"));
        assert!(named.contains(&"usr_bob"));
        assert_eq!(named.len(), 2, "exactly the two identified users are named");

        // The anonymous al_v2 AND the empty-user_id al_v3 are aggregate-only.
        assert_eq!(digest.aggregate.count, 2);
        // No anonymous token ever leaks into a named subject.
        assert!(
            !named.iter().any(|s| s.starts_with("al_v")),
            "a visitor token must never surface as a named subject"
        );
    }

    #[test]
    fn named_lead_carries_temperature_trend_and_why() {
        let now = 100 * DAY;
        let mut rows = Vec::new();
        // 9 pageviews for usr_alice, all recent, on two pages across two days.
        for i in 0..8 {
            let mut r = row(now - (i % 2) * DAY, Some("al_v1"), Some("usr_alice"));
            r.path = Some(if i % 3 == 0 {
                "/precos".into()
            } else {
                "/sobre".into()
            });
            r.dwell_ms = Some(20_000);
            rows.push(r);
        }
        let digest = compute_digest(&rows, now, 14 * DAY);
        assert_eq!(digest.named.len(), 1);
        let lead = &digest.named[0];
        assert_eq!(lead.subject, "usr_alice");
        assert_eq!(lead.temperature, Temperature::Hot); // recent + 8 views + >2min dwell
        assert!(lead.why.returned, "active across two days");
        assert_eq!(lead.why.active_days, 2);
        assert_eq!(lead.why.pageviews, 8);
        assert_eq!(lead.why.language.as_deref(), Some("pt"));
        assert!(!lead.why.top_pages.is_empty());
    }

    #[test]
    fn named_leads_sorted_hottest_first() {
        let now = 100 * DAY;
        let mut rows = Vec::new();
        // Hot: many recent, long dwell.
        for _ in 0..10 {
            let mut r = row(now, Some("al_hot"), Some("usr_hot"));
            r.dwell_ms = Some(60_000);
            rows.push(r);
        }
        // Cold: one old sparse view.
        let mut cold = row(now - 12 * DAY, Some("al_cold"), Some("usr_cold"));
        cold.dwell_ms = Some(1_000);
        rows.push(cold);

        let digest = compute_digest(&rows, now, 30 * DAY);
        assert_eq!(digest.named.len(), 2);
        assert_eq!(digest.named[0].subject, "usr_hot");
        assert_eq!(digest.named[0].temperature, Temperature::Hot);
        assert_eq!(digest.named[1].subject, "usr_cold");
        assert_eq!(digest.named[1].temperature, Temperature::Cold);
    }

    // ---- properties parsing ----

    #[test]
    fn parse_props_reads_both_server_and_marketing_shapes() {
        // Server-side pageview shape.
        let (referrer, lang) = parse_props(Some(r#"{"referrer":"https://x.com"}"#));
        assert_eq!(referrer.as_deref(), Some("https://x.com"));
        assert_eq!(lang, None);
        // Marketing shape (ref + lang).
        let (referrer, lang) = parse_props(Some(r#"{"ref":"https://y.com","lang":"en"}"#));
        assert_eq!(referrer.as_deref(), Some("https://y.com"));
        assert_eq!(lang.as_deref(), Some("en"));
        // Empty / malformed → no panic, no values.
        assert_eq!(parse_props(None), (None, None));
        assert_eq!(parse_props(Some("not json")), (None, None));
        assert_eq!(parse_props(Some(r#"{"ref":""}"#)), (None, None));
    }

    // ---- CO-499 MULTI-TENANT SCOPE (load_rows filter + access gate) ----

    use crate::storage::Storage;

    fn insert_pageview(
        conn: &rusqlite::Connection,
        universe: &str,
        user: Option<&str>,
        visitor: Option<&str>,
        ts_rfc3339: &str,
    ) {
        conn.execute(
            "INSERT INTO telemetry_events \
             (timestamp, event_type, event_name, universe_key, user_id, visitor_token, path, duration_ms) \
             VALUES (?1, 'pageview', 'pageview', ?2, ?3, ?4, '/precos', 40000)",
            rusqlite::params![ts_rfc3339, universe, user, visitor],
        )
        .unwrap();
    }

    #[test]
    fn load_rows_scopes_to_the_requested_universe_only() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let conn = storage.conn();
        // Universe A: one identified user + one anonymous visitor.
        insert_pageview(conn, "uni-a", Some("usr_alice"), Some("al_a1"), &ts);
        insert_pageview(conn, "uni-a", None, Some("al_a2"), &ts);
        // Universe B: a DIFFERENT identified user — must never appear in A's digest.
        insert_pageview(conn, "uni-b", Some("usr_bob"), Some("al_b1"), &ts);

        let since = (now - chrono::Duration::days(7)).timestamp();
        let rows = load_rows(conn, since, "uni-a");

        // The `universe` param is HONORED: exactly universe A's rows return; B is
        // never leaked across tenants.
        assert_eq!(rows.len(), 2, "exactly universe A's two rows");
        let users: Vec<&str> = rows.iter().filter_map(|r| r.user_id.as_deref()).collect();
        assert!(users.contains(&"usr_alice"));
        assert!(
            !users.contains(&"usr_bob"),
            "universe B's identified user must not leak into universe A's digest"
        );

        // The anonymous-aggregate invariant still holds on the loaded rows: the
        // anonymous visitor stays aggregate-only; only the identified user is named.
        let digest = compute_digest(&rows, now.timestamp(), 7 * DAY);
        let named: Vec<&str> = digest.named.iter().map(|l| l.subject.as_str()).collect();
        assert_eq!(named, vec!["usr_alice"]);
        assert_eq!(digest.aggregate.count, 1);
    }

    #[test]
    fn ensure_read_access_admits_readers_denies_others() {
        assert!(ensure_read_access(UniverseAccess::ReadOnly).is_ok());
        assert!(ensure_read_access(UniverseAccess::ReadWrite).is_ok());
        assert!(ensure_read_access(UniverseAccess::MetadataOnly).is_err());
        assert!(ensure_read_access(UniverseAccess::LoginRequired).is_err());
        assert!(ensure_read_access(UniverseAccess::Denied).is_err());
    }

    #[test]
    fn digest_is_scoped_to_a_universe_the_caller_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        let owner = storage.create_user("owner@x.com", "Owner").unwrap();
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: "tenant-a".into(),
                    name: "Tenant A".into(),
                    description: String::new(),
                },
                &owner.id,
            )
            .unwrap();

        // Caller WITH access (the owner) is admitted.
        assert!(
            ensure_read_access(storage.check_universe_access(Some(&owner.id), "tenant-a")).is_ok(),
            "owner has read access to its own (private) universe"
        );
        // Caller WITHOUT access (a telemetry:read holder for a *different* tenant)
        // is denied — the CO-499 cross-tenant leak is closed.
        assert!(
            ensure_read_access(storage.check_universe_access(Some("intruder"), "tenant-a"))
                .is_err(),
            "a caller without read access to the universe is denied"
        );
    }

    // ---- CO-518: window param (bot sends "7d", server only read `days`) ----

    #[test]
    fn parse_window_days_accepts_bot_form_and_legacy_days() {
        // The bot's "<n>d" form.
        assert_eq!(parse_window_days(None, Some("7d")), Some(7));
        assert_eq!(parse_window_days(None, Some("30d")), Some(30));
        assert_eq!(parse_window_days(None, Some("30D")), Some(30));
        // Bare integer string, with surrounding whitespace.
        assert_eq!(parse_window_days(None, Some("  14 ")), Some(14));
        // Legacy `days` still works and WINS when both are present.
        assert_eq!(parse_window_days(Some(5), None), Some(5));
        assert_eq!(parse_window_days(Some(5), Some("30d")), Some(5));
        // Neither / invalid / non-positive → None (caller uses the default).
        assert_eq!(parse_window_days(None, None), None);
        assert_eq!(parse_window_days(None, Some("abc")), None);
        assert_eq!(parse_window_days(None, Some("0d")), None);
        assert_eq!(parse_window_days(Some(0), Some("30d")), Some(30)); // 0 days falls through to window
    }

    /// CONTRACT (CO-518): deserialize the ACTUAL query string the bot sends
    /// (`universe=acme&window=30d`) through the SAME `serde` path axum's
    /// `Query<DigestQuery>` uses, and assert it resolves to a 30-day window. This
    /// would have caught the silent-drop bug: before the fix `DigestQuery` had no
    /// `window` field, so `window=30d` was discarded and the window stayed default.
    #[test]
    fn digest_query_reads_window_param_to_30_days() {
        let q: DigestQuery =
            serde_urlencoded::from_str("universe=acme&window=30d").expect("query parses");
        assert_eq!(q.window.as_deref(), Some("30d"));
        assert_eq!(q.days, None);
        // The handler's resolution step yields a 30-day window (clamped within range).
        let window_days = parse_window_days(q.days, q.window.as_deref())
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        assert_eq!(window_days, 30);
    }
}
