//! CO-479 — inbound webhook for the official WhatsApp Cloud API.
//!
//! Meta calls this endpoint directly (no auth gate — security is the HMAC
//! signature, mirroring `billing::routes::webhook_router`):
//! - `GET  /api/v1/whatsapp/webhook` — verification handshake; echoes
//!   `hub.challenge` when `hub.verify_token` matches `WHATSAPP_VERIFY_TOKEN`.
//! - `POST /api/v1/whatsapp/webhook` — message events, signed with
//!   `X-Hub-Signature-256: sha256=<hmac>` keyed by `WHATSAPP_APP_SECRET`.
//!
//! Env vars:
//! - `WHATSAPP_VERIFY_TOKEN` — shared secret echoed during GET verification.
//! - `WHATSAPP_APP_SECRET`   — Meta app secret; verifies POST signatures.
//!
//! CO-480: inbound messages are forwarded to the bot brain (`CO_BOT_BRAIN_URL`,
//! default the `whatsapp-bot` bridge) and the reply is sent via
//! [`crate::notification_providers::CloudApiProvider`] — **asynchronously**, so
//! Meta still gets its fast 200 ack while the model call runs in the background.
//!
//! CO-489: the inbound `phone_number_id` (which WhatsApp business number was
//! messaged) is forwarded to the brain so a single bot deploy can route to the
//! right tenant universe (the brain maps `phone_number_id → universe`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// CO-489 — tier-2 scalability hardening of the inbound reply path
// ---------------------------------------------------------------------------

/// Bound on every outbound hop made while answering an inbound message (the
/// brain call AND the Cloud API send). A normal brain reply is a few seconds;
/// 30s leaves head-room for a slow model while still capping a hung hop so a
/// stuck task can never leak forever (the old `Client::new()` had NO timeout).
const REPLY_HTTP_TIMEOUT_SECS: u64 = 30;

/// Default process-wide cap on concurrent inbound replies (brain hops). The
/// brain fronts a single local model, so unbounded `spawn`-per-message would
/// pile work up without limit; this bounds it. Override with
/// `CO_WHATSAPP_REPLY_CONCURRENCY`.
const DEFAULT_REPLY_CONCURRENCY: usize = 4;

/// How long a processed `wamid` stays in the in-memory seen-set. Meta delivers
/// at-least-once and retries for a bounded window; 5 minutes covers the retry
/// window so a redelivery doesn't re-run the brain and double-send.
const WAMID_TTL: Duration = Duration::from_secs(300);

/// ONE shared, timed [`reqwest::Client`] for the whole inbound/bot reply path,
/// reused across messages so connections are pooled and every hop is bounded by
/// [`REPLY_HTTP_TIMEOUT_SECS`]. Built once (mirrors `webhook_worker`'s timed
/// client); reused for the brain hop, the Cloud API send, and `bot_proxy`.
pub fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REPLY_HTTP_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Parse the reply-concurrency cap from the env value, falling back to
/// [`DEFAULT_REPLY_CONCURRENCY`] when unset, empty, non-numeric, or zero. Pure →
/// unit-testable.
pub fn parse_reply_concurrency(raw: Option<String>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_REPLY_CONCURRENCY)
}

/// Process-wide semaphore bounding concurrent inbound replies. Sized once from
/// `CO_WHATSAPP_REPLY_CONCURRENCY` (default [`DEFAULT_REPLY_CONCURRENCY`]).
fn reply_semaphore() -> Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| {
        let n = parse_reply_concurrency(
            crate::infra::secrets::global().get("CO_WHATSAPP_REPLY_CONCURRENCY"),
        );
        Arc::new(Semaphore::new(n))
    })
    .clone()
}

/// Pure idempotency decision (CO-489). Prunes entries older than `ttl`, then
/// returns `true` if `wamid` is first-seen — recording it at `now` — or `false`
/// if it was already seen within `ttl`. Takes the map by `&mut` so it is
/// testable without any global state or a running server.
pub fn dedup_decision(
    seen: &mut HashMap<String, Instant>,
    wamid: &str,
    now: Instant,
    ttl: Duration,
) -> bool {
    seen.retain(|_, t| now.duration_since(*t) < ttl);
    if seen.contains_key(wamid) {
        return false;
    }
    seen.insert(wamid.to_string(), now);
    true
}

/// Record `wamid` in the process-wide short-TTL seen-set and report whether it
/// should be dispatched to the brain (first-seen → `true`; a Meta redelivery
/// within [`WAMID_TTL`] → `false`). Thin lock around [`dedup_decision`].
fn mark_and_check_wamid(wamid: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = seen.lock().unwrap_or_else(|e| e.into_inner());
    dedup_decision(&mut guard, wamid, Instant::now(), WAMID_TTL)
}

