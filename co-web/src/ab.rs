//! A/B testing primitives — CO-121.
//!
//! Provides stable variant assignment (`assign`) and exposure logging (`expose`).
//! Assignment uses blake3(salt :: flag_key :: user_id) → bucket → variant.
//! Exposures write to OLTP (`ab_exposures`) and the telemetry pipeline for WAE.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

// Seed file embedded at compile time so the server ships its own default flags.
const SEED_FEATURE_FLAGS_YAML: &str = include_str!("../seed/feature_flags.yaml");

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub flag_key: String,
    pub description: String,
    /// JSON: `["control","treatment"]` or weighted `[{"v":"a","w":50},...]`
    pub variants: serde_json::Value,
    pub salt: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFlagRequest {
    pub flag_key: String,
    pub description: String,
    pub variants: serde_json::Value,
    pub salt: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleFlagRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SeedFlag {
    flag_key: String,
    description: String,
    variants: String,
    salt: String,
    enabled: bool,
}

// ---------------------------------------------------------------------------
// Assignment algorithm
// ---------------------------------------------------------------------------

/// Map a `(user_id, flag)` pair to a stable bucket in [0, 100).
/// Uses blake3(salt :: flag_key :: user_id) so rotating the salt re-randomises.
pub fn compute_bucket(user_id: &str, flag: &FeatureFlag) -> u64 {
    let input = format!("{}::{}::{}", flag.salt, flag.flag_key, user_id);
    let h = blake3::hash(input.as_bytes());
    u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap()) % 100
}

/// Select the variant for a given bucket from the flag's variant spec.
///
/// Supports two formats:
/// - Simple array: `["control","treatment"]` — even split
/// - Weighted array: `[{"v":"control","w":50},{"v":"treatment","w":50}]`
pub fn variant_for_bucket(flag: &FeatureFlag, bucket: u64) -> &str {
    let arr = match flag.variants.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return "control",
    };

    // Detect weighted format by checking if first element is an object.
    if arr[0].is_object() {
        let mut cumulative: u64 = 0;
        for item in arr {
            let v = item.get("v").and_then(|x| x.as_str()).unwrap_or("control");
            let w = item.get("w").and_then(|x| x.as_u64()).unwrap_or(0);
            cumulative += w;
            if bucket < cumulative {
                return v;
            }
        }
        return "control";
    }

    // Simple equal-split array.
    let n = arr.len() as u64;
    let idx = (bucket % n) as usize;
    arr[idx].as_str().unwrap_or("control")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the stable variant for `(user_id, flag_key)`.
///
/// On first call the assignment is computed and persisted via
/// `INSERT OR IGNORE INTO ab_assignments`. Subsequent calls for the same
/// `(user_id, flag_key)` return the stored variant without re-computing,
/// so the assignment survives hash-function changes as long as the row lives.
///
/// Returns `None` when the flag does not exist or is disabled.
pub fn assign(
    conn: &Connection,
    user_id: &str,
    flag_key: &str,
) -> rusqlite::Result<Option<String>> {
    let flag = load_flag(conn, flag_key)?;
    let flag = match flag {
        Some(f) if f.enabled => f,
        _ => return Ok(None),
    };

    // Return the existing row if already assigned.
    if let Some(variant) = conn
        .query_row(
            "SELECT variant FROM ab_assignments WHERE user_id = ?1 AND flag_key = ?2",
            params![user_id, flag_key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Some(variant));
    }

    // Compute and persist new assignment.
    let bucket = compute_bucket(user_id, &flag);
    let variant = variant_for_bucket(&flag, bucket).to_string();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT OR IGNORE INTO ab_assignments (user_id, flag_key, variant, assigned_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![user_id, flag_key, variant, now],
    )?;

    // Read back — handles the race where another writer won the INSERT OR IGNORE.
    let stored = conn.query_row(
        "SELECT variant FROM ab_assignments WHERE user_id = ?1 AND flag_key = ?2",
        params![user_id, flag_key],
        |r| r.get::<_, String>(0),
    )?;
    Ok(Some(stored))
}

