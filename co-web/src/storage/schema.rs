//! Free-standing schema helpers, row mappers, seed data, and unit tests.

use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::entry_index::EntryRow;
use crate::models::*;

use super::Storage;

#[cfg(test)]
use super::{
    SEED_DADOS_RASTREADOS_MD, SEED_LINHAS_DO_TEMPO_MD, SEED_PRIVACIDADE_MD, SEED_SOBRE_MD,
    SEED_TEMPLATE_INDEX_MD, SEED_TERMOS_MD,
};

/// Split a `.md` file with YAML frontmatter into `(frontmatter_yaml, body)`.
/// If no frontmatter is present, returns `("", whole_input)`.
pub(crate) fn split_frontmatter(md: &str) -> (&str, &str) {
    let s = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"));
    let Some(rest) = s else { return ("", md) };
    if let Some(end) = rest.find("\n---\n") {
        return (&rest[..end], rest[end + 5..].trim_start_matches('\n'));
    }
    if let Some(end) = rest.find("\r\n---\r\n") {
        return (
            &rest[..end],
            rest[end + 7..].trim_start_matches(['\n', '\r']),
        );
    }
    ("", md)
}

/// Convert the YAML frontmatter of a seed page into a `serde_json::Value` and
/// stamp `created`/`modified` to the supplied timestamp (so seeds always show
/// "now" rather than the file's original creation date).
pub(crate) fn seed_page_frontmatter(md: &str, now_str: &str) -> serde_json::Value {
    let (fm_yaml, _) = split_frontmatter(md);
    let mut fm: serde_json::Value = serde_yaml::from_str::<serde_yaml::Value>(fm_yaml)
        .ok()
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = fm.as_object_mut() {
        obj.insert("created".into(), json!(now_str));
        obj.insert("modified".into(), json!(now_str));
    }
    fm
}

pub(crate) fn seed_page_body(md: &str) -> &str {
    let (_, body) = split_frontmatter(md);
    body
}

/// Recursively collect file paths under `dir`. Returns absolute PathBufs in
/// dir-order. Caller filters by extension. No symlink-following; ignores
/// errors (skipped silently).
pub(crate) fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// Idempotent ALTER TABLE ADD COLUMN: checks `pragma_table_info` before issuing
/// the DDL so repeated calls (and partially-applied migrations) are safe.
/// Returns `true` if the column was added, `false` if it already existed.
/// CO-137: replaces bare `ALTER TABLE … ADD COLUMN` in migrations v17–v22 to
/// prevent "duplicate column name" panics on re-run after partial application.
pub(crate) fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> rusqlite::Result<bool> {
    let exists: bool = conn
        .query_row(
            &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"),
            params![column],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {column_def};"
        ))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Idempotent CREATE TABLE: queries `sqlite_master` before issuing the DDL.
/// Returns `true` if the table was created, `false` if it already existed.
///
/// Sibling of `ensure_column`. Surfaced after the third partial-apply incident
/// (CO-77 entries, CO-137 parent_key, CO-121 feature_flags). The standalone
/// `CREATE TABLE IF NOT EXISTS` SQL is already idempotent, so this helper
/// exists primarily to give callers a single, consistent surface for migrations
/// and to make it trivial to add observability (e.g. tracing) at the call site.
pub(crate) fn ensure_table(
    conn: &Connection,
    name: &str,
    body_sql: &str,
) -> rusqlite::Result<bool> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(body_sql)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// CO-165: Row-mapping helpers for recovery tables
// ---------------------------------------------------------------------------