pub fn router() -> Router<AppState> {
    Router::new().route("/whatsapp/webhook", get(verify_handler).post(event_handler))
}

/// CO-490: authenticated companion-app endpoints. Mounted separately from
/// [`router`] (the Meta webhook) because these routes MUST sit behind the
/// `require_auth` gate, whereas the webhook is Meta-HMAC-verified and ungated.
///
/// Two-step proof-of-possession flow (CO-490 #3):
/// - `POST /whatsapp/link/start`  — send a stateless OTP to the number.
/// - `POST /whatsapp/link/verify` — verify the OTP, then link + mint the token.
pub fn link_router() -> Router<AppState> {
    Router::new()
        .route("/whatsapp/link/start", post(link_start_handler))
        .route("/whatsapp/link/verify", post(link_verify_handler))
}

// ---------------------------------------------------------------------------
// GET — Meta verification handshake
// ---------------------------------------------------------------------------

async fn verify_handler(Query(params): Query<HashMap<String, String>>) -> Response {
    let expected = crate::infra::secrets::global()
        .get("WHATSAPP_VERIFY_TOKEN")
        .unwrap_or_default();
    let mode = params.get("hub.mode").map(String::as_str).unwrap_or("");
    let token = params
        .get("hub.verify_token")
        .map(String::as_str)
        .unwrap_or("");
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    if mode == "subscribe" && !expected.is_empty() && token == expected {
        (StatusCode::OK, challenge).into_response()
    } else {
        tracing::warn!("WhatsApp Cloud webhook: verification failed");
        (StatusCode::FORBIDDEN, "verification failed").into_response()
    }
}

// ---------------------------------------------------------------------------
// POST — signed message events
// ---------------------------------------------------------------------------

async fn event_handler(headers: HeaderMap, body: Bytes) -> Response {
    let app_secret = crate::infra::secrets::global()
        .get("WHATSAPP_APP_SECRET")
        .unwrap_or_default();

    // Meta requires a fast 200. If the secret is unset (not yet configured),
    // accept-and-ignore so the handshake/subscription works during setup.
    if app_secret.is_empty() {
        tracing::warn!("WHATSAPP_APP_SECRET unset — skipping signature verification");
        return StatusCode::OK.into_response();
    }

    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(&app_secret, &body, sig) {
        tracing::warn!("WhatsApp Cloud webhook: invalid signature");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // CO-480: reply ASYNCHRONOUSLY. Meta requires a fast 200 (it retries on
    // delay), and the brain/model call takes seconds — so never block the ack on
    // it. Spawning returns immediately, so the 200 below is still fast.
    for m in parse_inbound(&body) {
        // CO-489 idempotency: Meta is at-least-once and retries; a redelivery of
        // the same `wamid` must NOT re-run the brain and double-send. Skip any
        // wamid already processed within WAMID_TTL.
        if !m.wamid.is_empty() && !mark_and_check_wamid(&m.wamid) {
            tracing::info!(wamid = %m.wamid, "WhatsApp Cloud inbound: duplicate wamid, skipping");
            continue;
        }
        tracing::info!(from = %m.from, "WhatsApp Cloud inbound: {}", m.text);

        // CO-489 bounded concurrency: acquire a process-wide permit before the
        // brain hop so bursts/redeliveries fronting the single local model can't
        // pile up unbounded tasks. The task still runs (Meta already got its 200);
        // it just waits its turn, capping in-flight brain hops.
        let sem = reply_semaphore();
        tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            if let Err(e) = reply_to(&m).await {
                tracing::warn!(from = %m.from, "WhatsApp Cloud reply failed: {e}");
            }
        });
    }

    StatusCode::OK.into_response()
}

// ---------------------------------------------------------------------------
// CO-480 — inbound → bot brain → reply (via the official Cloud API)
// ---------------------------------------------------------------------------

/// The bot brain endpoint (the `whatsapp-bot` bridge's `/api/chat`, or any
/// `{text}→{reply}` JSON service). Same brain the companion app uses.
fn brain_url() -> String {
    crate::infra::secrets::global().get_or("CO_BOT_BRAIN_URL", "http://localhost:8765/api/chat")
}

/// Inbound message → brain reply → send via [`CloudApiProvider`]. No-op (logged)
/// when the Cloud API isn't configured; falls back to a fixed ack if the brain
/// is unreachable so the user always gets a response.
async fn reply_to(msg: &InboundMsg) -> Result<(), String> {
    // CO-497: cascade (Cloud → Evolution) so self-hosted deploys (Evolution only)
    // reply identically to managed Cloud deploys.
    let provider = crate::notification_providers::whatsapp_provider_cascade()
        .ok_or("no WhatsApp provider configured (Cloud API or Evolution)")?;
    // CO-489: ONE shared, timed client for both the brain hop and the Cloud send.
    let client = shared_client();
    let reply = fetch_brain_reply(client, &brain_url(), &msg.text, &msg.phone_number_id)
        .await
        .unwrap_or_else(|| "Recebi sua mensagem — já te respondo! 🙂".to_string());
    provider.send(client, &msg.from, &reply).await
}

