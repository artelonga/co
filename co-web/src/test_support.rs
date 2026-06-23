//! Shared test-only utilities (CO-477).
//!
//! ## Why this module exists
//!
//! Many `co-web` unit tests configure secrets/feature-flags by mutating the
//! **shared process environment** (`std::env::set_var` / `remove_var`) — most
//! commonly `JWT_SECRET`. Those mutations are global to the test *process*, so
//! under default parallel `cargo test` one test's `set_var`/`remove_var` can
//! land in the middle of another test's set → build-state → sign → request →
//! assert window.
//!
//! The classic symptom: a test signs a JWT with `JWT_SECRET="test-jwt-secret"`,
//! a concurrent test rewrites `JWT_SECRET` to a different value (e.g.
//! `"test-secret"`) before the first test's request reaches the auth gate, and
//! the gate — which re-reads `JWT_SECRET` from the live environment via
//! [`crate::infra::secrets::EnvSecretsProvider`] — rejects the now-mismatched
//! signature with `401` instead of the expected `200`.
//!
//! Before CO-477 each module that hit this grew its **own** lock
//! (`ENV_LOCK` in `kb_routes`, `ADMIN_ENV_LOCK` in `lead_routes`, `ENV_MUTEX`
//! in `webhook`, …). Separate mutexes don't actually serialize against each
//! other, so a guarded test in one module still raced an unguarded — or
//! differently-guarded — test in another module.
//!
//! ## The fix
//!
//! A single **process-wide** lock. Every test that mutates the process
//! environment acquires this one lock for the full duration of its
//! env-sensitive work, guaranteeing mutual exclusion across *all* modules.
//!
//! - Async (`#[tokio::test]`) tests: `let _env = env_lock().await;`
//! - Sync (`#[test]`) tests: `let _env = env_lock_blocking();`
//!
//! Both acquire the same underlying [`tokio::sync::Mutex`], so sync and async
//! env-mutating tests are serialized against each other too. A
//! `tokio::sync::Mutex` (rather than `std::sync::Mutex`) is used so async tests
//! can hold the guard across `.await` points without tripping the
//! `clippy::await_holding_lock` lint.
//!
//! The guard is poison-free by construction (`tokio::sync::Mutex` does not
//! poison), so a panicking test never wedges the whole suite.

use tokio::sync::{Mutex, MutexGuard};

/// The single process-wide environment lock. See the module docs.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

/// Acquire the process-wide environment lock from an **async** test.
///
/// ```ignore
/// #[tokio::test]
/// async fn my_test() {
///     let _env = co_web::test_support::env_lock().await;
///     unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
///     // ... build state, sign token, run request, assert ...
/// }
/// ```
pub async fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// Acquire the process-wide environment lock from a **synchronous** test.
///
/// Must be called from outside a Tokio runtime (i.e. a plain `#[test]`), which
/// is the case for every non-async unit test.
///
/// ```ignore
/// #[test]
/// fn my_test() {
///     let _env = co_web::test_support::env_lock_blocking();
///     unsafe { std::env::set_var("CO_KB_TOKEN", "secret") };
///     // ... assert ...
/// }
/// ```
pub fn env_lock_blocking() -> MutexGuard<'static, ()> {
    ENV_LOCK.blocking_lock()
}