pub(crate) fn row_to_recovery_channel(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::models::RecoveryChannel> {
    let nonce_bytes: Vec<u8> = row.get(4)?;
    let mut nonce = [0u8; 12];
    if nonce_bytes.len() == 12 {
        nonce.copy_from_slice(&nonce_bytes);
    }
    Ok(crate::models::RecoveryChannel {
        id: row.get(0)?,
        user_id: row.get(1)?,
        channel_type: row.get(2)?,
        value_ciphertext: row.get(3)?,
        value_nonce: nonce,
        value_lookup_hash: row.get(5)?,
        verified_at: row.get(6)?,
        created_at: row.get(7)?,
        last_used_at: row.get(8)?,
        lockout_until: row.get(9)?,
    })
}

pub(crate) fn row_to_recovery_verification(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::models::RecoveryVerification> {
    Ok(crate::models::RecoveryVerification {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        user_id: row.get(2)?,
        purpose: row.get(3)?,
        code_hash: row.get(4)?,
        expires_at: row.get(5)?,
        consumed_at: row.get(6)?,
        attempts: row.get(7)?,
        created_at: row.get(8)?,
    })
}

// ---------------------------------------------------------------------------
// SQL helper — upsert a single entry into the entries table
// ---------------------------------------------------------------------------

pub(crate) fn upsert_entry_row(
    conn: &Connection,
    universe_key: &str,
    entry: &co::entry::Entry,
) -> anyhow::Result<()> {
    let fm_json = serde_json::to_string(&entry.frontmatter)?;
    let title: Option<&str> = entry.frontmatter.get("title").and_then(|v| v.as_str());
    let created_at = entry
        .frontmatter
        .get("created")
        .and_then(|v| v.as_str())
        .map(String::from);
    let updated_at = entry
        .frontmatter
        .get("modified")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| created_at.clone());

    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(universe_key, path) DO UPDATE SET
           entry_type = excluded.entry_type,
           title = excluded.title,
           frontmatter_json = excluded.frontmatter_json,
           body = excluded.body,
           body_hash = excluded.body_hash,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at",
        params![
            entry.path,
            universe_key,
            entry.entry_type,
            title,
            fm_json,
            entry.body,
            entry.body_hash,
            created_at,
            updated_at,
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

pub(crate) fn entry_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let fm_str: String = row.get(4)?;
    let frontmatter: serde_json::Value =
        serde_json::from_str(&fm_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(EntryRow {
        path: row.get(0)?,
        universe_key: row.get(1)?,
        entry_type: row.get(2)?,
        title: row.get(3)?,
        frontmatter,
        body: row.get(5)?,
        body_hash: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        _score: None,
    })
}

pub(crate) fn entry_row_to_project(row: &EntryRow) -> Option<Project> {
    let fm = &row.frontmatter;
    let key = fm.get("key").and_then(|v| v.as_str())?.to_string();
    let name = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let next_id = fm.get("next_id").and_then(|v| v.as_u64()).unwrap_or(1);
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(Project {
        key,
        name,
        description: row.body.clone(),
        created_at,
        next_id,
        archived,
    })
}

pub(crate) fn entry_row_to_task(row: &EntryRow) -> Option<Task> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let project_key = fm
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = parse_status(fm.get("status").and_then(|v| v.as_str()).unwrap_or("todo"));
    let priority = parse_priority(
        fm.get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium"),
    );
    let due_date = fm
        .get("due")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<NaiveDate>().ok());
    let parent = fm.get("parent").and_then(|v| v.as_u64());
    let labels: Vec<String> = fm
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);
    let updated_at = fm
        .get("modified")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or(created_at);
    let archived = fm
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let assignee = fm
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(Task {
        id,
        key: format!("{}-{}", project_key, id),
        project_key,
        title,
        status,
        priority,
        due_date,
        parent,
        labels,
        created_at,
        updated_at,
        description: row.body.clone(),
        archived,
        assignee,
    })
}

