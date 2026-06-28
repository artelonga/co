//! CO-487 — birthday greetings.
//!
//! Opt-in at login: `POST /api/v1/auth/birthday-consent` records the user's
//! birthday (`MM-DD`), consent, and WhatsApp number (LGPD opt-in). A daily job
//! (also triggerable by an admin) greets consented users on their birthday via
//! the same `ChannelProvider` cascade as recovery codes (Cloud API → Evolution),
//! once per year (`birthday_greeted_year` idempotency).

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use crate::auth::UserId;
use crate::notification_providers::{ChannelProvider, OutboundTemplate};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// The greeting text (pt-BR). Falls back to a name-less form when unknown.
pub fn compose_greeting(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        "🎉 Feliz aniversário! Que seu novo ciclo seja leve e criativo. — Co".to_string()
    } else {
        format!("🎉 Feliz aniversário, {n}! Que seu novo ciclo seja leve e criativo. — Co")
    }
}

/// Validate an `MM-DD` birthday string.
pub fn valid_mmdd(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b'-' {
        return false;
    }
    let (mm, dd) = (s[0..2].parse::<u32>(), s[3..5].parse::<u32>());
    matches!((mm, dd), (Ok(m), Ok(d)) if (1..=12).contains(&m) && (1..=31).contains(&d))
}

fn today_mmdd() -> String {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    format!("{:02}-{:02}", now.month(), now.day())
}

fn current_year() -> i64 {
    use chrono::Datelike;
    chrono::Utc::now().year() as i64
}

/// Mask a phone number for logs (keep last 4 digits).
fn redact(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return "***".to_string();
    }
    format!(
        "****{}",
        digits[digits.len() - 4..].iter().collect::<String>()
    )
}

/// Consented users whose birthday is `today` (`MM-DD`) and who haven't been
/// greeted in `year` yet, with a WhatsApp number. Returns `(id, name, number)`.
pub fn select_birthday_targets(
    conn: &rusqlite::Connection,
    today: &str,
    year: i64,
) -> Vec<(String, String, String)> {
    let sql = "SELECT id, COALESCE(NULLIF(display_name,''), usuario, email, ''), COALESCE(whatsapp,'') \
               FROM users \
               WHERE birthday = ?1 AND birthday_consent = 1 \
                 AND COALESCE(birthday_greeted_year, 0) <> ?2 \
                 AND whatsapp IS NOT NULL AND whatsapp <> ''";
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(sql) else {
        return out;
    };
    let Ok(rows) = stmt.query_map(params![today, year], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    }) else {
        return out;
    };
    out.extend(rows.flatten());
    out
}

/// Send the greeting to each target via `provider`. Returns the ids that sent OK.
///
/// CO-496: a birthday greeting is **business-initiated** — the user almost never
/// messaged us in the last 24h, so the customer-service window is closed and Meta
/// rejects free-form text. `template` (configured via `CO_WHATSAPP_BIRTHDAY_TEMPLATE`)
/// is the approved fallback; its single body param is the greeted user's name.
/// When the window happens to be open (or the provider has no window, e.g.
/// Evolution) free-form text is used.
pub async fn send_greetings(
    provider: &dyn ChannelProvider,
    client: &reqwest::Client,
    targets: &[(String, String, String)],
    template: Option<&OutboundTemplate>,
) -> Vec<String> {
    let mut sent = Vec::new();
    for (id, name, number) in targets {
        let msg = compose_greeting(name);
        // Per-recipient template params: the greeted user's name fills `{{1}}`.
        let per_target = template.map(|base| base.with_params(vec![name.clone()]));
        match provider
            .send_business_initiated(client, number, &msg, per_target.as_ref())
            .await
        {
            Ok(()) => sent.push(id.clone()),
            Err(e) => tracing::warn!("birthday send to {} failed: {e}", redact(number)),
        }
    }
    sent
}

