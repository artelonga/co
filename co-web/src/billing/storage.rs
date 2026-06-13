//! CO-366: billing persistence helpers.
//!
//! Free functions over a `rusqlite::Connection` (the same connection held by
//! `Storage`) — call sites lock `Storage`, grab `.conn()`, run these, then drop
//! the lock before any `await`. Columns added by migration v77 are read with
//! explicit `SELECT`s (never `.ok()`-swallowed) so a missing column fails loudly
//! instead of aliasing to "row not found" (CO-137 rule).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::Plan;

/// Current billing state for a user — backs `GET /api/v1/me/billing/status`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BillingStatus {
    pub tier: String,
    pub plan: Option<String>,
    pub paid_at: Option<String>,
    pub billing_provider: Option<String>,
}

/// Flip a user to `tier = 'paid'` and record the plan / provider / external id.
/// Idempotent: re-running with the same args is harmless. Returns the number of
/// rows updated (0 if the user id is unknown).
pub fn mark_user_paid(
    conn: &Connection,
    user_id: &str,
    plan: Plan,
    provider: &str,
    external_id: Option<&str>,
    now_rfc3339: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE users \
         SET tier = 'paid', tier_plan = ?2, tier_paid_at = ?3, \
             billing_provider = ?4, billing_external_id = ?5 \
         WHERE id = ?1",
        params![user_id, plan.as_str(), now_rfc3339, provider, external_id,],
    )
}

/// Revert a user to the free tier on subscription cancellation. Keeps
/// `billing_provider` for history but clears the paid-plan markers.
pub fn mark_user_canceled(conn: &Connection, user_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE users \
         SET tier = 'free', tier_plan = 'free', tier_paid_at = NULL \
         WHERE id = ?1",
        params![user_id],
    )
}

/// Append a row to the `billing_events` audit table.
pub fn record_billing_event(
    conn: &Connection,
    user_id: &str,
    event_type: &str,
    provider: &str,
    payload_json: Option<&str>,
    now_rfc3339: &str,
) -> rusqlite::Result<()> {
    let id = format!("be_{}", nanoid::nanoid!(16));
    conn.execute(
        "INSERT INTO billing_events \
         (id, user_id, event_type, provider, payload_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, user_id, event_type, provider, payload_json, now_rfc3339],
    )?;
    Ok(())
}

/// Read a user's current billing status. Returns `None` when the user id is
/// unknown. Reads the migration-v77 columns explicitly so a missing column
/// surfaces as an error rather than `None`.
pub fn get_billing_status(
    conn: &Connection,
    user_id: &str,
) -> rusqlite::Result<Option<BillingStatus>> {
    conn.query_row(
        "SELECT tier, tier_plan, tier_paid_at, billing_provider \
         FROM users WHERE id = ?1",
        params![user_id],
        |row| {
            Ok(BillingStatus {
                tier: row.get(0)?,
                plan: row.get(1)?,
                paid_at: row.get(2)?,
                billing_provider: row.get(3)?,
            })
        },
    )
    .optional()
}

// ---------------------------------------------------------------------------
// Conversion KPI (CO-366 → surfaced in /gestao/resumo, CO-360)
// ---------------------------------------------------------------------------

/// The BaaS `t_register → t_payment` conversion KPI.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConversionKpi {
    pub registered_7d: i64,
    pub registered_30d: i64,
    pub registered_90d: i64,
    pub total_users: i64,
    pub paid_users: i64,
    /// paid / total (0.0 when there are no users).
    pub conversion_rate: f64,
    /// Mean seconds between `created_at` and `tier_paid_at` across paid users;
    /// `None` when nobody has converted yet.
    pub mean_conversion_seconds: Option<f64>,
}

