//! CO-201: background web push delivery worker.
//!
//! Spawned at startup; ticks every 10 seconds and delivers unread notifications
//! to each user's registered push subscriptions via VAPID-signed Web Push.
//! Respects quiet hours and per-user push preferences from CO-199.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::notification_email_worker::is_in_quiet_hours;
use crate::server::AppState;
use crate::storage::notifications::{NotificationPreferences, NotificationWithActor};
use crate::storage::push_subscriptions::PushSubscription;

const MAX_PER_USER: usize = 20;
pub const MAX_FAILURES: i64 = 5;

pub async fn run(state: AppState) {
    tracing::info!("notif push worker active");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
    loop {
        interval.tick().await;
        if let Err(e) = tick(&state, Utc::now()).await {
            tracing::warn!("notif push tick: {e}");
        }
    }
}

pub async fn tick(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<()> {
    let vapid_private_key = std::env::var("VAPID_PRIVATE_KEY").unwrap_or_default();
    let vapid_subject = std::env::var("VAPID_SUBJECT")
        .unwrap_or_else(|_| "mailto:noreply@co.artelonga.com.br".to_string());

    let users = {
        let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
        storage.list_users_with_pending_push_notifications()
    };

    for (user_id, _display_name) in users {
        let (prefs, batch, subscriptions) = {
            let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
            let prefs = storage.get_preferences(&user_id);
            let batch = storage.list_pending_push_notifications(&user_id, MAX_PER_USER);
            let subscriptions = storage.list_all_push_subscriptions_for_user(&user_id);
            (prefs, batch, subscriptions)
        };

        if batch.is_empty() || subscriptions.is_empty() {
            continue;
        }

        if is_in_quiet_hours(&prefs, now) {
            continue;
        }

        let filtered: Vec<&NotificationWithActor> = filter_by_push_prefs(&batch, &prefs);
        if filtered.is_empty() {
            continue;
        }

        let mut delivered_ids: Vec<String> = Vec::new();

        for notif in &filtered {
            let payload = build_push_payload(notif);

            for sub in &subscriptions {
                if sub.failure_count >= MAX_FAILURES {
                    continue;
                }

                let outcome = if vapid_private_key.is_empty() {
                    tracing::info!(
                        "push [dev] user={} notif={} sub={}",
                        user_id,
                        notif.id,
                        &sub.id
                    );
                    DeliveryOutcome::Success
                } else {
                    deliver_push(sub, &payload, &vapid_private_key, &vapid_subject).await
                };

                let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
                match outcome {
                    DeliveryOutcome::Success => {
                        let _ = storage.update_push_subscription_last_success(&sub.id);
                    }
                    DeliveryOutcome::Gone => {
                        tracing::info!("push 410 Gone: removing sub {}", sub.id);
                        let _ = storage.delete_push_subscription_by_endpoint(&sub.endpoint);
                    }
                    DeliveryOutcome::TransientError => {
                        match storage.increment_push_failure_count(&sub.id) {
                            Ok(count) if count >= MAX_FAILURES => {
                                tracing::info!(
                                    "push failure_count={count}: removing dead sub {}",
                                    sub.id
                                );
                                let _ = storage.delete_push_subscription_by_endpoint(&sub.endpoint);
                            }
                            Ok(count) => {
                                tracing::warn!(
                                    "push delivery failed ({count}/{}): sub={}",
                                    MAX_FAILURES,
                                    sub.id
                                );
                            }
                            Err(e) => tracing::warn!("push failure_count update: {e}"),
                        }
                    }
                }
            }

            delivered_ids.push(notif.id.clone());
        }

        if !delivered_ids.is_empty() {
            let storage = state.storage.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(e) = storage.mark_notifications_push_delivered(&delivered_ids) {
                tracing::warn!("mark_push_delivered for {user_id}: {e}");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delivery outcome
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum DeliveryOutcome {
    Success,
    /// HTTP 410 Gone — subscription endpoint has been unregistered.
    Gone,
    /// Any other failure — increment failure_count; prune if >= MAX_FAILURES.
    TransientError,
}

/// Classify an HTTP status code returned by a push service.
pub fn outcome_from_status(status: u16) -> DeliveryOutcome {
    match status {
        200..=202 => DeliveryOutcome::Success,
        410 => DeliveryOutcome::Gone,
        _ => DeliveryOutcome::TransientError,
    }
}

// ---------------------------------------------------------------------------
// Push payload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
}

pub fn build_push_payload(notif: &NotificationWithActor) -> PushPayload {
    let actor_name = notif
        .actor
        .as_ref()
        .map(|a| a.display_name.as_str())
        .unwrap_or("CO");

    match notif.event_type.as_str() {
        "chat.dm" => {
            let room = notif.context.room_id.as_deref().unwrap_or("");
            let msg_id = notif.context.object_id.as_deref().unwrap_or("");
            PushPayload {
                title: actor_name.to_string(),
                body: "te enviou uma mensagem".to_string(),
                url: format!("/?dm={}#msg-{}", room, msg_id),
                tag: format!("dm-{}", room),
            }
        }
        "chat.mention" => {
            let universe = notif.context.universe_key.as_deref().unwrap_or("");
            let room = notif.context.room_id.as_deref().unwrap_or("");
            let msg_id = notif.context.object_id.as_deref().unwrap_or("");
            PushPayload {
                title: actor_name.to_string(),
                body: "mencionou você".to_string(),
                url: format!("/{}#chat-{}-{}", universe, room, msg_id),
                tag: format!("chat-{}", room),
            }
        }
        "chat.message" => {
            let universe = notif.context.universe_key.as_deref().unwrap_or("");
            let room = notif.context.room_id.as_deref().unwrap_or("");
            let msg_id = notif.context.object_id.as_deref().unwrap_or("");
            PushPayload {
                title: universe.to_string(),
                body: format!("nova mensagem de {actor_name}"),
                url: format!("/{}#chat-{}-{}", universe, room, msg_id),
                tag: format!("chat-{}", room),
            }
        }
        "universe.invitation" => {
            let universe = notif
                .summary
                .params
                .get("universe")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let inv_id = notif.context.object_id.as_deref().unwrap_or("");
            PushPayload {
                title: actor_name.to_string(),
                body: format!("te convidou para {universe}"),
                url: format!("/invitations/{}", inv_id),
                tag: format!("invitation-{}", inv_id),
            }
        }
        _ => PushPayload {
            title: "CO".to_string(),
            body: "Nova notificação".to_string(),
            url: "/".to_string(),
            tag: "co-notification".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Push preference filtering
// ---------------------------------------------------------------------------

pub fn filter_by_push_prefs<'a>(
    batch: &'a [NotificationWithActor],
    prefs: &NotificationPreferences,
) -> Vec<&'a NotificationWithActor> {
    batch
        .iter()
        .filter(|n| match n.event_type.as_str() {
            "chat.message" => prefs.push_chat_message,
            "chat.dm" => prefs.push_chat_dm,
            "chat.mention" => prefs.push_mention,
            "universe.invitation" => prefs.push_invitation,
            _ => true,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP delivery
// ---------------------------------------------------------------------------

async fn deliver_push(
    sub: &PushSubscription,
    payload: &PushPayload,
    vapid_private_key: &str,
    vapid_subject: &str,
) -> DeliveryOutcome {
    let content = match serde_json::to_vec(payload) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("push payload serialize: {e}");
            return DeliveryOutcome::TransientError;
        }
    };

    let encrypted = match ece_encrypt(&sub.p256dh, &sub.auth, &content) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("push ECE encrypt sub={}: {e}", sub.id);
            return DeliveryOutcome::TransientError;
        }
    };

    let origin = push_service_origin(&sub.endpoint);
    let jwt = match vapid_jwt(vapid_private_key, origin, vapid_subject) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("VAPID JWT: {e}");
            return DeliveryOutcome::TransientError;
        }
    };
    let pub_key = match vapid_public_key_from_private(vapid_private_key) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("VAPID public key: {e}");
            return DeliveryOutcome::TransientError;
        }
    };

    let authorization = format!("vapid t={jwt},k={pub_key}");

    let client = reqwest::Client::new();
    let result = client
        .post(&sub.endpoint)
        .header("Authorization", authorization)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", "60")
        .body(encrypted)
        .send()
        .await;

    match result {
        Ok(r) => outcome_from_status(r.status().as_u16()),
        Err(e) => {
            tracing::warn!("push HTTP error sub={}: {e}", sub.id);
            DeliveryOutcome::TransientError
        }
    }
}