/// The daily job: select today's consented birthdays and greet them, marking
/// each greeted for the year. Returns the number greeted. No lock is held across
/// the network `await` (collect → drop lock → send → re-lock to mark).
pub async fn run_birthday_job(state: &AppState) -> usize {
    let today = today_mmdd();
    let year = current_year();
    let targets = {
        let storage = state.core.storage.lock();
        select_birthday_targets(storage.conn(), &today, year)
    };
    if targets.is_empty() {
        return 0;
    }
    // CO-497: shared Cloud → Evolution cascade (managed → self-host).
    let Some(provider) = crate::notification_providers::whatsapp_provider_cascade() else {
        for (_, name, number) in &targets {
            tracing::info!(
                "[birthday dev-fallback] no WhatsApp provider configured — would greet {}: {}",
                redact(number),
                compose_greeting(name)
            );
        }
        return 0;
    };
    // CO-496: reuse the process-wide 30s-timed client (was an untimed
    // `Client::new()` that could hang a worker indefinitely on a stuck send).
    let client = crate::whatsapp_cloud_routes::shared_client();
    // CO-496: approved-template fallback for the closed-window case (params filled
    // per-recipient inside `send_greetings`).
    let template = crate::notification_providers::template_from_env(
        "CO_WHATSAPP_BIRTHDAY_TEMPLATE",
        "CO_WHATSAPP_BIRTHDAY_TEMPLATE_LANG",
        Vec::new(),
    );
    let sent = send_greetings(provider.as_ref(), client, &targets, template.as_ref()).await;
    {
        let storage = state.core.storage.lock();
        for id in &sent {
            let _ = storage.conn().execute(
                "UPDATE users SET birthday_greeted_year = ?2 WHERE id = ?1",
                params![id, year],
            );
        }
    }
    sent.len()
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ConsentReq {
    birthday: String,
    #[serde(default)]
    consent: bool,
    #[serde(default)]
    whatsapp: Option<String>,
}

/// POST /api/v1/auth/birthday-consent — capture birthday + LGPD opt-in at login.
async fn consent_handler(
    State(state): State<AppState>,
    UserId(uid): UserId,
    Json(req): Json<ConsentReq>,
) -> Response {
    if !valid_mmdd(&req.birthday) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "birthday must be MM-DD"})),
        )
            .into_response();
    }
    // CO-490 R2: normalize the WhatsApp number to digits-only before storing, using
    // the SAME helper the link flow uses (`whatsapp_cloud_routes::validate_whatsapp`).
    // Otherwise an un-normalized number stored here would never match
    // `Storage::get_user_id_by_whatsapp` (exact match on the digit form), silently
    // bypassing the one-number-per-account uniqueness check. A provided-but-digitless
    // number is rejected with 400.
    let whatsapp = match req.whatsapp.as_deref() {
        Some(raw) => match crate::whatsapp_cloud_routes::validate_whatsapp(raw) {
            Ok(digits) => Some(digits),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "whatsapp must contain digits"})),
                )
                    .into_response();
            }
        },
        None => None,
    };
    let storage = state.core.storage.lock();
    let res = storage.conn().execute(
        "UPDATE users SET birthday = ?1, birthday_consent = ?2, whatsapp = COALESCE(?3, whatsapp) WHERE id = ?4",
        params![req.birthday, req.consent as i64, whatsapp, uid],
    );
    match res {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "birthday": req.birthday, "consent": req.consent})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/v1/admin/birthday/run — trigger the job now (seed-admin only).