/// POST `{text, phone_number_id}` to the brain, return its `reply`. The
/// `phone_number_id` selects the tenant universe brain-side (CO-489). `None` on
/// any transport/parse failure (caller substitutes a fallback).
async fn fetch_brain_reply(
    client: &reqwest::Client,
    url: &str,
    text: &str,
    phone_number_id: &str,
) -> Option<String> {
    let mut builder = client.post(url).json(&serde_json::json!({
        "text": text,
        "sender": "whatsapp",
        "phone_number_id": phone_number_id,
    }));
    // CO-490 (#5): authenticate co-web → bot over loopback when configured.
    if let Some((name, value)) = crate::bot_proxy_routes::brain_auth_header(
        crate::infra::secrets::global().get("CO_BOT_SHARED_SECRET"),
    ) {
        builder = builder.header(name, value);
    }
    let resp = builder.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.bytes().await.ok()?;
    parse_brain_reply(&body)
}

/// Extract the non-empty `reply` string from the brain's JSON response.
pub fn parse_brain_reply(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("reply")
        .and_then(|r| r.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a server)
// ---------------------------------------------------------------------------

/// Verify Meta's `X-Hub-Signature-256: sha256=<hex>` against the raw body.
pub fn verify_signature(app_secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hexsig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let expected = crate::billing::hmac_sha256_hex(app_secret, body);
    crate::billing::constant_time_eq(expected.as_bytes(), hexsig.as_bytes())
}

/// A single inbound text message lifted from a Cloud API webhook payload.
#[derive(Debug, PartialEq)]
pub struct InboundMsg {
    pub from: String,
    pub text: String,
    /// Which business number was messaged — selects the tenant brain-side (CO-489).
    pub phone_number_id: String,
    /// Meta's message id (`wamid.*`). Used to dedup at-least-once redeliveries so
    /// the brain isn't re-run and the user double-replied (CO-489). May be empty
    /// for a payload that omits it (then dedup is skipped for that message).
    pub wamid: String,
}

/// Extract text messages from a Cloud API webhook body. Status-only callbacks
/// (delivery receipts, read markers) yield an empty vec.
pub fn parse_inbound(body: &[u8]) -> Vec<InboundMsg> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return out;
    };
    let entries = v.get("entry").and_then(|e| e.as_array());
    for entry in entries.into_iter().flatten() {
        let changes = entry.get("changes").and_then(|c| c.as_array());
        for change in changes.into_iter().flatten() {
            let value = change.get("value");
            // CO-489: the business number id lives once per change, in metadata.
            let phone_number_id = value
                .and_then(|val| val.get("metadata"))
                .and_then(|md| md.get("phone_number_id"))
                .and_then(|p| p.as_str())
                .unwrap_or_default()
                .to_string();
            let msgs = value
                .and_then(|val| val.get("messages"))
                .and_then(|m| m.as_array());
            for m in msgs.into_iter().flatten() {
                let from = m
                    .get("from")
                    .and_then(|f| f.as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = m
                    .get("text")
                    .and_then(|t| t.get("body"))
                    .and_then(|b| b.as_str())
                    .unwrap_or_default()
                    .to_string();
                // CO-489: carry the message id for at-least-once dedup.
                let wamid = m
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !from.is_empty() && !text.is_empty() {
                    out.push(InboundMsg {
                        from,
                        text,
                        phone_number_id: phone_number_id.clone(),
                        wamid,
                    });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CO-490 — companion-app account link (authenticated) → bot identity ingest
// ---------------------------------------------------------------------------

/// Default identity-ingest URL when `CO_BOT_IDENTITY_URL` is unset. Loopback
/// only: the bot runs as a sidecar, never reachable off-box.
const DEFAULT_IDENTITY_URL: &str = "http://127.0.0.1:8765/identity";

/// Name stamped on the API token minted for the bot. Lets the user recognise
/// (and revoke) the bot's token in their token list.
const LINK_TOKEN_NAME: &str = "whatsapp-bot";

/// CO-490 (#2): the MINIMUM capability set the bot token carries. The bot only
/// reads/writes the user's content entries and reads which universes the user
/// has — nothing more. Explicitly excludes `universes:write` (the bot never
/// creates/edits universes), every admin-surface capability (`gestao:read`,
/// `funnel:read`, `chat:*`, `deployments:read`), and `agent:dispatch`. There is
/// no token-management capability in the vocabulary, so a leaked bot token can
/// never mint or revoke tokens. A leaked secret then grants only this set, never
/// owner-tier authority.
fn bot_token_scopes() -> std::collections::BTreeSet<String> {
    crate::auth::capabilities::resolve_scopes(&[
        "entries:read".to_string(),
        "entries:write".to_string(),
        "universes:read".to_string(),
        // CO-499: read-only telemetry so the bot can produce the lead-temperature
        // digest (output B) for the user's own universes. Still least-privilege —
        // read-only, no write/admin/token-management.
        "telemetry:read".to_string(),
    ])
    .expect("static bot scope set is valid")
}

/// OTP rotation period in seconds (CO-490 #3): a code is valid for the current
/// 300s bucket and the immediately preceding one (so a code minted near a bucket
/// boundary still verifies for up to ~5 more minutes).
const OTP_PERIOD_SECS: u64 = 300;

#[derive(Debug, Deserialize)]
pub struct LinkStartRequest {
    pub whatsapp: String,
}

#[derive(Debug, Deserialize)]
pub struct LinkVerifyRequest {
    pub whatsapp: String,
    pub code: String,
}

/// Current unix time in seconds (0 if the clock is before the epoch — never).
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The OTP HMAC secret: `CO_WHATSAPP_OTP_SECRET` when set/non-empty, else the
/// `JWT_SECRET` (already required in prod). Never returns an empty string in a
/// configured deployment.
fn otp_secret() -> String {
    crate::infra::secrets::global()
        .get("CO_WHATSAPP_OTP_SECRET")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::auth::jwt_secret)
}

/// CO-490 (#3): the 300s time bucket for `unix_secs`. The OTP is a pure function
/// of `(user_id, digits, bucket)` so no server-side state is stored.
pub fn otp_bucket(unix_secs: u64) -> u64 {
    unix_secs / OTP_PERIOD_SECS
}

/// Number of decimal digits in the OTP (CO-490 R3: widened 6→8 to harden against
/// brute force — 10^8 keyspace within the 5-minute window instead of 10^6).
const OTP_DIGITS: usize = 8;

/// Extract the first [`OTP_DIGITS`] decimal digits (`0`–`9`) of a hex string. A
/// 64-char SHA-256 hex digest contains ~40 decimal digits on average, so this
/// never short-pads in practice; the `0` pad is a belt-and-suspenders fallback
/// for a pathological all-letters digest. Pure → unit-testable.
fn first_n_decimal_digits(hex: &str) -> String {
    let mut digits: String = hex
        .chars()
        .filter(char::is_ascii_digit)
        .take(OTP_DIGITS)
        .collect();
    while digits.len() < OTP_DIGITS {
        digits.push('0');
    }
    digits
}

/// Compute the stateless 8-digit OTP = first [`OTP_DIGITS`] decimal digits of
/// `HMAC-SHA256(secret, "<user_id>|<digits>|<bucket>")`. Pure → unit-testable.
pub fn compute_otp(secret: &str, user_id: &str, digits: &str, bucket: u64) -> String {
    let msg = format!("{user_id}|{digits}|{bucket}");
    let hex = crate::billing::hmac_sha256_hex(secret, msg.as_bytes());
    first_n_decimal_digits(&hex)
}

/// CO-490 (#3): does `code` match the OTP for the current OR previous 300s
/// bucket? Constant-time compare (no early-exit timing leak). Pure → testable.
pub fn otp_matches(secret: &str, user_id: &str, digits: &str, code: &str, now_secs: u64) -> bool {
    let bucket = otp_bucket(now_secs);
    let candidates = [bucket, bucket.saturating_sub(1)];
    let mut ok = false;
    for b in candidates {
        let expected = compute_otp(secret, user_id, digits, b);
        // Fold every candidate into one constant-time pass (no early return).
        ok |= crate::billing::constant_time_eq(expected.as_bytes(), code.as_bytes());
    }
    ok
}

/// POST `/api/v1/whatsapp/link/start` — send a proof-of-possession OTP to the
/// number. Authenticated; the OTP binds the *caller's own* `user_id`, so a code
/// minted for one user can never verify another. The code is sent via the
/// official WhatsApp Cloud API (so the user must control the number) and is
/// **never** returned in the HTTP response.
pub async fn link_start_handler(
    user_id: crate::auth::UserId,
    Json(req): Json<LinkStartRequest>,
) -> Result<impl IntoResponse, AppError> {
    let digits = validate_whatsapp(&req.whatsapp)?;

    // Proof-of-possession requires an outbound channel; without it we cannot
    // prove the caller controls the number, so refuse rather than link blindly.
    // CO-497: cascade (Cloud → Evolution) so self-hosters on Evolution can link.
    let provider = crate::notification_providers::whatsapp_provider_cascade().ok_or_else(|| {
        AppError::ServiceUnavailable(
            "no WhatsApp provider configured (Cloud API or Evolution) — cannot send code".into(),
        )
    })?;

    let code = compute_otp(&otp_secret(), &user_id.0, &digits, otp_bucket(now_unix()));
    let client = shared_client();
    let body = format!("Seu código de verificação CO: {code}");
    provider.send(client, &digits, &body).await.map_err(|e| {
        AppError::ServiceUnavailable(format!("failed to send verification code: {e}"))
    })?;

    // CO-491: surface the consent version the user is being asked to agree to, so
    // the agreed version is known/auditable end-to-end (the link prompt shows the
    // versioned `agreement` text; the user agrees by completing the OTP verify).
    let consent_version = super::whatsapp_consent::consent_version();
    tracing::info!(
        user_id = %user_id.0,
        consent_version = %consent_version,
        "whatsapp link/start: presenting consent version"
    );

    // Never echo the code.
    Ok(Json(
        serde_json::json!({ "sent": true, "consent_version": consent_version }),
    ))
}

/// POST `/api/v1/whatsapp/link/verify` — verify the OTP, then link + mint.
///
/// Security: every write is scoped to `user_id.0` — the id resolved by the
/// `require_auth`/`UserId` extractor from the caller's own session/token. On a
/// valid code we (a) reject if the number is already linked to a *different*
/// user (`409`), (b) set the caller's own `users.whatsapp`, (c) revoke the
/// caller's prior `whatsapp-bot` tokens, (d) mint a least-privilege scoped token
/// (#2), and (e) best-effort push `{whatsapp, token, user_id}` to the bot over
/// loopback. The minted token is **never** echoed in the HTTP response.
pub async fn link_verify_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
    Json(req): Json<LinkVerifyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let digits = validate_whatsapp(&req.whatsapp)?;
    let code = req.code.trim();

    // Proof of possession: the caller must echo the code we sent to the number.
    if !otp_matches(&otp_secret(), &user_id.0, &digits, code, now_unix()) {
        return Err(AppError::Unauthorized("invalid or expired code".into()));
    }

    // (a) uniqueness, (b) set number, (c) revoke prior bot tokens, (d) mint the
    // scoped token — all under one lock; the lock drops before any network I/O.
    let token = {
        let storage = crate::server::state::lock_storage(&state);

        // (a) one-number-per-account: reject if another user already owns it.
        if let Some(other) = storage.get_user_id_by_whatsapp(&digits)
            && other != user_id.0
        {
            return Err(AppError::Conflict(
                "this WhatsApp number is already linked to another account".into(),
            ));
        }

        // (b) set the caller's own number (digit-normalized → stable for the
        //     uniqueness compare above).
        storage
            .set_user_whatsapp(&user_id.0, &digits)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // (c) #6: revoke prior `whatsapp-bot` tokens before minting a fresh one.
        storage
            .delete_api_tokens_by_name(&user_id.0, LINK_TOKEN_NAME)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // (d) #2: mint a LEAST-PRIVILEGE scoped token (not owner-tier).
        let tok = storage
            .create_api_token_with_scopes(&user_id.0, LINK_TOKEN_NAME, &bot_token_scopes())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        tok.token.unwrap_or_default()
    };

    // (e) best-effort push {whatsapp, token, user_id} to the bot identity ingest.
    // Skipped silently when the shared secret is unset; a transport/HTTP error is
    // logged but NEVER fails the link (the user is already linked CO-side).
    best_effort_identity_push(&user_id.0, &digits, &token).await;

    // CO-491: log the consent version the user agreed to (LGPD demonstrabilidade).
    // Durable per-user persistence of the agreed version needs a schema column =
    // a coordinated migration (NOT added here; tracked in CHANGELOG-PENDING) — for
    // now the agreed version is auditable via this log + the response field.
    let consent_version = super::whatsapp_consent::consent_version();
    tracing::info!(
        user_id = %user_id.0,
        consent_version = %consent_version,
        "whatsapp link/verify: user agreed to consent version"
    );

    // Token is intentionally NOT returned — it reaches the bot over loopback only.
    Ok(Json(
        serde_json::json!({ "linked": true, "consent_version": consent_version }),
    ))
}

/// Normalize + validate the inbound number to its decimal digits (strips `+`,
/// spaces, dashes). Rejects an input with no digits. Pure → unit-testable. The
/// digit-only form is what the OTP, uniqueness check, and `users.whatsapp` all
/// agree on, so an inbound `+55 41 99999-9999` and a stored `5541999999999`
/// compare equal.
pub fn validate_whatsapp(raw: &str) -> Result<String, AppError> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return Err(AppError::BadRequest("whatsapp is required".into()));
    }
    Ok(digits)
}

/// A fully-built identity-ingest request: where to POST, the shared secret for
/// the `X-Bot-Secret` header, and the JSON body. Constructed by
/// [`build_identity_push`] so it can be asserted in tests without a live socket.
#[derive(Debug, PartialEq)]
pub struct IdentityPush {
    pub url: String,
    pub secret: String,
    pub body: serde_json::Value,
}

/// Build the identity-ingest request from env-derived config. Returns `None`
/// (→ skip the push, link still succeeds) when the shared secret is unset/empty,
/// since the bot ingest rejects any push without a matching `X-Bot-Secret`. The
/// URL falls back to [`DEFAULT_IDENTITY_URL`] when unset/empty. The body carries
/// `{whatsapp, token, user_id}` so the bot can key identities by `user_id`
/// (CO-490 #1/#3e). Pure → testable.
pub fn build_identity_push(
    url: Option<String>,
    secret: Option<String>,
    user_id: &str,
    whatsapp: &str,
    token: &str,
) -> Option<IdentityPush> {
    let secret = secret.filter(|s| !s.is_empty())?;
    let url = url
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_IDENTITY_URL.to_string());
    Some(IdentityPush {
        url,
        secret,
        body: serde_json::json!({
            "whatsapp": whatsapp,
            "token": token,
            "user_id": user_id,
        }),
    })
}

