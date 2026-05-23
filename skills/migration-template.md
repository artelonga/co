# Migration Template — CO Platform

## File Location

```
co-web/src/db/migrations/v{N}_{description}.sql
```

Migrations run in numeric order on startup.

## Safe ALTER TABLE Pattern

```sql
-- v14_add_content_hash.sql
ALTER TABLE entries ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
```

**Rules:**
- Use `ADD COLUMN ... DEFAULT ...` for backwards compatibility
- New columns must have a `DEFAULT` value so existing rows are valid
- Never assume a new column exists in a SELECT without checking the migration version

## Anti-Pattern (CO-137 incident)

```rust
// WRONG — swallowing a SELECT error on a newly added column:
let hash = row.get::<_, String>("content_hash").ok();  // silently returns None if column missing

// CORRECT — let it fail loudly:
let hash = row.get::<_, String>("content_hash")?;
```

## Migration Registration

```rust
// co-web/src/db/migrations.rs
const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, sql: include_str!("migrations/v1_init.sql") },
    // …
    Migration { version: 14, sql: include_str!("migrations/v14_add_content_hash.sql") },
];
```

## Testing Migrations

Write an integration test that:
1. Creates a fresh in-memory SQLite DB
2. Applies all migrations
3. Verifies schema is consistent with model queries

```rust
#[tokio::test]
async fn migrations_apply_cleanly() {
    let db = Storage::open_in_memory().await.unwrap();
    // Verify all expected columns exist
}
```