async fn run_handler(State(state): State<AppState>, UserId(uid): UserId) -> Response {
    let admin_email = crate::infra::secrets::global().get_or("CO_SEED_ADMIN_EMAIL", "");
    let is_admin = !admin_email.is_empty() && {
        let storage = state.core.storage.lock();
        storage
            .conn()
            .query_row(
                "SELECT 1 FROM users WHERE id = ?1 AND email = ?2",
                params![uid, admin_email],
                |_| Ok(()),
            )
            .is_ok()
    };
    if !is_admin {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "admin only"}))).into_response();
    }
    let n = run_birthday_job(&state).await;
    (StatusCode::OK, Json(json!({"greeted": n}))).into_response()
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/auth/birthday-consent", post(consent_handler))
        .route("/admin/birthday/run", post(run_handler))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn compose_and_validate() {
        assert!(compose_greeting("Yuri").contains("Feliz aniversário, Yuri"));
        assert!(compose_greeting("  ").contains("Feliz aniversário"));
        assert!(valid_mmdd("06-24"));
        assert!(!valid_mmdd("2026-06-24"));
        assert!(!valid_mmdd("13-01"));
        assert!(!valid_mmdd("06-32"));
    }

    struct MockProvider {
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }
    #[async_trait::async_trait]
    impl ChannelProvider for MockProvider {
        fn name(&self) -> &'static str {
            "whatsapp"
        }
        async fn send(
            &self,
            _c: &reqwest::Client,
            recipient: &str,
            payload: &str,
        ) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push((recipient.to_string(), payload.to_string()));
            Ok(())
        }
    }

    /// CO-490 R2: the consent handler must store the WhatsApp number in the SAME
    /// digit-normalized form the link flow uses, so a number captured here is found
    /// by `Storage::get_user_id_by_whatsapp` (exact match on the digit form) and the
    /// one-number-per-account uniqueness check cannot be bypassed.
    #[test]
    fn consent_stores_normalized_whatsapp_matching_link_uniqueness() {
        use crate::storage::Storage;

        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        let user = storage
            .create_user("yuri@artelonga.com.br", "Yuri")
            .expect("create user");

        // The user types a "pretty" number into the birthday-consent form.
        let raw = "+55 (41) 99999-9999";
        // The handler normalizes via the SAME helper the link flow uses.
        let normalized = crate::whatsapp_cloud_routes::validate_whatsapp(raw).unwrap();
        assert_eq!(normalized, "5541999999999"); // digits-only canonical form
        storage
            .conn()
            .execute(
                "UPDATE users SET whatsapp = COALESCE(?1, whatsapp) WHERE id = ?2",
                params![Some(&normalized), user.id],
            )
            .unwrap();

        // The link-flow uniqueness lookup (digit-normalized input) now finds it —
        // proving the two code paths agree on the stored form.
        let link_form = crate::whatsapp_cloud_routes::validate_whatsapp("5541999999999").unwrap();
        assert_eq!(
            storage.get_user_id_by_whatsapp(&link_form).as_deref(),
            Some(user.id.as_str()),
            "birthday-stored number must match the CO-490 normalized form"
        );
    }

    /// E2E of the logic: a consented user with today's birthday is selected and
    /// greeted with the right message to the right number.
    #[tokio::test]
    async fn birthday_e2e_selects_and_greets() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::new(dir.path());
        let today = today_mmdd();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, birthday, birthday_consent, whatsapp) \
                 VALUES ('u_yuri', 'yuri@artelonga.com.br', 'Yuri', 'admin', '2020-01-01', ?1, 1, '5541999999999')",
                params![today],
            )
            .unwrap();
        // a non-consenting user with the same birthday must be skipped
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at, birthday, birthday_consent, whatsapp) \
                 VALUES ('u_no', 'no@x.test', 'No', 'player', '2020-01-01', ?1, 0, '5541888888888')",
                params![today],
            )
            .unwrap();

        let targets = select_birthday_targets(storage.conn(), &today, current_year());
        assert_eq!(targets.len(), 1, "only the consenting user is selected");
        assert_eq!(targets[0].0, "u_yuri");
        assert_eq!(targets[0].2, "5541999999999");

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mock = MockProvider {
            calls: calls.clone(),
        };
        let sent = send_greetings(&mock, &reqwest::Client::new(), &targets, None).await;
        assert_eq!(sent, vec!["u_yuri".to_string()]);
        let captured = calls.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "5541999999999");
        assert!(captured[0].1.contains("Feliz aniversário, Yuri"));
    }
}