/// Record that `user_id` was exposed to `variant` of `flag_key`.
///
/// Writes to:
/// 1. `ab_exposures` — durable OLTP record, indexed for batch export.
/// 2. `telemetry_events` — feeds the WAE pipeline (CO-118).
pub fn expose(
    conn: &Connection,
    user_id: &str,
    flag_key: &str,
    variant: &str,
    universe_id: Option<&str>,
) -> rusqlite::Result<()> {
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO ab_exposures (user_id, flag_key, variant, universe_id, exposed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id, flag_key, variant, universe_id, now],
    )?;

    // Mirror to telemetry_events so the WAE pipeline (CO-118) picks it up.
    let props = serde_json::json!({
        "flag_key": flag_key,
        "variant": variant,
    });
    let _ = conn.execute(
        "INSERT INTO telemetry_events \
         (timestamp, event_type, event_name, universe_key, user_id, properties) \
         VALUES (?1, 'ab_exposure', 'ab.exposure', ?2, ?3, ?4)",
        params![
            now,
            universe_id,
            user_id,
            serde_json::to_string(&props).unwrap_or_default(),
        ],
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Admin helpers
// ---------------------------------------------------------------------------

/// Load a single flag by key.
pub fn load_flag(conn: &Connection, flag_key: &str) -> rusqlite::Result<Option<FeatureFlag>> {
    conn.query_row(
        "SELECT flag_key, description, variants, salt, enabled, created_at, updated_at \
         FROM feature_flags WHERE flag_key = ?1",
        params![flag_key],
        row_to_flag,
    )
    .optional()
}

/// List all flags.
pub fn list_flags(conn: &Connection) -> rusqlite::Result<Vec<FeatureFlag>> {
    let mut stmt = conn.prepare(
        "SELECT flag_key, description, variants, salt, enabled, created_at, updated_at \
         FROM feature_flags ORDER BY flag_key",
    )?;
    let rows = stmt.query_map([], row_to_flag)?;
    rows.collect()
}

/// Create a new flag. Returns `false` if the key already exists.
pub fn create_flag(conn: &Connection, req: &CreateFlagRequest) -> rusqlite::Result<bool> {
    let variants_str = serde_json::to_string(&req.variants).unwrap_or_else(|_| "[]".into());
    let now = Utc::now().to_rfc3339();
    let rows = conn.execute(
        "INSERT OR IGNORE INTO feature_flags \
         (flag_key, description, variants, salt, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            req.flag_key,
            req.description,
            variants_str,
            req.salt,
            req.enabled as i64,
            now,
        ],
    )?;
    Ok(rows > 0)
}

/// Enable or disable a flag.
pub fn toggle_flag(conn: &Connection, flag_key: &str, enabled: bool) -> rusqlite::Result<bool> {
    let now = Utc::now().to_rfc3339();
    let rows = conn.execute(
        "UPDATE feature_flags SET enabled = ?1, updated_at = ?2 WHERE flag_key = ?3",
        params![enabled as i64, now, flag_key],
    )?;
    Ok(rows > 0)
}

// ---------------------------------------------------------------------------
// Seed
// ---------------------------------------------------------------------------