/// Read identity-push config from env, build the request, and POST it — all
/// best-effort: any missing secret, client-build error, or transport/HTTP error
/// is silently skipped or logged, NEVER propagated (the link already succeeded
/// CO-side). Kept off the storage-lock path; safe to call after the lock drops.
async fn best_effort_identity_push(user_id: &str, whatsapp: &str, token: &str) {
    let Some(push) = build_identity_push(
        crate::infra::secrets::global().get("CO_BOT_IDENTITY_URL"),
        crate::infra::secrets::global().get("CO_BOT_SHARED_SECRET"),
        user_id,
        whatsapp,
        token,
    ) else {
        return;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    else {
        return;
    };
    if let Err(e) = push_identity(&client, &push).await {
        tracing::warn!("CO-490 bot identity push failed (link still succeeded): {e}");
    }
}

/// POST the identity payload to the bot, with the `X-Bot-Secret` header. Returns
/// `Err` on transport failure or any non-2xx status (caller logs and ignores).
async fn push_identity(client: &reqwest::Client, push: &IdentityPush) -> Result<(), String> {
    let resp = client
        .post(&push.url)
        .header("X-Bot-Secret", &push.secret)
        .json(&push.body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("identity ingest returned {}", resp.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> &'static [u8] {
        // Raw STRING (not byte string) so the UTF-8 accent is allowed.
        r#"{"object":"whatsapp_business_account","entry":[{"id":"WABA","changes":[
          {"value":{"messaging_product":"whatsapp",
            "metadata":{"display_phone_number":"5541999999999","phone_number_id":"1238594552665575"},
            "messages":[
            {"from":"5541999999999","id":"wamid.X","type":"text","text":{"body":"qual minha próxima tarefa?"}}
          ]},"field":"messages"}]}]}"#
            .as_bytes()
    }

    #[test]
    fn parse_inbound_extracts_from_text_and_phone_number_id() {
        let msgs = parse_inbound(sample_payload());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "5541999999999");
        assert_eq!(msgs[0].text, "qual minha próxima tarefa?");
        // CO-489: the tenant key rides through to the brain.
        assert_eq!(msgs[0].phone_number_id, "1238594552665575");
        // CO-489: the wamid is carried for at-least-once dedup.
        assert_eq!(msgs[0].wamid, "wamid.X");
    }

    #[test]
    fn parse_inbound_tolerates_missing_metadata() {
        let no_meta = r#"{"entry":[{"changes":[{"value":{"messages":[
            {"from":"55","type":"text","text":{"body":"oi"}}]}}]}]}"#
            .as_bytes();
        let msgs = parse_inbound(no_meta);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].phone_number_id, ""); // empty → brain falls back to default
        assert_eq!(msgs[0].wamid, ""); // no id → dedup skipped for this message
    }

    // --- CO-489: tier-2 scalability hardening -------------------------------

    #[test]
    fn dedup_first_seen_then_repeat_then_expired() {
        let ttl = Duration::from_secs(300);
        let t0 = Instant::now();
        let mut seen = HashMap::new();

        // First sighting → dispatch.
        assert!(dedup_decision(&mut seen, "wamid.A", t0, ttl));
        // Repeat within the TTL window → skip (Meta redelivery).
        assert!(!dedup_decision(
            &mut seen,
            "wamid.A",
            t0 + Duration::from_secs(60),
            ttl
        ));
        // A DIFFERENT wamid in the same window still dispatches.
        assert!(dedup_decision(
            &mut seen,
            "wamid.B",
            t0 + Duration::from_secs(61),
            ttl
        ));
        // Past the TTL the old entry is pruned, so the same wamid dispatches again.
        assert!(dedup_decision(
            &mut seen,
            "wamid.A",
            t0 + Duration::from_secs(360),
            ttl
        ));
    }

    #[test]
    fn reply_concurrency_parses_from_env_with_default() {
        // Unset → default.
        assert_eq!(parse_reply_concurrency(None), DEFAULT_REPLY_CONCURRENCY);
        // Valid numeric override (trimmed).
        assert_eq!(parse_reply_concurrency(Some("8".into())), 8);
        assert_eq!(parse_reply_concurrency(Some("  6 ".into())), 6);
        // Invalid / empty / zero → default (never an unbounded 0-permit semaphore).
        assert_eq!(
            parse_reply_concurrency(Some(String::new())),
            DEFAULT_REPLY_CONCURRENCY
        );
        assert_eq!(
            parse_reply_concurrency(Some("abc".into())),
            DEFAULT_REPLY_CONCURRENCY
        );
        assert_eq!(
            parse_reply_concurrency(Some("0".into())),
            DEFAULT_REPLY_CONCURRENCY
        );
    }

    #[test]
    fn shared_client_is_built_once() {
        // The OnceLock hands back the SAME instance every call (pooled + timed).
        assert!(std::ptr::eq(shared_client(), shared_client()));
    }

    #[test]
    fn parse_inbound_ignores_status_only_callbacks() {
        let status =
            br#"{"entry":[{"changes":[{"value":{"statuses":[{"status":"delivered"}]}}]}]}"#;
        assert!(parse_inbound(status).is_empty());
    }

    #[test]
    fn parse_brain_reply_extracts_reply() {
        assert_eq!(
            parse_brain_reply(
                r#"{"reply":"Tua próxima tarefa é X","intent":"next_task"}"#.as_bytes()
            ),
            Some("Tua próxima tarefa é X".to_string())
        );
        assert_eq!(parse_brain_reply(br#"{"reply":""}"#), None); // empty → fallback
        assert_eq!(parse_brain_reply(br#"{"intent":"chat"}"#), None); // missing
        assert_eq!(parse_brain_reply(b"not json"), None);
    }

    #[test]
    fn verify_signature_accepts_valid_and_rejects_forged() {
        let secret = "app-secret";
        let body = sample_payload();
        let good = format!("sha256={}", crate::billing::hmac_sha256_hex(secret, body));
        assert!(verify_signature(secret, body, &good));
        assert!(!verify_signature(secret, body, "sha256=deadbeef"));
        assert!(!verify_signature(secret, body, "deadbeef")); // missing prefix
        assert!(!verify_signature("wrong-secret", body, &good));
    }

    // --- CO-490: link endpoint helpers ---------------------------------------

    #[test]
    fn validate_whatsapp_rejects_empty_and_whitespace() {
        // The 400 path: empty / whitespace-only / no-digits → BadRequest.
        assert!(matches!(
            validate_whatsapp(""),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_whatsapp("   "),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_whatsapp("not-a-number"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn validate_whatsapp_normalizes_to_digits() {
        // Trims, strips `+`/spaces/dashes → bare decimal digits.
        assert_eq!(
            validate_whatsapp("  5541999999999 ").unwrap(),
            "5541999999999"
        );
        assert_eq!(
            validate_whatsapp("+55 (41) 99999-9999").unwrap(),
            "5541999999999"
        );
    }

    // --- CO-490 #3: stateless OTP helpers ---

    #[test]
    fn first_n_decimal_digits_filters_and_pads() {
        // Letters skipped, first eight digits kept (CO-490 R3: widened to 8).
        assert_eq!(first_n_decimal_digits("a1b2c3d4e5f6g7h8i9"), "12345678");
        // All-letters pathological input → padded to 8 zeros.
        assert_eq!(first_n_decimal_digits("abcdef"), "00000000");
    }

    #[test]
    fn otp_right_code_passes_wrong_fails() {
        let secret = "otp-secret";
        let now = 1_000_000u64;
        let code = compute_otp(secret, "usr_alice", "5541999999999", otp_bucket(now));
        // CO-490 R3: 8-digit OTP (was 6) to harden brute force.
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_digit()));

        // Right code in the current bucket verifies.
        assert!(otp_matches(
            secret,
            "usr_alice",
            "5541999999999",
            &code,
            now
        ));
        // Wrong code fails.
        assert!(!otp_matches(
            secret,
            "usr_alice",
            "5541999999999",
            "00000000",
            now
        ));
        // Right code but a DIFFERENT user fails (code binds the user_id).
        assert!(!otp_matches(secret, "usr_bob", "5541999999999", &code, now));
        // Right code but a DIFFERENT number fails.
        assert!(!otp_matches(
            secret,
            "usr_alice",
            "5541000000000",
            &code,
            now
        ));
    }

    #[test]
    fn otp_prev_bucket_passes_but_older_expires() {
        let secret = "otp-secret";
        // Code minted at the start of a bucket.
        let minted_at = 2_000 * OTP_PERIOD_SECS; // bucket boundary
        let code = compute_otp(secret, "usr_alice", "5541999999999", otp_bucket(minted_at));

        // Still inside the NEXT bucket → previous-bucket grace verifies.
        let next_bucket = minted_at + OTP_PERIOD_SECS + 1;
        assert!(otp_matches(
            secret,
            "usr_alice",
            "5541999999999",
            &code,
            next_bucket
        ));

        // Two buckets later → expired (outside current + previous window).
        let two_later = minted_at + 2 * OTP_PERIOD_SECS + 1;
        assert!(!otp_matches(
            secret,
            "usr_alice",
            "5541999999999",
            &code,
            two_later
        ));
    }

    #[test]
    fn build_identity_push_skips_when_secret_unset() {
        // No secret → None → push is skipped, link still succeeds.
        assert_eq!(
            build_identity_push(
                Some("http://x/identity".into()),
                None,
                "usr_alice",
                "5541999999999",
                "co_tok",
            ),
            None
        );
        // Empty secret is treated as unset.
        assert_eq!(
            build_identity_push(
                None,
                Some(String::new()),
                "usr_alice",
                "5541999999999",
                "co_tok"
            ),
            None
        );
    }

    #[test]
    fn build_identity_push_uses_default_url_when_unset() {
        let push = build_identity_push(
            None,
            Some("shh".into()),
            "usr_alice",
            "5541999999999",
            "co_tok",
        )
        .unwrap();
        assert_eq!(push.url, DEFAULT_IDENTITY_URL);
        assert_eq!(push.secret, "shh");
    }

    #[test]
    fn build_identity_push_builds_payload_and_header_secret() {
        let push = build_identity_push(
            Some("http://bot.local/identity".into()),
            Some("super-secret".into()),
            "usr_alice",
            "5541988887777",
            "co_abc123",
        )
        .unwrap();

        // URL + the value that becomes the X-Bot-Secret header.
        assert_eq!(push.url, "http://bot.local/identity");
        assert_eq!(push.secret, "super-secret");
        // Body carries exactly {whatsapp, token, user_id} (#1/#3e key the bot by user_id).
        assert_eq!(push.body["whatsapp"], "5541988887777");
        assert_eq!(push.body["token"], "co_abc123");
        assert_eq!(push.body["user_id"], "usr_alice");
        assert_eq!(
            push.body.as_object().map(|o| o.len()),
            Some(3),
            "body must contain whatsapp + token + user_id"
        );
    }

    #[test]
    fn bot_token_scopes_is_least_privilege() {
        let scopes = bot_token_scopes();
        // The least-privilege set: read/write entries, read universes, read
        // telemetry (CO-499 digest). All read except entries:write.
        assert!(scopes.contains("entries:read"));
        assert!(scopes.contains("entries:write"));
        assert!(scopes.contains("universes:read"));
        assert!(scopes.contains("telemetry:read"));
        // Never universes:write, admin surfaces, or agent dispatch.
        assert!(!scopes.contains("universes:write"));
        assert!(!scopes.contains("chat:read"));
        assert!(!scopes.contains("chat:write"));
        assert!(!scopes.contains("gestao:read"));
        assert!(!scopes.contains("funnel:read"));
        assert!(!scopes.contains("deployments:read"));
        assert!(!scopes.contains("agent:dispatch"));
        assert_eq!(
            scopes.len(),
            4,
            "exactly the four least-privilege scopes (entries r/w, universes:read, telemetry:read)"
        );
    }

    #[test]
    fn set_user_whatsapp_round_trips_and_is_scoped_to_caller() {
        use crate::storage::Storage;

        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path().to_str().unwrap());
        let alice = storage
            .create_user("alice@example.com", "Alice")
            .expect("create alice");
        let bob = storage
            .create_user("bob@example.com", "Bob")
            .expect("create bob");

        // Round-trip: setting Alice's number stores and reads back trimmed.
        let updated = storage
            .set_user_whatsapp(&alice.id, "  5541999990000 ")
            .expect("set whatsapp");
        assert_eq!(updated, 1, "exactly one row (the caller's) updated");
        assert_eq!(
            storage.get_user_whatsapp(&alice.id).as_deref(),
            Some("5541999990000")
        );

        // Scope: Bob is untouched by Alice's link.
        assert_eq!(storage.get_user_whatsapp(&bob.id), None);

        // Empty input is rejected at the storage layer too (defense in depth).
        assert!(storage.set_user_whatsapp(&alice.id, "   ").is_err());
    }
}