fn push_service_origin(endpoint: &str) -> &str {
    let after_scheme = endpoint.find("://").map(|i| i + 3).unwrap_or(0);
    let path_start = endpoint[after_scheme..]
        .find('/')
        .map(|i| after_scheme + i)
        .unwrap_or(endpoint.len());
    &endpoint[..path_start]
}

// ---------------------------------------------------------------------------
// VAPID JWT signing (ES256)
// ---------------------------------------------------------------------------

fn vapid_jwt(private_key_b64: &str, audience: &str, subject: &str) -> anyhow::Result<String> {
    use base64::Engine as _;
    use p256::ecdsa::signature::Signer;

    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let key_bytes = engine.decode(private_key_b64)?;
    let signing_key = p256::ecdsa::SigningKey::from_slice(&key_bytes)?;

    let header = engine.encode(r#"{"typ":"JWT","alg":"ES256"}"#);
    let exp = Utc::now().timestamp() + 43200; // 12 hours
    let payload_json = serde_json::json!({
        "sub": subject,
        "aud": audience,
        "exp": exp,
    });
    let payload = engine.encode(serde_json::to_string(&payload_json)?);
    let signing_input = format!("{header}.{payload}");

    let sig: p256::ecdsa::Signature = signing_key.sign(signing_input.as_bytes());
    let sig_b64 = engine.encode(sig.to_bytes());

    Ok(format!("{signing_input}.{sig_b64}"))
}

pub fn vapid_public_key_from_private(private_key_b64: &str) -> anyhow::Result<String> {
    use base64::Engine as _;

    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let key_bytes = engine.decode(private_key_b64)?;
    let signing_key = p256::ecdsa::SigningKey::from_slice(&key_bytes)?;
    let verifying_key = signing_key.verifying_key();
    let encoded = verifying_key.to_encoded_point(false); // uncompressed, 65 bytes
    Ok(engine.encode(encoded.as_bytes()))
}

// ---------------------------------------------------------------------------
// AES-128-GCM content encoding (RFC 8188 + RFC 8291)
// ---------------------------------------------------------------------------

fn ece_encrypt(p256dh_b64: &str, auth_b64: &str, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes128Gcm, Key, Nonce};
    use base64::Engine as _;
    use hkdf::Hkdf;
    use p256::PublicKey;
    use p256::ecdh::EphemeralSecret;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use rand::RngCore;
    use sha2::Sha256;

    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let p256dh_bytes = engine.decode(p256dh_b64)?;
    let auth_bytes = engine.decode(auth_b64)?;

    // Generate ephemeral sender key pair
    let mut rng = rand::thread_rng();
    let ephemeral_secret = EphemeralSecret::random(&mut rng);
    let ephemeral_pk = PublicKey::from(&ephemeral_secret);
    let ephemeral_pk_enc = ephemeral_pk.to_encoded_point(false); // 65 bytes uncompressed
    let ephemeral_pk_bytes = ephemeral_pk_enc.as_bytes();

    // Decode receiver's public key (p256dh)
    let receiver_pk = PublicKey::from_sec1_bytes(&p256dh_bytes)
        .map_err(|e| anyhow::anyhow!("invalid p256dh: {e}"))?;

    // ECDH shared secret
    let shared = ephemeral_secret.diffie_hellman(&receiver_pk);
    let ecdh_bytes = shared.raw_secret_bytes();

    // Random salt (16 bytes)
    let mut salt = [0u8; 16];
    rng.fill_bytes(&mut salt);

    // RFC 8291 key derivation:
    // PRK_key = HKDF-Extract(salt=auth_secret, IKM=ecdh_secret)
    let prk_key = Hkdf::<Sha256>::new(Some(&auth_bytes), ecdh_bytes.as_slice());

    // key_info = "WebPush: info\0" || ua_public(65) || as_public(65)
    let mut key_info: Vec<u8> = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&p256dh_bytes); // receiver (ua) public key
    key_info.extend_from_slice(ephemeral_pk_bytes); // sender (as) public key

    let mut ikm = [0u8; 32];
    prk_key
        .expand(&key_info, &mut ikm)
        .map_err(|_| anyhow::anyhow!("HKDF expand key_info failed"))?;

    // PRK = HKDF-Extract(salt=random_salt, IKM=ikm)
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);

    // CEK = first 16 bytes from HKDF-Expand(PRK, "Content-Encoding: aes128gcm\0")
    let mut cek = [0u8; 16];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| anyhow::anyhow!("HKDF expand CEK failed"))?;

    // Nonce = 12 bytes from HKDF-Expand(PRK, "Content-Encoding: nonce\0")
    let mut nonce_bytes = [0u8; 12];
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce_bytes)
        .map_err(|_| anyhow::anyhow!("HKDF expand nonce failed"))?;

    // Pad: plaintext || 0x02 (RFC 8188 record delimiter, no content padding for v1)
    let mut padded = plaintext.to_vec();
    padded.push(2);

    // AES-128-GCM encrypt (appends 16-byte auth tag automatically)
    let key = Key::<Aes128Gcm>::from_slice(&cek);
    let cipher = Aes128Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, padded.as_slice())
        .map_err(|e| anyhow::anyhow!("AES-GCM encrypt: {e}"))?;

    // ECE header: salt(16) | rs(4 big-endian) | idlen(1) | server_public_key(65)
    let rs: u32 = 4096;
    let mut output = Vec::with_capacity(16 + 4 + 1 + ephemeral_pk_bytes.len() + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&rs.to_be_bytes());
    output.push(ephemeral_pk_bytes.len() as u8); // 65
    output.extend_from_slice(ephemeral_pk_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use rusqlite::params;
    use tempfile::tempdir;

    use crate::storage::Storage;
    use crate::storage::notifications::{
        NotificationActor, NotificationContext, NotificationSummary,
    };

    fn make_prefs_with_quiet(start: Option<&str>, end: Option<&str>) -> NotificationPreferences {
        NotificationPreferences {
            user_id: "usr_push_test".into(),
            email_chat_message: false,
            email_chat_dm: true,
            email_invitation: true,
            email_mention: true,
            email_digest_freq: "daily".into(),
            push_chat_message: true,
            push_chat_dm: true,
            push_invitation: true,
            push_mention: true,
            in_app_chat_message: true,
            in_app_chat_dm: true,
            in_app_invitation: true,
            in_app_mention: true,
            quiet_hours_start: start.map(|s| s.to_string()),
            quiet_hours_end: end.map(|s| s.to_string()),
            quiet_hours_tz: "UTC".into(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn make_notif(event_type: &str) -> NotificationWithActor {
        NotificationWithActor {
            id: format!("notif_{}", nanoid::nanoid!(6)),
            event_type: event_type.into(),
            actor: Some(NotificationActor {
                user_id: "actor1".into(),
                display_name: "Maria".into(),
                usuario: Some("maria".into()),
            }),
            summary: NotificationSummary {
                key: format!("notif.{event_type}"),
                params: serde_json::json!({"universe": "myuniverse"}),
            },
            context: NotificationContext {
                universe_key: Some("myuniverse".into()),
                room_id: Some("room_abc".into()),
                object_id: Some("msg_xyz".into()),
            },
            created_at: Utc::now().to_rfc3339(),
            read_at: None,
        }
    }

    fn insert_user_and_sub(dir: &std::path::Path) -> (String, String) {
        let storage = Storage::new(dir.to_str().unwrap());
        storage.ensure_push_subscriptions_table();
        let user_id = format!("usr_pw_{}", nanoid::nanoid!(8));
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO users (id, display_name, tier, created_at) \
                 VALUES (?1, 'Test', 'player', ?2)",
                params![user_id, now],
            )
            .unwrap();
        let sub_id = storage
            .upsert_push_subscription(
                &user_id,
                "https://push.example.com/test",
                "BLxCdummyp256dh",
                "dummyauth",
                None,
            )
            .unwrap();
        (user_id, sub_id)
    }

    // =========================================================================
    // test_payload_size_under_4kb
    // =========================================================================

    #[test]
    fn test_payload_size_under_4kb() {
        let notif = make_notif("chat.dm");
        let payload = build_push_payload(&notif);
        let json = serde_json::to_vec(&payload).unwrap();
        assert!(
            json.len() < 4096,
            "push payload must be under 4KB (got {} bytes)",
            json.len()
        );
    }

    // =========================================================================
    // test_quiet_hours_skip_logic
    // =========================================================================

    #[test]
    fn test_quiet_hours_skip_logic() {
        // 22:00–07:00 overnight quiet window
        let prefs = make_prefs_with_quiet(Some("22:00"), Some("07:00"));

        // 23:30 UTC — inside quiet window
        let inside = Utc.with_ymd_and_hms(2026, 5, 11, 23, 30, 0).unwrap();
        assert!(
            is_in_quiet_hours(&prefs, inside),
            "23:30 should be in quiet hours"
        );

        // 12:00 UTC — outside quiet window
        let outside = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        assert!(
            !is_in_quiet_hours(&prefs, outside),
            "12:00 should NOT be in quiet hours"
        );

        // Push filter should skip during quiet hours
        let notif = make_notif("chat.dm");
        let batch = vec![notif];
        let filtered = filter_by_push_prefs(&batch, &prefs);
        // filter_by_push_prefs only checks event type prefs, not quiet hours
        // (quiet hours are checked in tick() before calling filter_by_push_prefs)
        assert_eq!(filtered.len(), 1, "push_chat_dm is enabled in prefs");
    }

    // =========================================================================
    // test_410_gone_response_deletes_subscription
    // =========================================================================

    #[test]
    fn test_410_gone_response_deletes_subscription() {
        let dir = tempdir().unwrap();
        let (user_id, _sub_id) = insert_user_and_sub(dir.path());

        // Simulate a 410 response: delete the subscription
        let storage = Storage::new(dir.path().to_str().unwrap());
        let subs = storage.list_all_push_subscriptions_for_user(&user_id);
        assert_eq!(subs.len(), 1);

        let outcome = outcome_from_status(410);
        assert_eq!(outcome, DeliveryOutcome::Gone);

        // Apply the outcome as the worker would
        storage
            .delete_push_subscription_by_endpoint(&subs[0].endpoint)
            .unwrap();

        let subs_after = storage.list_all_push_subscriptions_for_user(&user_id);
        assert!(
            subs_after.is_empty(),
            "subscription must be deleted after 410 Gone"
        );
    }

    // =========================================================================
    // test_5xx_response_increments_failure_count
    // =========================================================================

    #[test]
    fn test_5xx_response_increments_failure_count() {
        let dir = tempdir().unwrap();
        let (_user_id, sub_id) = insert_user_and_sub(dir.path());

        let outcome = outcome_from_status(503);
        assert_eq!(outcome, DeliveryOutcome::TransientError);

        let storage = Storage::new(dir.path().to_str().unwrap());
        let new_count = storage.increment_push_failure_count(&sub_id).unwrap();
        assert_eq!(new_count, 1, "failure_count should be incremented to 1");
    }

    // =========================================================================
    // test_failure_count_5_marks_dead
    // =========================================================================

    #[test]
    fn test_failure_count_5_marks_dead() {
        let dir = tempdir().unwrap();
        let (user_id, sub_id) = insert_user_and_sub(dir.path());

        let storage = Storage::new(dir.path().to_str().unwrap());

        // Increment failure_count 5 times
        for _ in 0..5 {
            storage.increment_push_failure_count(&sub_id).unwrap();
        }

        // On the 5th failure the worker deletes the subscription
        let subs = storage.list_all_push_subscriptions_for_user(&user_id);
        let sub = &subs[0];
        assert_eq!(
            sub.failure_count, 5,
            "failure_count should be 5 before pruning"
        );

        // Worker checks failure_count >= MAX_FAILURES and deletes
        if sub.failure_count >= MAX_FAILURES {
            storage
                .delete_push_subscription_by_endpoint(&sub.endpoint)
                .unwrap();
        }

        let subs_after = storage.list_all_push_subscriptions_for_user(&user_id);
        assert!(
            subs_after.is_empty(),
            "dead subscription (failure_count=5) must be pruned"
        );
    }
}
