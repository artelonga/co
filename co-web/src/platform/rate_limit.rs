//! Per-tier token bucket rate limiting and storage quota enforcement.
//!
//! CO-80: original per-tier token buckets.
//! CO-397: adds abuse heuristics (404/401 tracking + temp bans), trusted-IP
//!         bypass via CO_TRUSTED_IPS, User-Agent gate, X-RateLimit-* headers,
//!         and a 10× authenticated budget over anonymous.
//!
//! Token buckets are in-process (acceptable for single-replica deployment).
//! Each bucket is keyed by `"{user_or_anon_key}:{op}"`.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::auth::extract_session_cookie;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Tier
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Anonymous,
    User,
    Pro,
    Admin,
}

impl Tier {
    /// 1.45.0 model collapse: there is only one authenticated tier — every
    /// authenticated user is an admin. Legacy tier values (`user`, `player`,
    /// `pro`) on existing user rows still parse cleanly; they all resolve to
    /// `Tier::Admin` at runtime so older accounts don't need a DB migration.
    /// `Tier::User` and `Tier::Pro` are kept as enum variants for the unit
    /// tests around `tier_limits` and historical comparison; they are no
    /// longer produced by `parse`.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "" | "anonymous" => Tier::Anonymous,
            _ => Tier::Admin,
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Anonymous => write!(f, "anonymous"),
            Tier::User => write!(f, "user"),
            Tier::Pro => write!(f, "pro"),
            Tier::Admin => write!(f, "admin"),
        }
    }
}

// ---------------------------------------------------------------------------
// Operation class
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpClass {
    Read,
    Write,
}

impl OpClass {
    fn from_method(method: &Method) -> Self {
        if *method == Method::GET || *method == Method::HEAD {
            OpClass::Read
        } else {
            OpClass::Write
        }
    }

    fn key_suffix(self) -> &'static str {
        match self {
            OpClass::Read => "r",
            OpClass::Write => "w",
        }
    }
}

// ---------------------------------------------------------------------------
// Tier limits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct TierLimits {
    /// Requests per minute. `None` = unlimited.
    pub reads_per_min: Option<u64>,
    pub writes_per_min: Option<u64>,
    /// Total entries across all owned universes. `None` = unlimited.
    pub storage_entries: Option<i64>,
    /// Maximum owned universes. `None` = unlimited.
    pub max_universes: Option<i64>,
}