/// Compute the conversion KPI from the `users` table. Reads the migration-v77
/// `tier_paid_at` column explicitly — a missing column surfaces as an error
/// rather than silently zeroing.
pub fn query_conversion_kpi(conn: &Connection) -> rusqlite::Result<ConversionKpi> {
    let count_since = |days: i64| -> rusqlite::Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM users WHERE created_at >= datetime('now', ?1)",
            params![format!("-{days} days")],
            |r| r.get(0),
        )
    };
    let registered_7d = count_since(7)?;
    let registered_30d = count_since(30)?;
    let registered_90d = count_since(90)?;
    let total_users: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    let paid_users: i64 =
        conn.query_row("SELECT COUNT(*) FROM users WHERE tier = 'paid'", [], |r| {
            r.get(0)
        })?;
    let mean_conversion_seconds: Option<f64> = conn.query_row(
        "SELECT AVG((julianday(tier_paid_at) - julianday(created_at)) * 86400.0) \
         FROM users WHERE tier_paid_at IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let conversion_rate = if total_users > 0 {
        paid_users as f64 / total_users as f64
    } else {
        0.0
    };
    Ok(ConversionKpi {
        registered_7d,
        registered_30d,
        registered_90d,
        total_users,
        paid_users,
        conversion_rate,
        mean_conversion_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY, email TEXT, display_name TEXT,
                tier TEXT NOT NULL DEFAULT 'free', created_at TEXT,
                tier_paid_at TEXT, tier_plan TEXT,
                billing_provider TEXT, billing_external_id TEXT
            );
            CREATE TABLE billing_events (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, event_type TEXT NOT NULL,
                provider TEXT NOT NULL, payload_json TEXT, created_at TEXT NOT NULL
            );
            INSERT INTO users (id, email, display_name, tier, created_at)
                VALUES ('usr_1','a@b.com','A','free','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn mark_paid_flips_tier_and_records_plan() {
        let conn = db();
        let n = mark_user_paid(
            &conn,
            "usr_1",
            Plan::Pro,
            "hostinger",
            Some("cus_42"),
            "2026-02-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(n, 1);
        let status = get_billing_status(&conn, "usr_1").unwrap().unwrap();
        assert_eq!(status.tier, "paid");
        assert_eq!(status.plan.as_deref(), Some("pro"));
        assert_eq!(status.paid_at.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert_eq!(status.billing_provider.as_deref(), Some("hostinger"));
    }

    #[test]
    fn mark_paid_unknown_user_is_zero_rows() {
        let conn = db();
        let n = mark_user_paid(&conn, "nope", Plan::Starter, "manual", None, "t").unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn cancel_reverts_to_free() {
        let conn = db();
        mark_user_paid(&conn, "usr_1", Plan::Pro, "hostinger", None, "t").unwrap();
        mark_user_canceled(&conn, "usr_1").unwrap();
        let status = get_billing_status(&conn, "usr_1").unwrap().unwrap();
        assert_eq!(status.tier, "free");
        assert_eq!(status.plan.as_deref(), Some("free"));
        assert_eq!(status.paid_at, None);
    }

    #[test]
    fn record_event_inserts_row() {
        let conn = db();
        record_billing_event(
            &conn,
            "usr_1",
            "checkout_created",
            "hostinger",
            Some(r#"{"plan":"pro"}"#),
            "2026-02-01T00:00:00Z",
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM billing_events WHERE user_id='usr_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn status_unknown_user_is_none() {
        let conn = db();
        assert_eq!(get_billing_status(&conn, "nope").unwrap(), None);
    }

    #[test]
    fn conversion_kpi_counts_and_rate() {
        let conn = db(); // 1 free user created 2026-01-01
        // Add a second user who converted to paid one day after registering.
        conn.execute_batch(
            "INSERT INTO users (id, email, display_name, tier, created_at, tier_paid_at, tier_plan)
                VALUES ('usr_2','c@d.com','C','paid','2026-03-01T00:00:00Z','2026-03-02T00:00:00Z','pro');",
        )
        .unwrap();
        let kpi = query_conversion_kpi(&conn).unwrap();
        assert_eq!(kpi.total_users, 2);
        assert_eq!(kpi.paid_users, 1);
        assert!((kpi.conversion_rate - 0.5).abs() < 1e-9);
        // ~1 day between register and payment.
        let mean = kpi.mean_conversion_seconds.unwrap();
        assert!((mean - 86400.0).abs() < 1.0, "expected ~86400s, got {mean}");
    }

    #[test]
    fn conversion_kpi_empty_is_zero() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY, created_at TEXT, tier TEXT, tier_paid_at TEXT
            );",
        )
        .unwrap();
        let kpi = query_conversion_kpi(&conn).unwrap();
        assert_eq!(kpi.total_users, 0);
        assert_eq!(kpi.paid_users, 0);
        assert_eq!(kpi.conversion_rate, 0.0);
        assert_eq!(kpi.mean_conversion_seconds, None);
    }
}