pub(crate) fn entry_row_to_comment(
    row: &EntryRow,
    project_key: &str,
    task_id: u64,
) -> Option<Comment> {
    let fm = &row.frontmatter;
    let id = fm.get("id").and_then(|v| v.as_u64())?;
    let author = fm
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("Anonymous")
        .to_string();
    let created_at = fm
        .get("created")
        .and_then(|v| v.as_str())
        .map(parse_datetime)
        .unwrap_or_else(Utc::now);

    Some(Comment {
        id,
        project_key: project_key.to_string(),
        task_id,
        author,
        body: row.body.clone(),
        created_at,
    })
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

pub fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub(crate) fn parse_status(s: &str) -> TaskStatus {
    match s {
        "todo" => TaskStatus::Todo,
        "in_progress" => TaskStatus::InProgress,
        "in_review" => TaskStatus::InReview,
        "done" => TaskStatus::Done,
        _ => TaskStatus::Todo,
    }
}

pub(crate) fn parse_priority(s: &str) -> Priority {
    match s {
        "low" => Priority::Low,
        "medium" => Priority::Medium,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        _ => Priority::Medium,
    }
}

// --- Seed Data ---

pub fn seed_data(storage: &mut Storage) {
    use chrono::NaiveDate;

    let ds = CreateProject {
        name: "Design System".into(),
        key: "DS".into(),
        description: "Shared component library and design tokens".into(),
        ..Default::default()
    };
    storage.create_project(ds).unwrap();

    let api = CreateProject {
        name: "Backend API".into(),
        key: "API".into(),
        description: "Core REST API and data services".into(),
        ..Default::default()
    };
    storage.create_project(api).unwrap();

    // --- Design System tasks ---
    let ds_tasks = vec![
        CreateTask {
            title: "Define visual identity".into(),
            description: "Create logo, color palette, and typography for the design system.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 1),
            parent: None,
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Build component showcase".into(),
            description: "Develop a web-based showcase of all available components and patterns."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["web".into(), "design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Organize first design review".into(),
            description:
                "Schedule review session, prepare demos, and gather feedback from stakeholders."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 20),
            parent: None,
            labels: vec!["review".into()],
            assignee: None,
        },
        CreateTask {
            title: "Produce component catalog".into(),
            description:
                "Document each component with usage examples, props, and accessibility notes."
                    .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Set up documentation site".into(),
            description: "Deploy a static site with guidelines and a monthly content calendar."
                .into(),
            status: TaskStatus::Done,
            priority: Priority::Low,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 10),
            parent: None,
            labels: vec!["marketing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Select color palette".into(),
            description: "Define primary and secondary colors aligned with the project identity."
                .into(),
            status: TaskStatus::InReview,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 25),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
        CreateTask {
            title: "Design logo".into(),
            description: "Create 3 logo proposals for team vote.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 28),
            parent: Some(1),
            labels: vec!["design".into()],
            assignee: None,
        },
    ];

    for task in ds_tasks {
        storage.create_task("ds", task).unwrap();
    }

    // --- Backend API tasks ---
    let api_tasks = vec![
        CreateTask {
            title: "Database schema design".into(),
            description: "Design and document the relational schema for all core entities.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 30),
            parent: None,
            labels: vec!["database".into(), "urgent".into()],
            assignee: None,
        },
        CreateTask {
            title: "API documentation".into(),
            description: "Write OpenAPI specs and usage guides for every endpoint.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: None,
            labels: vec!["docs".into()],
            assignee: None,
        },
        CreateTask {
            title: "Authentication module".into(),
            description: "Implement JWT-based auth with refresh tokens and role-based access."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 1),
            parent: None,
            labels: vec!["security".into(), "auth".into()],
            assignee: None,
        },
        CreateTask {
            title: "Rate limiting and throttling".into(),
            description: "Add per-endpoint rate limits and IP-based throttling to protect the API."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 7, 1),
            parent: None,
            labels: vec!["security".into()],
            assignee: None,
        },
        CreateTask {
            title: "CI/CD pipeline setup".into(),
            description:
                "Configure automated testing, linting, and deployment for the API service.".into(),
            status: TaskStatus::InReview,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 15),
            parent: None,
            labels: vec!["devops".into()],
            assignee: None,
        },
        CreateTask {
            title: "Write migration scripts".into(),
            description: "Create versioned SQL migrations for the initial schema.".into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 4, 10),
            parent: Some(1),
            labels: vec!["database".into()],
            assignee: None,
        },
        CreateTask {
            title: "Integration test suite".into(),
            description: "Build end-to-end tests covering all critical API workflows.".into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 1),
            parent: Some(3),
            labels: vec!["testing".into()],
            assignee: None,
        },
        CreateTask {
            title: "Load testing workshop".into(),
            description:
                "Run load tests to identify bottlenecks and establish performance baselines.".into(),
            status: TaskStatus::Done,
            priority: Priority::Medium,
            due_date: NaiveDate::from_ymd_opt(2026, 3, 8),
            parent: Some(1),
            labels: vec!["testing".into(), "performance".into()],
            assignee: None,
        },
    ];

    for task in api_tasks {
        storage.create_task("api", task).unwrap();
    }

    // --- Platform ---
    let plt = CreateProject {
        name: "Platform".into(),
        key: "PLT".into(),
        description: "Unified platform for management and collaboration".into(),
        ..Default::default()
    };
    storage.create_project(plt).unwrap();

    let plt_tasks = vec![
        CreateTask {
            title: "Initial Launch".into(),
            description: "Launch epic: prepare and publish the first versions of the product."
                .into(),
            status: TaskStatus::InProgress,
            priority: Priority::Critical,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: None,
            labels: vec!["epic".into(), "launch".into()],
            assignee: None,
        },
        CreateTask {
            title: "Internal MVP".into(),
            description: "Minimum viable version for internal team use. Validate core \
                           workflows, identify critical bugs, and collect feedback before \
                           the public launch."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 5, 15),
            parent: Some(1),
            labels: vec!["mvp".into()],
            assignee: None,
        },
        CreateTask {
            title: "Public MVP".into(),
            description: "First public version of the product. Incorporate fixes from the \
                           internal MVP, prepare onboarding, documentation, and production \
                           infrastructure."
                .into(),
            status: TaskStatus::Todo,
            priority: Priority::High,
            due_date: NaiveDate::from_ymd_opt(2026, 6, 30),
            parent: Some(1),
            labels: vec!["mvp".into(), "public".into()],
            assignee: None,
        },
    ];

    for task in plt_tasks {
        storage.create_task("plt", task).unwrap();
    }
}

