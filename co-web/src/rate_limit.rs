//! Per-tier token bucket rate limiting and storage quota enforcement (CO-80).
//!
//! Token buckets are in-process (acceptable for v1 single-replica deployment).
//! Each bucket is keyed by `"{user_or_anon_key}:{op}"`.
//! Tier limits are declared in code and enforced in middleware.

use std::collections::HashMap;
use std::time::Instant;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode},
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
    pub fn parse(s: &str) -> Self {
        match s {
            "admin" => Tier::Admin,
            "pro" => Tier::Pro,
            "player" | "user" => Tier::User,
            _ => Tier::Anonymous,
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
        Tier::Anonymous => TierLimits {
            reads_per_min: Some(20),
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
    /// Returns `Ok(())` if allowed, `Err(retry_after_secs)` if rate limited.
    pub fn try_consume(&mut self) -> Result<(), u64> {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
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
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            buckets: HashMap::new(),
        }
    }

    /// Check and consume one token for (key, capacity_per_min).
    /// Returns `Ok(())` if allowed, `Err(retry_after_secs)` if limited.
    pub fn check(&mut self, key: &str, capacity_per_min: u64) -> Result<(), u64> {
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
// Identity helpers
// ---------------------------------------------------------------------------

/// Extract (user_id, tier) from request headers for authenticated users.
/// Returns `None` if the request has no valid JWT or session cookie.
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

/// Internal: get bucket key + tier for rate limiting.
/// Authenticated → (user_id, tier). Anonymous → (anon:{ip}, Anonymous).
fn extract_rate_limit_identity(headers: &HeaderMap) -> (String, Tier) {
    extract_auth_identity(headers).unwrap_or_else(|| {
        let ip = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        (format!("anon:{}", ip), Tier::Anonymous)
    })
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
// Rate limit middleware
// ---------------------------------------------------------------------------

pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();
    let method = req.method().clone();

    // Skip: non-API routes, CORS preflight, health endpoints.
    if !path.starts_with("/api/") || method == Method::OPTIONS || path.starts_with("/api/health") {
        return Ok(next.run(req).await);
    }

    let (bucket_key, tier) = extract_rate_limit_identity(req.headers());

    // Admin tier: unlimited, no bucket check.
    if tier == Tier::Admin {
        return Ok(next.run(req).await);
    }

    // CO-145: bulk-upload escape hatch. Authenticated callers can opt out of
    // the per-min cap by sending `X-Admin-Override-Quota: true`. Anonymous
    // requests cannot — the header is only honored when the JWT/session
    // resolves a real user. CO-90 keeps `tier` billing-only, so the admin
    // owns universes via owner_id, not a global tier bypass — and the
    // ownership check still runs inside the route handler.
    if tier != Tier::Anonymous && has_admin_override(req.headers()) {
        return Ok(next.run(req).await);
    }

    let op = OpClass::from_method(&method);
    let limits = tier_limits(tier);

    let capacity = match op {
        OpClass::Read => limits.reads_per_min,
        OpClass::Write => limits.writes_per_min,
    };

    let Some(capacity) = capacity else {
        return Ok(next.run(req).await);
    };

    let key = format!("{}:{}", bucket_key, op.key_suffix());

    let result = state
        .rate_limiter
        .lock()
        .map_err(|_| rate_limited_response(1))?
        .check(&key, capacity);

    match result {
        Ok(()) => Ok(next.run(req).await),
        Err(retry_after) => Err(rate_limited_response(retry_after)),
    }
}

fn rate_limited_response(retry_after: u64) -> Response {
    let body = json!({
        "error": "rate_limited",
        "message": "Muitas requisições. Tente novamente em instantes.",
        "message_en": "Too many requests. Please try again shortly.",
        "retry_after": retry_after,
    });
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    if let Ok(v) = retry_after.to_string().parse() {
        resp.headers_mut().insert("retry-after", v);
    }
    resp
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
        assert_eq!(Tier::parse("pro"), Tier::Pro);
        assert_eq!(Tier::parse("user"), Tier::User);
        assert_eq!(Tier::parse("player"), Tier::User);
        assert_eq!(Tier::parse("anonymous"), Tier::Anonymous);
        assert_eq!(Tier::parse(""), Tier::Anonymous);
    }

    #[test]
    fn test_tier_limits_anonymous() {
        let lim = tier_limits(Tier::Anonymous);
        assert_eq!(lim.reads_per_min, Some(20));
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
    fn test_tier_limits_admin_unlimited() {
        let lim = tier_limits(Tier::Admin);
        assert!(lim.reads_per_min.is_none());
        assert!(lim.writes_per_min.is_none());
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
        for i in 0..20 {
            assert!(
                limiter.check("anon:127.0.0.1:r", 20).is_ok(),
                "request {i} should be allowed"
            );
        }
        assert!(
            limiter.check("anon:127.0.0.1:r", 20).is_err(),
            "21st request should be rate limited"
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
}
