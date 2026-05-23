# Rust Architecture — CO Platform

## Module Ownership

| Crate | Path | Role |
|-------|------|------|
| `core` | `core/src/` | Shared types, config, validation |
| `co-cli` | `co-cli/src/` | CLI commands (`co init`, `co new`, etc.) |
| `co-web` | `co-web/src/` | Axum web server |
| `co-auto` | `dev/co-auto/src/` | Automated task pipeline |

## AppState Pattern

`co-web` uses segregated sub-states (CO-221):

```rust
// co-web/src/server/state.rs
pub struct AppState {
    pub storage: Arc<parking_lot::Mutex<Storage>>,
    pub auth: Arc<AuthState>,
    // …
}
```

- Use `parking_lot::Mutex` (not `std::sync::Mutex`) — avoids poison on panic (CO-203)
- Never hold the `Storage` lock across an `await` point

## Route Pattern

```rust
// co-web/src/routes/foo_routes.rs
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/foo", get(list_foo).post(create_foo))
        .with_state(state)
}

async fn list_foo(State(state): State<AppState>) -> impl IntoResponse {
    // …
}
```

Register in `co-web/src/server/router.rs`.

## Migration Pattern

```rust
// co-web/src/db/migrations/v{N}_{name}.sql
ALTER TABLE entries ADD COLUMN new_col TEXT NOT NULL DEFAULT '';
```

- One file per migration, numbered sequentially
- Use `ADD COLUMN ... DEFAULT ...` for backwards-compatible additions
- Never use `.ok()` to swallow a SELECT on a newly migrated column (CO-137)

## Testing

```rust
// Integration test (preferred)
#[tokio::test]
async fn test_endpoint() {
    let app = build_test_app().await;  // no real port, uses tower::ServiceExt
    let resp = app.oneshot(Request::builder()…).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```

- Bind to `127.0.0.1` only; never `0.0.0.0`
- Use `tempfile::tempdir()` for test databases
- Set `JWT_SECRET=test-secret` in test setup