#[cfg(test)]
mod seed_md_tests {
    use super::*;

    #[test]
    fn split_frontmatter_extracts_yaml_and_body() {
        let md = "---
slug: foo
title: Bar
order: 3
---

# Heading

Body text.
";
        let (fm, body) = split_frontmatter(md);
        assert!(fm.contains("slug: foo"));
        assert!(fm.contains("title: Bar"));
        assert!(body.starts_with("# Heading"));
        assert!(body.contains("Body text."));
    }

    #[test]
    fn split_frontmatter_handles_no_frontmatter() {
        let md = "# Just markdown

No frontmatter.";
        let (fm, body) = split_frontmatter(md);
        assert_eq!(fm, "");
        assert_eq!(body, md);
    }

    #[test]
    fn seed_page_frontmatter_overrides_timestamps() {
        let md = "---
slug: x
title: T
order: 1
tags:
  - a
created: 2020-01-01T00:00:00Z
modified: 2020-01-01T00:00:00Z
---

body";
        let now = "2026-04-26T00:00:00+00:00";
        let fm = seed_page_frontmatter(md, now);
        assert_eq!(fm.get("slug").and_then(|v| v.as_str()), Some("x"));
        assert_eq!(fm.get("title").and_then(|v| v.as_str()), Some("T"));
        assert_eq!(fm.get("created").and_then(|v| v.as_str()), Some(now));
        assert_eq!(fm.get("modified").and_then(|v| v.as_str()), Some(now));
    }

    #[test]
    fn embedded_seed_md_files_parse() {
        for md in [
            SEED_TEMPLATE_INDEX_MD,
            SEED_SOBRE_MD,
            SEED_TERMOS_MD,
            SEED_PRIVACIDADE_MD,
            SEED_DADOS_RASTREADOS_MD,
            SEED_LINHAS_DO_TEMPO_MD,
        ] {
            let now = "2026-04-26T00:00:00+00:00";
            let fm = seed_page_frontmatter(md, now);
            assert!(
                fm.get("slug").and_then(|v| v.as_str()).is_some(),
                "missing slug"
            );
            assert!(
                fm.get("title").and_then(|v| v.as_str()).is_some(),
                "missing title"
            );
            let body = seed_page_body(md);
            assert!(
                body.starts_with("# "),
                "body should start with H1, got: {:?}",
                &body[..40.min(body.len())]
            );
        }
    }
}

#[cfg(test)]
mod ensure_column_tests {
    use rusqlite::Connection;

    use super::ensure_column;

    fn make_test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY);")
            .expect("create test table");
        conn
    }

    #[test]
    fn adds_missing_column() {
        let conn = make_test_conn();
        let added = ensure_column(&conn, "t", "foo", "TEXT").expect("ensure_column");
        assert!(added, "should report column was added");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('t') WHERE name = 'foo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "column should exist after ensure_column");
    }

    #[test]
    fn no_op_if_column_exists() {
        let conn = make_test_conn();
        ensure_column(&conn, "t", "foo", "TEXT").expect("first call");
        let added = ensure_column(&conn, "t", "foo", "TEXT").expect("second call");
        assert!(!added, "should report no-op when column already exists");
    }

    #[test]
    fn idempotent_repeated_calls() {
        let conn = make_test_conn();
        for i in 0..5 {
            let added =
                ensure_column(&conn, "t", "bar", "INTEGER DEFAULT 0").expect("idempotent call");
            assert_eq!(added, i == 0, "only first call should add the column");
        }
    }

    #[test]
    fn partial_migration_recovery() {
        // Simulates the CO-137 scenario: schema_version shows v22 was applied
        // but the ALTER TABLE never ran. ensure_column should recover cleanly.
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE universes (key TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (22);",
        )
        .expect("setup");

        // Column doesn't exist yet (partial migration state)
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('universes') WHERE name = 'parent_key'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "precondition: parent_key should be missing");

        // ensure_column adds it without panic
        let added = ensure_column(&conn, "universes", "parent_key", "TEXT")
            .expect("ensure_column on partial migration");
        assert!(added, "should have added the missing column");

        // Verify it's now present
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('universes') WHERE name = 'parent_key'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "parent_key should exist after recovery");
    }
}