pub fn tier_limits(tier: Tier) -> TierLimits {
    match tier {
        // CO-397: 60 reads/min for anonymous (down from 120).
        // A typical SPA load fetches ~10-15 requests; 60/min covers 4 loads
        // per minute while rate-limiting scrapers at the 61st request.
        Tier::Anonymous => TierLimits {
            reads_per_min: Some(60),
            writes_per_min: Some(5),
            storage_entries: Some(100),
            max_universes: Some(1),
        },
        Tier::User => TierLimits {
            reads_per_min: Some(200),
            writes_per_min: Some(60),
            storage_entries: Some(10_000),
            max_universes: Some(10),
        },
        Tier::Pro => TierLimits {
            reads_per_min: Some(2000),
            writes_per_min: Some(600),
            storage_entries: Some(1_000_000),
            max_universes: None,
        },
        // Admin tier storage limits remain unlimited for quota checks.
        // Rate limits for admin (all authenticated users) are enforced separately
        // in the middleware at 600 reads/min, 60 writes/min (CO-397).
        Tier::Admin => TierLimits {
            reads_per_min: None,
            writes_per_min: None,
            storage_entries: None,
            max_universes: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Token bucket
// ---------------------------------------------------------------------------

pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity_per_min: u64) -> Self {
        let cap = capacity_per_min as f64;
        TokenBucket {
            tokens: cap,
            capacity: cap,
            refill_rate: cap / 60.0,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token.
    /// Returns `Ok(remaining_floor)` if allowed, `Err(retry_after_secs)` if limited.
    pub fn try_consume(&mut self) -> Result<u64, u64> {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(self.tokens.floor() as u64)
        } else {
            let deficit = 1.0 - self.tokens;
            let retry_secs = (deficit / self.refill_rate).ceil() as u64;
            Err(retry_secs.max(1))
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

pub struct RateLimiter {
    buckets: HashMap<String, TokenBucket>,
    /// CO-397: abuse heuristics (co-packed to avoid a new IntegrationsState field).
    pub abuse: AbuseTracker,
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            buckets: HashMap::new(),
            abuse: AbuseTracker::new(),
        }
    }

    /// Check and consume one token for (key, capacity_per_min).
    /// Returns `Ok(remaining_floor)` if allowed, `Err(retry_after_secs)` if limited.
    pub fn check(&mut self, key: &str, capacity_per_min: u64) -> Result<u64, u64> {
        self.buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(capacity_per_min))
            .try_consume()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CO-397: Abuse heuristics — per-IP 404/401 tracking + temp bans
// ---------------------------------------------------------------------------

/// Per-IP counters for abuse heuristics (sliding 1-minute windows).
struct IpRecord {
    recent_404s: VecDeque<Instant>,
    recent_401s: VecDeque<Instant>,
    banned_until: Option<Instant>,
}

impl IpRecord {
    fn new() -> Self {
        Self {
            recent_404s: VecDeque::new(),
            recent_401s: VecDeque::new(),
            banned_until: None,
        }
    }
}

pub struct AbuseTracker {
    records: HashMap<String, IpRecord>,
}

impl Default for AbuseTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl AbuseTracker {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Returns true if this IP is currently under a temp ban.
    pub fn is_banned(&mut self, ip: &str) -> bool {
        let now = Instant::now();
        let Some(rec) = self.records.get_mut(ip) else {
            return false;
        };
        match rec.banned_until {
            Some(until) if until > now => true,
            Some(_) => {
                rec.banned_until = None;
                false
            }
            None => false,
        }
    }

    /// Record a non-2xx response status.
    /// Returns `Some(ban_kind)` if a new ban was applied, `None` otherwise.
    pub fn record_error(&mut self, ip: &str, status: u16) -> Option<&'static str> {
        let now = Instant::now();
        let rec = self
            .records
            .entry(ip.to_string())
            .or_insert_with(IpRecord::new);
        let cutoff = now - Duration::from_secs(60);

        match status {
            404 => {
                rec.recent_404s.retain(|t| *t > cutoff);
                rec.recent_404s.push_back(now);
                if rec.recent_404s.len() >= 30 {
                    rec.banned_until = Some(now + Duration::from_secs(15 * 60));
                    return Some("ban_404");
                }
            }
            401 => {
                rec.recent_401s.retain(|t| *t > cutoff);
                rec.recent_401s.push_back(now);
                if rec.recent_401s.len() >= 10 {
                    rec.banned_until = Some(now + Duration::from_secs(15 * 60));
                    return Some("ban_401");
                }
            }
            _ => {}
        }
        None
    }
}

// ---------------------------------------------------------------------------
// CO-397: Trusted IP bypass (CO_TRUSTED_IPS env var)
// ---------------------------------------------------------------------------

fn parse_cidr(s: &str) -> Option<(IpAddr, u8)> {
    if let Some((ip_str, bits_str)) = s.split_once('/') {
        let addr: IpAddr = ip_str.trim().parse().ok()?;
        let bits: u8 = bits_str.trim().parse().ok()?;
        Some((addr, bits))
    } else {
        let addr: IpAddr = s.trim().parse().ok()?;
        let bits = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        Some((addr, bits))
    }
}

fn ip_in_net(addr: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (addr, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            if prefix >= 32 {
                return a == n;
            }
            let mask: u32 = !0u32 << (32 - prefix);
            u32::from(a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            if prefix >= 128 {
                return a == n;
            }
            let mask: u128 = !0u128 << (128 - prefix);
            u128::from(a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}

fn is_trusted_ip(client_ip: &str) -> bool {
    let Ok(client_addr): Result<IpAddr, _> = client_ip.parse() else {
        return false;
    };
    let raw = crate::infra::secrets::global().get_or("CO_TRUSTED_IPS", "");
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| parse_cidr(s.trim()))
        .any(|(net, bits)| ip_in_net(client_addr, net, bits))
}

/// Returns true for loopback, RFC-1918 private, and "unknown" addresses.
/// Used to skip UA enforcement and abuse tracking for internal/test traffic.
fn is_private_or_loopback(ip_str: &str) -> bool {
    if ip_str == "unknown" {
        return true;
    }
    match ip_str.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => addr.is_loopback() || addr.is_private() || addr.octets()[0] == 0,
        Ok(IpAddr::V6(addr)) => addr.is_loopback(),
        Err(_) => true,
    }
}

// ---------------------------------------------------------------------------
// Identity helpers
// ---------------------------------------------------------------------------

/// Extract (user_id, tier) from request headers, JWT-only.
/// Returns `None` for unauthenticated or API-token-only requests. Callers that
/// also want long-lived API tokens resolved (e.g., the rate-limit middleware)
/// should use [`extract_auth_identity_with_token`].
pub fn extract_auth_identity(headers: &HeaderMap) -> Option<(String, Tier)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(headers))?;

    let secret = crate::auth::jwt_secret();
    crate::auth::decode_claims(&token, &secret)
        .ok()
        .map(|claims| (claims.sub, Tier::parse(&claims.tier)))
}

/// Like [`extract_auth_identity`] but also resolves long-lived API tokens
/// (CO-35) by looking up the `api_tokens` table and reading the owner's tier
/// from the `users` table. Without this, API tokens authenticate at the route
/// handler but fall through to the Anonymous-by-IP bucket here, so a single
/// admin running multiple background workers gets rate-limited as if it were
/// public traffic.
pub fn extract_auth_identity_with_token(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(String, Tier)> {
    if let Some(id) = extract_auth_identity(headers) {
        return Some(id);
    }

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_cookie(headers))?;

    let storage = state.core.storage.lock();
    let api_token = storage.get_api_token_by_value(&token).ok().flatten()?;
    let user = storage.get_user_by_id(&api_token.user_id)?;
    Some((api_token.user_id, Tier::parse(&user.tier)))
}

/// Internal: get bucket key + tier for rate limiting.
/// Authenticated → (user_id, tier). Anonymous → (anon:{ip}, Anonymous).
fn extract_rate_limit_identity(state: &AppState, headers: &HeaderMap) -> (String, Tier) {
    extract_auth_identity_with_token(state, headers).unwrap_or_else(|| {
        let ip = extract_client_ip(headers);
        (format!("anon:{}", ip), Tier::Anonymous)
    })
}

pub fn extract_client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

// ---------------------------------------------------------------------------
// Admin override
// ---------------------------------------------------------------------------

fn has_admin_override(headers: &HeaderMap) -> bool {
    headers
        .get("x-admin-override-quota")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// CO-397: Response helpers
// ---------------------------------------------------------------------------

fn rate_limited_response(retry_after: u64, limit: u64) -> Response {
    let body = json!({
        "error": "rate_limited",
        "retry_after_s": retry_after,
    });
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    let hdrs = resp.headers_mut();
    if let Ok(v) = retry_after.to_string().parse() {
        hdrs.insert("retry-after", v);
    }
    add_rate_limit_headers(hdrs, limit, 0, unix_now() + retry_after);
    resp
}

fn missing_ua_response() -> Response {
    let body = json!({ "error": "missing_user_agent" });
    (StatusCode::BAD_REQUEST, axum::Json(body)).into_response()
}

fn banned_response() -> Response {
    let body = json!({
        "error": "rate_limited",
        "retry_after_s": 900u64,
    });
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    resp.headers_mut()
        .insert("retry-after", HeaderValue::from_static("900"));
    resp
}

fn add_rate_limit_headers(
    headers: &mut axum::http::HeaderMap,
    limit: u64,
    remaining: u64,
    reset: u64,
) {
    fn hv(n: u64) -> HeaderValue {
        HeaderValue::from_str(&n.to_string()).unwrap_or(HeaderValue::from_static("0"))
    }
    headers.insert(HeaderName::from_static("x-ratelimit-limit"), hv(limit));
    headers.insert(
        HeaderName::from_static("x-ratelimit-remaining"),
        hv(remaining),
    );
    headers.insert(HeaderName::from_static("x-ratelimit-reset"), hv(reset));
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// CO-397: Abuse logging to atividades
// ---------------------------------------------------------------------------

fn log_abuse_event(state: &AppState, kind: &str, ip: &str, ua: Option<&str>) {
    use crate::atividade::sha256_short;
    use crate::atividade::{Acao, Atividade, Tipo, log_atividade};

    let ip_hash = sha256_short(ip);
    tracing::warn!(co_api_abuse_kind = %kind, ip_hash = %ip_hash, "rate limit abuse event");

    log_atividade(
        state.clone(),
        Atividade {
            acao: Acao::Criar,
            entidade: "api_abuse".to_string(),
            entidade_id: Some(kind.to_string()),
            before: None,
            after: Some(json!({"kind": kind, "ip_hash": ip_hash})),
            tipo: Tipo::Sistema,
            user_id: None,
            ip: Some(ip.to_string()),
            user_agent: ua.map(String::from),
        },
    );
}

// ---------------------------------------------------------------------------
// Rate limit middleware
// ---------------------------------------------------------------------------

pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let method = req.method().clone();

    // CO-208: test-only bypass
    if state.core.config.bypass_rate_limit && state.core.config.co_env == "test" {
        return next.run(req).await;
    }

    // Skip: non-API routes, CORS preflight, health endpoints.
    if !path.starts_with("/api/") || method == Method::OPTIONS || path.starts_with("/api/health") {
        return next.run(req).await;
    }

    let headers = req.headers().clone();
    let client_ip = extract_client_ip(&headers);

    // CO-397: trusted IP bypass (CO_TRUSTED_IPS=CSV of CIDRs)
    if is_trusted_ip(&client_ip) {
        return next.run(req).await;
    }

    let is_public_ip = !is_private_or_loopback(&client_ip);

    // CO-397: User-Agent gate — reject empty/single-char UA with 400.
    // Only enforced for publicly-routed IPs; loopback, RFC-1918 private,
    // and unresolved ("unknown") addresses are exempt so that internal
    // services and unit tests aren't affected.
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok());
    if is_public_ip && ua.map(|s| s.trim().len()).unwrap_or(0) <= 1 {
        log_abuse_event(&state, "missing_user_agent", &client_ip, ua);
        return missing_ua_response();
    }

    let op = OpClass::from_method(&method);
    let (bucket_key, tier) = extract_rate_limit_identity(&state, &headers);

    // CO-145: admin override header bypasses rate limits for authenticated users.
    if tier == Tier::Admin && has_admin_override(&headers) {
        return next.run(req).await;
    }

    // CO-397: authenticated (Admin) budget = 600r/60w per min (10× anonymous).
    let capacity = match tier {
        Tier::Admin => match op {
            OpClass::Read => 600u64,
            OpClass::Write => 60u64,
        },
        _ => {
            let limits = tier_limits(tier);
            let cap = match op {
                OpClass::Read => limits.reads_per_min,
                OpClass::Write => limits.writes_per_min,
            };
            let Some(cap) = cap else {
                // Unlimited tier — pass through.
                return next.run(req).await;
            };
            cap
        }
    };

    let key = format!("{}:{}", bucket_key, op.key_suffix());
    let reset_epoch = unix_now() + 60;

    // Check ban AND consume rate-limit token in a single locked section (no lock across .await).
    let check_result = {
        let mut guard = state
            .integrations
            .rate_limiter
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        if guard.abuse.is_banned(&client_ip) {
            return banned_response();
        }

        guard.check(&key, capacity)
    };

    match check_result {
        Ok(remaining) => {
            let mut resp = next.run(req).await;

            // Track non-2xx for abuse heuristics — only for publicly-routed IPs.
            // Private/loopback IPs are exempt so test traffic never triggers bans.
            if is_public_ip {
                let status = resp.status().as_u16();
                let ban_kind = {
                    let mut guard = state
                        .integrations
                        .rate_limiter
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    guard.abuse.record_error(&client_ip, status)
                };
                if let Some(kind) = ban_kind {
                    log_abuse_event(&state, kind, &client_ip, ua);
                }
            }

            add_rate_limit_headers(resp.headers_mut(), capacity, remaining, reset_epoch);
            resp
        }
        Err(retry_after) => {
            log_abuse_event(&state, "rate_limited", &client_ip, ua);
            rate_limited_response(retry_after, capacity)
        }
    }
}

// ---------------------------------------------------------------------------
// Storage quota checks
// ---------------------------------------------------------------------------

/// Check that the authenticated user has not exceeded their tier's storage quota.
///
/// Admin tier with `X-Admin-Override-Quota: true` bypasses the check (audit logged).
pub fn check_storage_quota(
    storage: &crate::storage::Storage,
    user_id: &str,
    tier: Tier,
    headers: &HeaderMap,
) -> Result<(), crate::error::AppError> {
    if tier == Tier::Admin && has_admin_override(headers) {
        tracing::warn!(user_id, "Admin quota override used for storage quota check");
        return Ok(());
    }

    let limits = tier_limits(tier);
    let Some(limit) = limits.storage_entries else {
        return Ok(());
    };

    let used = storage.count_user_entries(user_id);
    if used >= limit {
        return Err(crate::error::AppError::StorageQuotaExceeded {
            used,
            limit,
            tier: tier.to_string(),
        });
    }
    Ok(())
}

/// Check that the authenticated user has not exceeded their tier's universe count quota.
///
/// Admin tier with `X-Admin-Override-Quota: true` bypasses the check (audit logged).
pub fn check_universe_quota(
    storage: &crate::storage::Storage,
    user_id: &str,
    tier: Tier,
    headers: &HeaderMap,
) -> Result<(), crate::error::AppError> {
    if tier == Tier::Admin && has_admin_override(headers) {
        tracing::warn!(
            user_id,
            "Admin quota override used for universe count check"
        );
        return Ok(());
    }

    let limits = tier_limits(tier);
    let Some(limit) = limits.max_universes else {
        return Ok(());
    };

    let used = storage.count_user_universes(user_id);
    if used >= limit {
        return Err(crate::error::AppError::StorageQuotaExceeded {
            used,
            limit,
            tier: tier.to_string(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_from_str() {
        assert_eq!(Tier::parse("admin"), Tier::Admin);
        assert_eq!(Tier::parse("pro"), Tier::Admin);
        assert_eq!(Tier::parse("user"), Tier::Admin);
        assert_eq!(Tier::parse("player"), Tier::Admin);
        assert_eq!(Tier::parse("anonymous"), Tier::Anonymous);
        assert_eq!(Tier::parse(""), Tier::Anonymous);
    }

    #[test]
    fn test_tier_limits_anonymous() {
        let lim = tier_limits(Tier::Anonymous);
        // CO-397: changed from 120 to 60 reads/min.
        assert_eq!(lim.reads_per_min, Some(60));
        assert_eq!(lim.writes_per_min, Some(5));
        assert_eq!(lim.storage_entries, Some(100));
        assert_eq!(lim.max_universes, Some(1));
    }

    #[test]
    fn test_tier_limits_user() {
        let lim = tier_limits(Tier::User);
        assert_eq!(lim.reads_per_min, Some(200));
        assert_eq!(lim.writes_per_min, Some(60));
        assert_eq!(lim.storage_entries, Some(10_000));
        assert_eq!(lim.max_universes, Some(10));
    }

    #[test]
    fn test_tier_limits_pro() {
        let lim = tier_limits(Tier::Pro);
        assert_eq!(lim.reads_per_min, Some(2000));
        assert_eq!(lim.writes_per_min, Some(600));
        assert_eq!(lim.storage_entries, Some(1_000_000));
        assert_eq!(lim.max_universes, None);
    }

    #[test]
    fn test_tier_limits_admin_unlimited_storage() {
        let lim = tier_limits(Tier::Admin);
        // Storage quota stays unlimited; rate limits are applied in middleware separately.
        assert!(lim.storage_entries.is_none());
        assert!(lim.max_universes.is_none());
    }

    #[test]
    fn test_token_bucket_depletes_and_rejects() {
        let mut bucket = TokenBucket::new(3);
        assert!(bucket.try_consume().is_ok(), "1st allowed");
        assert!(bucket.try_consume().is_ok(), "2nd allowed");
        assert!(bucket.try_consume().is_ok(), "3rd allowed");
        assert!(bucket.try_consume().is_err(), "4th should be rejected");
    }

    #[test]
    fn test_token_bucket_returns_remaining() {
        let mut bucket = TokenBucket::new(5);
        let r0 = bucket.try_consume().unwrap();
        assert_eq!(r0, 4, "4 remaining after 1st consume");
        let r1 = bucket.try_consume().unwrap();
        assert_eq!(r1, 3, "3 remaining after 2nd consume");
    }

    #[test]
    fn test_token_bucket_retry_after_positive() {
        let mut bucket = TokenBucket::new(1);
        assert!(bucket.try_consume().is_ok());
        let retry = bucket.try_consume().unwrap_err();
        assert!(
            retry >= 1,
            "retry_after must be at least 1 second, got {retry}"
        );
    }

    #[test]
    fn test_rate_limiter_anonymous_read_limit() {
        let mut limiter = RateLimiter::new();
        for i in 0..60 {
            assert!(
                limiter.check("anon:127.0.0.1:r", 60).is_ok(),
                "request {i} should be allowed"
            );
        }
        assert!(
            limiter.check("anon:127.0.0.1:r", 60).is_err(),
            "61st request should be rate limited"
        );
    }

    #[test]
    fn test_rate_limiter_authenticated_has_10x_budget() {
        let mut limiter = RateLimiter::new();
        // Authenticated budget = 600 reads/min (10× anonymous 60)
        for i in 0..600 {
            assert!(
                limiter.check("user-id:r", 600).is_ok(),
                "request {i} should be allowed for authenticated tier"
            );
        }
        assert!(
            limiter.check("user-id:r", 600).is_err(),
            "601st request should be rate limited for authenticated tier"
        );
    }

    #[test]
    fn test_rate_limiter_pro_read_limit() {
        let mut limiter = RateLimiter::new();
        for i in 0..2000 {
            assert!(
                limiter.check("pro-user-id:r", 2000).is_ok(),
                "request {i} should be allowed for pro tier"
            );
        }
        assert!(
            limiter.check("pro-user-id:r", 2000).is_err(),
            "2001st request should be rate limited for pro tier"
        );
    }

    #[test]
    fn test_rate_limiter_separate_buckets_per_user() {
        let mut limiter = RateLimiter::new();
        for _ in 0..20 {
            assert!(limiter.check("user-a:r", 20).is_ok());
        }
        assert!(limiter.check("user-a:r", 20).is_err(), "user-a exhausted");
        assert!(
            limiter.check("user-b:r", 20).is_ok(),
            "user-b independent bucket"
        );
    }

    #[test]
    fn test_rate_limiter_read_write_separate_buckets() {
        let mut limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check("user-x:w", 5).is_ok());
        }
        assert!(limiter.check("user-x:w", 5).is_err(), "writes exhausted");
        assert!(limiter.check("user-x:r", 20).is_ok(), "reads independent");
    }

    // ---------------------------------------------------------------------------
    // Abuse tracker tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_abuse_tracker_not_banned_initially() {
        let mut tracker = AbuseTracker::new();
        assert!(!tracker.is_banned("1.2.3.4"));
    }

    #[test]
    fn test_abuse_tracker_ban_on_30_404s() {
        let mut tracker = AbuseTracker::new();
        for i in 0..29 {
            let result = tracker.record_error("1.2.3.4", 404);
            assert!(result.is_none(), "no ban before 30th 404 (at {i})");
        }
        let result = tracker.record_error("1.2.3.4", 404);
        assert_eq!(result, Some("ban_404"), "30th 404 triggers ban");
        assert!(tracker.is_banned("1.2.3.4"), "IP is now banned");
    }

    #[test]
    fn test_abuse_tracker_ban_on_10_401s() {
        let mut tracker = AbuseTracker::new();
        for i in 0..9 {
            let result = tracker.record_error("5.6.7.8", 401);
            assert!(result.is_none(), "no ban before 10th 401 (at {i})");
        }
        let result = tracker.record_error("5.6.7.8", 401);
        assert_eq!(result, Some("ban_401"), "10th 401 triggers ban");
        assert!(tracker.is_banned("5.6.7.8"), "IP is now banned");
    }

    #[test]
    fn test_abuse_tracker_independent_ips() {
        let mut tracker = AbuseTracker::new();
        for _ in 0..30 {
            tracker.record_error("1.1.1.1", 404);
        }
        assert!(tracker.is_banned("1.1.1.1"));
        assert!(!tracker.is_banned("2.2.2.2"), "different IP not affected");
    }

    // ---------------------------------------------------------------------------
    // Trusted IP tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_ip_in_net_v4_exact() {
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let n: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(ip_in_net(a, n, 32));
        let other: IpAddr = "127.0.0.2".parse().unwrap();
        assert!(!ip_in_net(other, n, 32));
    }

    #[test]
    fn test_ip_in_net_v4_slash24() {
        let a: IpAddr = "192.168.1.100".parse().unwrap();
        let n: IpAddr = "192.168.1.0".parse().unwrap();
        assert!(ip_in_net(a, n, 24));
        let outside: IpAddr = "192.168.2.1".parse().unwrap();
        assert!(!ip_in_net(outside, n, 24));
    }

    #[test]
    fn test_parse_cidr_with_slash32() {
        let (addr, bits) = parse_cidr("127.0.0.1/32").unwrap();
        assert_eq!(bits, 32);
        assert_eq!(addr, "127.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_parse_cidr_without_slash() {
        let (addr, bits) = parse_cidr("10.0.0.1").unwrap();
        assert_eq!(bits, 32);
        assert_eq!(addr, "10.0.0.1".parse::<IpAddr>().unwrap());
    }
}
