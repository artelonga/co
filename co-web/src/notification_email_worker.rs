//! CO-200: background email digest worker.
//!
//! Spawned at startup; runs a 60-second tick loop and fans out per-user email
//! digests based on each user's `email_digest_freq` preference + quiet hours.

use std::collections::HashMap;

use chrono::{DateTime, Timelike, Utc};

use crate::server::AppState;
use crate::storage::notifications::{NotificationPreferences, NotificationWithActor};

const MAX_PER_DIGEST: usize = 50;
const MAX_FAILURES: u32 = 5;

pub async fn run(state: AppState) {
    tracing::info!("notif email worker active");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut failure_counts: HashMap<String, u32> = HashMap::new();
    loop {
        interval.tick().await;
        if let Err(e) = tick(&state, &mut failure_counts).await {
            tracing::warn!("notif email tick: {e}");
        }
    }
}

async fn tick(state: &AppState, failure_counts: &mut HashMap<String, u32>) -> anyhow::Result<()> {
    let now = Utc::now();

    let users = {
        let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
        storage.list_users_with_pending_email_notifications()
    };

    for (user_id, email, display_name, lang) in users {
        if email.is_empty() {
            continue;
        }

        let failures = failure_counts.get(&user_id).copied().unwrap_or(0);
        if failures >= MAX_FAILURES {
            continue;
        }

        let (prefs, batch) = {
            let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
            let prefs = storage.get_preferences(&user_id);
            let batch = storage.list_pending_email_notifications(&user_id, MAX_PER_DIGEST);
            (prefs, batch)
        };

        if batch.is_empty() {
            continue;
        }

        let filtered: Vec<&NotificationWithActor> = filter_by_prefs(&batch, &prefs);
        if filtered.is_empty() {
            continue;
        }

        let oldest_created_at = filtered
            .iter()
            .map(|n| n.created_at.as_str())
            .min()
            .unwrap_or("");

        if !should_send_now(&prefs, oldest_created_at, now) {
            continue;
        }

        let ids: Vec<String> = filtered.iter().map(|n| n.id.clone()).collect();

        match send_digest_email(&email, &display_name, &filtered, &lang).await {
            Ok(()) => {
                failure_counts.remove(&user_id);
                let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
                if let Err(e) = storage.mark_notifications_email_delivered(&ids) {
                    tracing::warn!("mark_delivered for {user_id}: {e}");
                }
            }
            Err(e) => {
                let count = failure_counts.entry(user_id.clone()).or_insert(0);
                *count += 1;
                tracing::warn!("digest send for {user_id} (attempt {}): {e}", *count);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Frequency + quiet-hours policy
// ---------------------------------------------------------------------------

/// Returns true if a digest should be sent now based on frequency preference
/// and quiet hours.
pub fn should_send_now(
    prefs: &NotificationPreferences,
    oldest_created_at: &str,
    now: DateTime<Utc>,
) -> bool {
    if prefs.email_digest_freq == "never" {
        return false;
    }

    if is_in_quiet_hours(prefs, now) {
        return false;
    }

    let oldest_dt = chrono::DateTime::parse_from_rfc3339(oldest_created_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now);

    let age_secs = (now - oldest_dt).num_seconds().max(0);

    match prefs.email_digest_freq.as_str() {
        "instant" => true,
        "hourly" => age_secs >= 55 * 60,
        "daily" => age_secs >= 23 * 3600,
        "weekly" => age_secs >= 6 * 86400 + 12 * 3600,
        _ => false,
    }
}

/// Returns true if `now` falls within the user's quiet-hours window (timezone-aware).
pub fn is_in_quiet_hours(prefs: &NotificationPreferences, now: DateTime<Utc>) -> bool {
    let (Some(start_str), Some(end_str)) = (&prefs.quiet_hours_start, &prefs.quiet_hours_end)
    else {
        return false;
    };

    let offset = tz_to_fixed_offset(&prefs.quiet_hours_tz);
    let local = now.with_timezone(&offset);
    let local_mins = local.hour() * 60 + local.minute();
    let start_mins = parse_hhmm(start_str);
    let end_mins = parse_hhmm(end_str);

    if start_mins < end_mins {
        // Normal range (e.g., 09:00–18:00)
        local_mins >= start_mins && local_mins < end_mins
    } else {
        // Overnight range (e.g., 22:00–07:00)
        local_mins >= start_mins || local_mins < end_mins
    }
}

fn parse_hhmm(s: &str) -> u32 {
    let mut parts = s.splitn(2, ':');
    let h = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let m = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    h * 60 + m
}

fn tz_to_fixed_offset(tz: &str) -> chrono::FixedOffset {
    match tz {
        "UTC" | "Etc/UTC" | "GMT" | "Etc/GMT" => chrono::FixedOffset::east_opt(0).unwrap(),
        "America/Sao_Paulo" | "America/Fortaleza" | "America/Belem" | "America/Recife"
        | "America/Maceio" | "America/Santarem" => chrono::FixedOffset::west_opt(3 * 3600).unwrap(),
        "America/Manaus" | "America/Porto_Velho" | "America/Boa_Vista" => {
            chrono::FixedOffset::west_opt(4 * 3600).unwrap()
        }
        "America/Rio_Branco" | "America/Eirunepe" => {
            chrono::FixedOffset::west_opt(5 * 3600).unwrap()
        }
        "America/New_York" | "America/Toronto" => chrono::FixedOffset::west_opt(5 * 3600).unwrap(),
        "America/Chicago" | "America/Winnipeg" => chrono::FixedOffset::west_opt(6 * 3600).unwrap(),
        "America/Denver" | "America/Phoenix" => chrono::FixedOffset::west_opt(7 * 3600).unwrap(),
        "America/Los_Angeles" | "America/Vancouver" => {
            chrono::FixedOffset::west_opt(8 * 3600).unwrap()
        }
        "Europe/Lisbon" | "Europe/London" => chrono::FixedOffset::east_opt(0).unwrap(),
        "Europe/Paris" | "Europe/Berlin" | "Europe/Madrid" | "Europe/Rome" => {
            chrono::FixedOffset::east_opt(3600).unwrap()
        }
        "Europe/Helsinki" | "Europe/Athens" | "Europe/Bucharest" => {
            chrono::FixedOffset::east_opt(2 * 3600).unwrap()
        }
        _ => chrono::FixedOffset::east_opt(0).unwrap(),
    }
}

// ---------------------------------------------------------------------------
// Preference filtering
// ---------------------------------------------------------------------------

/// Keep only notifications whose event_type is enabled in the user's prefs.
pub fn filter_by_prefs<'a>(
    batch: &'a [NotificationWithActor],
    prefs: &NotificationPreferences,
) -> Vec<&'a NotificationWithActor> {
    batch
        .iter()
        .filter(|n| match n.event_type.as_str() {
            "chat.message" => prefs.email_chat_message,
            "chat.dm" => prefs.email_chat_dm,
            "chat.mention" => prefs.email_mention,
            "universe.invitation" => prefs.email_invitation,
            _ => true,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Email content
// ---------------------------------------------------------------------------

/// Build the notification deep-link URL.
pub fn notif_link(base_url: &str, notif: &NotificationWithActor) -> String {
    let base = base_url.trim_end_matches('/');
    match notif.event_type.as_str() {
        "chat.message" | "chat.mention" => {
            let uk = notif.context.universe_key.as_deref().unwrap_or("");
            let room = notif.context.room_id.as_deref().unwrap_or("");
            let obj = notif.context.object_id.as_deref().unwrap_or("");
            format!("{base}/{uk}#chat-{room}-{obj}")
        }
        "chat.dm" => {
            let room = notif.context.room_id.as_deref().unwrap_or("");
            let obj = notif.context.object_id.as_deref().unwrap_or("");
            format!("{base}/?dm={room}#msg-{obj}")
        }
        "universe.invitation" => {
            let obj = notif.context.object_id.as_deref().unwrap_or("");
            format!("{base}/invitations/{obj}")
        }
        _ => base.to_string(),
    }
}

/// Build the localized email subject.
pub fn build_subject(notifs: &[&NotificationWithActor], lang: &str) -> String {
    if notifs.is_empty() {
        return match lang {
            "en" => "CO notifications".to_string(),
            _ => "Notificações no CO".to_string(),
        };
    }

    if notifs.len() == 1 {
        let actor_name = notifs[0]
            .actor
            .as_ref()
            .map(|a| a.display_name.as_str())
            .unwrap_or("CO");
        return match (notifs[0].event_type.as_str(), lang) {
            ("chat.dm", "en") => format!("{actor_name} sent you a message in CO"),
            ("chat.dm", _) => format!("{actor_name} te enviou uma mensagem no CO"),
            ("universe.invitation", "en") => {
                format!("{actor_name} invited you to a universe in CO")
            }
            ("universe.invitation", _) => {
                format!("{actor_name} te convidou para um universo no CO")
            }
            ("chat.mention", "en") => format!("{actor_name} mentioned you in CO"),
            ("chat.mention", _) => format!("{actor_name} mencionou você no CO"),
            (_, "en") => format!("{actor_name} sent a message in CO"),
            (_, _) => format!("{actor_name} te enviou uma mensagem no CO"),
        };
    }

    let n = notifs.len();
    match lang {
        "en" => format!("{n} notifications in CO"),
        _ => format!("{n} notificações no CO"),
    }
}

fn notif_summary_text(notif: &NotificationWithActor, lang: &str) -> String {
    let actor_name = notif
        .actor
        .as_ref()
        .map(|a| a.display_name.as_str())
        .unwrap_or("CO");
    match (notif.summary.key.as_str(), lang) {
        ("notif.chat.dm", "en") => format!("{actor_name} sent you a message"),
        ("notif.chat.dm", _) => format!("{actor_name} te enviou uma mensagem"),
        ("notif.chat.message", "en") => {
            let universe = notif
                .summary
                .params
                .get("universe")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("New message in {universe}")
        }
        ("notif.chat.message", _) => {
            let universe = notif
                .summary
                .params
                .get("universe")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Nova mensagem em {universe}")
        }
        ("notif.universe.invitation", "en") => {
            let universe = notif
                .summary
                .params
                .get("universe")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{actor_name} invited you to {universe}")
        }
        ("notif.universe.invitation", _) => {
            let universe = notif
                .summary
                .params
                .get("universe")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{actor_name} te convidou para {universe}")
        }
        ("notif.chat.mention", "en") => {
            let room = notif
                .summary
                .params
                .get("room")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{actor_name} mentioned you in {room}")
        }
        ("notif.chat.mention", _) => {
            let room = notif
                .summary
                .params
                .get("room")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{actor_name} mencionou você em {room}")
        }
        (key, _) => key.to_string(),
    }
}

/// Build the CSS-inlined HTML body for the digest email.
pub fn build_body_html(
    notifs: &[&NotificationWithActor],
    display_name: &str,
    lang: &str,
    base_url: &str,
) -> String {
    let settings_url = format!("{}/settings", base_url.trim_end_matches('/'));

    let (greeting, pending_line, link_label, footer_msg, team_sig) = match lang {
        "en" => (
            format!("Hello {display_name},"),
            format!("You have {} pending notification(s):", notifs.len()),
            "View",
            format!("To change notification frequency, visit: {settings_url}"),
            "— CO team",
        ),
        _ => (
            format!("Olá {display_name},"),
            format!("Você tem {} notificação(ões) pendente(s):", notifs.len()),
            "Ver",
            format!("Para alterar suas preferências de notificação, visite: {settings_url}"),
            "— equipe CO",
        ),
    };

    let mut items = String::new();
    for notif in notifs {
        let link = notif_link(base_url, notif);
        let summary = notif_summary_text(notif, lang);
        items.push_str(&format!(
            "<li style=\"margin-bottom:12px;\">\
             <strong style=\"color:#111;\">{summary}</strong><br>\
             <a href=\"{link}\" style=\"color:#4f46e5;text-decoration:none;\">{link_label} →</a>\
             </li>\n"
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<style>
body{{font-family:system-ui,sans-serif;background:#fff;color:#111;max-width:600px;margin:40px auto;padding:20px;line-height:1.5}}
ul{{padding-left:20px}}
a{{color:#4f46e5}}
.footer{{margin-top:32px;color:#666;font-size:.85em;border-top:1px solid #eee;padding-top:16px}}
@media(prefers-color-scheme:dark){{body{{background:#1a1a1a;color:#eee}}a{{color:#818cf8}}.footer{{border-color:#333}}strong{{color:#eee!important}}}}
</style>
</head>
<body>
<p>{greeting}</p>
<p>{pending_line}</p>
<ul>
{items}</ul>
<div class="footer">
<p>{footer_msg}</p>
<p>{team_sig}</p>
</div>
</body>
</html>"#,
        lang = lang,
        greeting = greeting,
        pending_line = pending_line,
        items = items,
        footer_msg = footer_msg,
        team_sig = team_sig,
    )
}

// ---------------------------------------------------------------------------
// Delivery cascade: Resend → SMTP → log
// ---------------------------------------------------------------------------

async fn send_digest_email(
    email: &str,
    display_name: &str,
    notifs: &[&NotificationWithActor],
    lang: &str,
) -> anyhow::Result<()> {
    let base_url = "https://co.artelonga.com.br";
    let from = std::env::var("NOTIF_FROM_EMAIL")
        .unwrap_or_else(|_| "notificacoes@seguranca.artelonga.com.br".to_string());
    let subject = build_subject(notifs, lang);
    let body_html = build_body_html(notifs, display_name, lang, base_url);

    // 1. Try Resend (HTML)
    if let Ok(api_key) = std::env::var("RESEND_API_KEY") {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "from": from,
            "to": [email],
            "subject": subject,
            "html": body_html,
        });
        match client
            .post("https://api.resend.com/emails")
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("digest sent to {email} via Resend");
                return Ok(());
            }
            Ok(resp) => {
                tracing::warn!(
                    "Resend digest to {email}: {} {}",
                    resp.status(),
                    resp.text().await.unwrap_or_default()
                );
            }
            Err(e) => tracing::warn!("Resend request for {email}: {e}"),
        }
    }

    // 2. Try SMTP (HTML)
    if let Ok(true) = crate::email_smtp::send_html_email(email, &from, &subject, &body_html).await {
        tracing::info!("digest sent to {email} via SMTP");
        return Ok(());
    }

    // 3. Log only (dev / no provider configured)
    tracing::info!(
        "digest [no-provider] to={email} subject={subject:?} (HTML body, {} notifs)",
        notifs.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    use crate::storage::notifications::{
        NotificationActor, NotificationContext, NotificationSummary,
    };

    fn make_prefs(freq: &str) -> NotificationPreferences {
        NotificationPreferences {
            user_id: "usr_test".into(),
            email_chat_message: true,
            email_chat_dm: true,
            email_invitation: true,
            email_mention: true,
            email_digest_freq: freq.into(),
            push_chat_message: true,
            push_chat_dm: true,
            push_invitation: true,
            push_mention: true,
            in_app_chat_message: true,
            in_app_chat_dm: true,
            in_app_invitation: true,
            in_app_mention: true,
            quiet_hours_start: None,
            quiet_hours_end: None,
            quiet_hours_tz: "UTC".into(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn make_notif(id: &str, event_type: &str, created_at: &str) -> NotificationWithActor {
        NotificationWithActor {
            id: id.into(),
            event_type: event_type.into(),
            actor: Some(NotificationActor {
                user_id: "usr_actor".into(),
                display_name: "Maria".into(),
                usuario: Some("maria".into()),
            }),
            summary: NotificationSummary {
                key: format!("notif.{event_type}"),
                params: serde_json::json!({}),
            },
            context: NotificationContext {
                universe_key: Some("myuniverse".into()),
                room_id: Some("room_123".into()),
                object_id: Some("msg_456".into()),
            },
            created_at: created_at.into(),
            read_at: None,
        }
    }

    // -- should_send_now --

    #[test]
    fn test_should_send_instant_with_pending() {
        let prefs = make_prefs("instant");
        let now = Utc::now();
        let created_at = (now - chrono::Duration::seconds(10)).to_rfc3339();
        assert!(should_send_now(&prefs, &created_at, now));
    }

    #[test]
    fn test_should_send_hourly_oldest_55min() {
        let prefs = make_prefs("hourly");
        let now = Utc::now();
        let created_at = (now - chrono::Duration::minutes(56)).to_rfc3339();
        assert!(should_send_now(&prefs, &created_at, now));
    }

    #[test]
    fn test_should_send_daily_oldest_23h() {
        let prefs = make_prefs("daily");
        let now = Utc::now();
        let created_at = (now - chrono::Duration::hours(24)).to_rfc3339();
        assert!(should_send_now(&prefs, &created_at, now));
    }

    #[test]
    fn test_should_send_never_returns_false() {
        let prefs = make_prefs("never");
        let now = Utc::now();
        let created_at = (now - chrono::Duration::hours(100)).to_rfc3339();
        assert!(!should_send_now(&prefs, &created_at, now));
    }

    // -- quiet hours --

    #[test]
    fn test_quiet_hours_normal_range_skips() {
        let mut prefs = make_prefs("instant");
        prefs.quiet_hours_start = Some("09:00".into());
        prefs.quiet_hours_end = Some("18:00".into());
        prefs.quiet_hours_tz = "UTC".into();
        // 12:00 UTC falls inside 09:00–18:00
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        assert!(is_in_quiet_hours(&prefs, now));
        // should_send_now returns false during quiet hours
        let created_at = (now - chrono::Duration::seconds(5)).to_rfc3339();
        assert!(!should_send_now(&prefs, &created_at, now));
    }

    #[test]
    fn test_quiet_hours_overnight_range_skips() {
        let mut prefs = make_prefs("instant");
        prefs.quiet_hours_start = Some("22:00".into());
        prefs.quiet_hours_end = Some("07:00".into());
        prefs.quiet_hours_tz = "UTC".into();
        // 23:30 UTC is inside the overnight window
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 23, 30, 0).unwrap();
        assert!(is_in_quiet_hours(&prefs, now));
    }

    #[test]
    fn test_quiet_hours_outside_range_allows() {
        let mut prefs = make_prefs("instant");
        prefs.quiet_hours_start = Some("22:00".into());
        prefs.quiet_hours_end = Some("07:00".into());
        prefs.quiet_hours_tz = "UTC".into();
        // 14:00 UTC is outside the overnight window
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 14, 0, 0).unwrap();
        assert!(!is_in_quiet_hours(&prefs, now));
    }

    // -- localization --

    #[test]
    fn test_subject_localization_pt() {
        let n = make_notif("n1", "chat.dm", &Utc::now().to_rfc3339());
        let subject = build_subject(&[&n], "pt");
        assert!(
            subject.contains("Maria"),
            "subject should include actor name"
        );
        assert!(
            subject.contains("mensagem") || subject.contains("CO"),
            "subject should be Portuguese: {subject}"
        );
    }

    #[test]
    fn test_subject_localization_en() {
        let n = make_notif("n1", "chat.dm", &Utc::now().to_rfc3339());
        let subject = build_subject(&[&n], "en");
        assert!(
            subject.contains("Maria"),
            "subject should include actor name"
        );
        assert!(
            subject.contains("message") || subject.contains("CO"),
            "subject should be English: {subject}"
        );
    }

    // -- link targets --

    #[test]
    fn test_body_link_targets_correct_for_each_event_type() {
        let base = "https://co.example.com";

        let chat_msg = make_notif("n1", "chat.message", &Utc::now().to_rfc3339());
        let link = notif_link(base, &chat_msg);
        assert!(link.contains("myuniverse"), "chat.message link: {link}");
        assert!(
            link.contains("chat-room_123-msg_456"),
            "chat.message anchor: {link}"
        );

        let dm = make_notif("n2", "chat.dm", &Utc::now().to_rfc3339());
        let link = notif_link(base, &dm);
        assert!(link.contains("dm=room_123"), "chat.dm link: {link}");
        assert!(link.contains("msg-msg_456"), "chat.dm anchor: {link}");

        let invitation = NotificationWithActor {
            id: "n3".into(),
            event_type: "universe.invitation".into(),
            actor: None,
            summary: NotificationSummary {
                key: "notif.universe.invitation".into(),
                params: serde_json::json!({}),
            },
            context: NotificationContext {
                universe_key: None,
                room_id: None,
                object_id: Some("inv_789".into()),
            },
            created_at: Utc::now().to_rfc3339(),
            read_at: None,
        };
        let link = notif_link(base, &invitation);
        assert!(
            link.contains("invitations/inv_789"),
            "invitation link: {link}"
        );
    }

    // -- filter_by_prefs --

    #[test]
    fn test_filter_by_prefs_excludes_disabled_event_types() {
        let mut prefs = make_prefs("daily");
        prefs.email_chat_message = false;

        let chat_msg = make_notif("n1", "chat.message", &Utc::now().to_rfc3339());
        let dm = make_notif("n2", "chat.dm", &Utc::now().to_rfc3339());
        let batch = vec![chat_msg, dm];

        let filtered = filter_by_prefs(&batch, &prefs);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_type, "chat.dm");
    }

    // -- mark_delivered storage test --

    #[test]
    fn test_mark_delivered_only_marks_sent_batch() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::new(dir.path());
        let now = Utc::now().to_rfc3339();

        storage
            .conn()
            .execute(
                "INSERT INTO users (id, email, display_name, tier, created_at) \
                 VALUES ('usr_t1', 'a@example.com', 'A', 'player', ?1)",
                rusqlite::params![now],
            )
            .unwrap();

        let summary = r#"{"key":"notif.chat.dm","params":{}}"#;
        for (id, obj) in [("notif_a", "obj1"), ("notif_b", "obj2")] {
            storage
                .conn()
                .execute(
                    "INSERT INTO user_notifications \
                     (id, user_id, event_type, actor_id, object_id, summary, created_at) \
                     VALUES (?1, 'usr_t1', 'chat.dm', 'usr_actor', ?2, ?3, ?4)",
                    rusqlite::params![id, obj, summary, now],
                )
                .unwrap();
        }

        storage
            .mark_notifications_email_delivered(&["notif_a".to_string()])
            .unwrap();

        let delivered: Option<String> = storage
            .conn()
            .query_row(
                "SELECT delivered_email_at FROM user_notifications WHERE id = 'notif_a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(delivered.is_some(), "notif_a must be marked delivered");

        let not_delivered: Option<String> = storage
            .conn()
            .query_row(
                "SELECT delivered_email_at FROM user_notifications WHERE id = 'notif_b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            not_delivered.is_none(),
            "notif_b must NOT be marked delivered"
        );
    }
}