/// Seed flags from the embedded YAML file. Idempotent — uses INSERT OR IGNORE.
pub fn seed_flags(conn: &Connection) -> rusqlite::Result<()> {
    let flags: Vec<SeedFlag> = serde_yaml::from_str(SEED_FEATURE_FLAGS_YAML).unwrap_or_default();
    let now = Utc::now().to_rfc3339();
    for f in flags {
        conn.execute(
            "INSERT OR IGNORE INTO feature_flags \
             (flag_key, description, variants, salt, enabled, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                f.flag_key,
                f.description,
                f.variants,
                f.salt,
                f.enabled as i64,
                now,
            ],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn row_to_flag(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeatureFlag> {
    let variants_str: String = row.get(2)?;
    Ok(FeatureFlag {
        flag_key: row.get(0)?,
        description: row.get(1)?,
        variants: serde_json::from_str(&variants_str).unwrap_or_default(),
        salt: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE feature_flags (
                flag_key    TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                variants    TEXT NOT NULL,
                salt        TEXT NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE TABLE ab_assignments (
                user_id     TEXT NOT NULL,
                flag_key    TEXT NOT NULL,
                variant     TEXT NOT NULL,
                assigned_at TEXT NOT NULL,
                PRIMARY KEY (user_id, flag_key)
            );
            CREATE TABLE ab_exposures (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     TEXT NOT NULL,
                flag_key    TEXT NOT NULL,
                variant     TEXT NOT NULL,
                universe_id TEXT,
                exposed_at  TEXT NOT NULL
            );
            CREATE TABLE telemetry_events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp    TEXT NOT NULL,
                visitor_token TEXT,
                user_id      TEXT,
                session_id   TEXT,
                event_type   TEXT NOT NULL,
                event_name   TEXT NOT NULL,
                universe_key TEXT,
                path         TEXT,
                properties   TEXT,
                duration_ms  INTEGER,
                ip_hash      TEXT,
                ua_device    TEXT,
                ua_browser   TEXT,
                ua_os        TEXT
            );",
        )
        .unwrap();
        conn
    }

    fn insert_test_flag(conn: &Connection, enabled: bool) {
        conn.execute(
            "INSERT INTO feature_flags \
             (flag_key, description, variants, salt, enabled, created_at, updated_at) \
             VALUES ('test_flag', 'Test flag', '[\"control\",\"treatment\"]', 'salt1', ?1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            params![enabled as i64],
        ).unwrap();
    }

    #[test]
    fn test_assign_stable_1000_calls() {
        let conn = setup_db();
        insert_test_flag(&conn, true);

        let first = assign(&conn, "user-42", "test_flag").unwrap().unwrap();
        for _ in 1..1000 {
            let v = assign(&conn, "user-42", "test_flag").unwrap().unwrap();
            assert_eq!(v, first, "variant must be stable across all calls");
        }
    }

    #[test]
    fn test_assign_disabled_flag_returns_none() {
        let conn = setup_db();
        insert_test_flag(&conn, false);
        let result = assign(&conn, "user-1", "test_flag").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_assign_missing_flag_returns_none() {
        let conn = setup_db();
        let result = assign(&conn, "user-1", "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_expose_writes_oltp_and_telemetry() {
        let conn = setup_db();
        expose(&conn, "user-1", "test_flag", "control", Some("u-abc")).unwrap();

        let exposure_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM ab_exposures", [], |r| r.get(0))
            .unwrap();
        assert_eq!(exposure_count, 1);

        let telemetry_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE event_type = 'ab_exposure'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(telemetry_count, 1);
    }

    #[test]
    fn test_compute_bucket_deterministic() {
        let flag = FeatureFlag {
            flag_key: "test".into(),
            description: String::new(),
            variants: serde_json::json!(["control", "treatment"]),
            salt: "salt123".into(),
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let b1 = compute_bucket("user-abc", &flag);
        let b2 = compute_bucket("user-abc", &flag);
        assert_eq!(b1, b2, "bucket must be deterministic");
        assert!(b1 < 100, "bucket must be in [0, 100)");
    }

    #[test]
    fn test_variant_for_bucket_even_split() {
        let flag = FeatureFlag {
            flag_key: "f".into(),
            description: String::new(),
            variants: serde_json::json!(["control", "treatment"]),
            salt: "s".into(),
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert_eq!(variant_for_bucket(&flag, 0), "control");
        assert_eq!(variant_for_bucket(&flag, 1), "treatment");
        assert_eq!(variant_for_bucket(&flag, 2), "control");
    }

    #[test]
    fn test_variant_for_bucket_weighted() {
        let flag = FeatureFlag {
            flag_key: "f".into(),
            description: String::new(),
            variants: serde_json::json!([{"v":"control","w":90},{"v":"treatment","w":10}]),
            salt: "s".into(),
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert_eq!(variant_for_bucket(&flag, 0), "control");
        assert_eq!(variant_for_bucket(&flag, 89), "control");
        assert_eq!(variant_for_bucket(&flag, 90), "treatment");
        assert_eq!(variant_for_bucket(&flag, 99), "treatment");
    }

    #[test]
    fn test_create_and_toggle_flag() {
        let conn = setup_db();
        let req = CreateFlagRequest {
            flag_key: "new_flag".into(),
            description: "A new flag".into(),
            variants: serde_json::json!(["control", "treatment"]),
            salt: "salt-x".into(),
            enabled: false,
        };
        assert!(create_flag(&conn, &req).unwrap());
        // Duplicate insert returns false
        assert!(!create_flag(&conn, &req).unwrap());

        assert!(toggle_flag(&conn, "new_flag", true).unwrap());
        let flag = load_flag(&conn, "new_flag").unwrap().unwrap();
        assert!(flag.enabled);
    }

    #[test]
    fn test_seed_flags_idempotent() {
        let conn = setup_db();
        seed_flags(&conn).unwrap();
        seed_flags(&conn).unwrap(); // second call must not error
        let flags = list_flags(&conn).unwrap();
        assert!(!flags.is_empty(), "seed must produce at least one flag");
        let home_flag = flags.iter().find(|f| f.flag_key == "home_v2_layout");
        assert!(home_flag.is_some(), "home_v2_layout must be seeded");
    }

    #[test]
    fn test_assign_different_users_may_differ() {
        let conn = setup_db();
        insert_test_flag(&conn, true);
        // With a 50/50 split and a deterministic hash, most user pairs differ.
        let mut variants = std::collections::HashSet::new();
        for i in 0..20 {
            let v = assign(&conn, &format!("user-{i}"), "test_flag")
                .unwrap()
                .unwrap();
            variants.insert(v);
        }
        assert!(
            variants.len() > 1,
            "20 different users should hit both variants"
        );
    }
}
