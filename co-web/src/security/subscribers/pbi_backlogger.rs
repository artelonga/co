//! CO-388: `PBIBacklogger` — creates sprint PBIs from Medium+ security findings.
//!
//! Subscribes to `security.finding_detected` events. When severity is Medium,
//! High, or Critical, creates a PBI entry in the CO universe so Yuri sees it
//! on the sprint board alongside feature work.
//!
//! PBI path: `work/co/security/SEC-<finding_id>.md`
//! PBI type: `pbi` with frontmatter derived from the finding.

use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::params;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::{Event, Visibility};
use crate::security::audit::Severity;
use crate::storage::Storage;

/// The CO universe key into which security PBIs are written.
const PBI_UNIVERSE_KEY: &str = "co";

/// Fields extracted from a `security.finding_detected` event payload.
struct PbiFields {
    finding_id: String,
    pr_number: i64,
    category: String,
    file_path: String,
    description: String,
    cwe: String,
    severity: Severity,
    severity_str: String,
}

impl PbiFields {
    fn from_payload(p: &serde_json::Value) -> Self {
        let severity_str = p["severity"].as_str().unwrap_or("info").to_string();
        Self {
            finding_id: p["id"].as_str().unwrap_or("").to_string(),
            pr_number: p["pr_number"].as_i64().unwrap_or(0),
            category: p["category"].as_str().unwrap_or("other").to_string(),
            file_path: p["file_path"].as_str().unwrap_or("").to_string(),
            description: p["description"].as_str().unwrap_or("").to_string(),
            cwe: p["cwe"].as_str().unwrap_or("").to_string(),
            severity: Severity::parse(&severity_str),
            severity_str,
        }
    }

    fn priority(&self) -> &'static str {
        if self.severity.blocks_merge() {
            "critical"
        } else {
            "high"
        }
    }

    fn pbi_path(&self) -> String {
        format!("work/co/security/SEC-{}.md", self.finding_id)
    }

    fn title(&self) -> String {
        format!("Security: {}", self.description)
    }

    fn body(&self) -> String {
        format!(
            "---\n\
             type: pbi\n\
             title: \"Security: {description}\"\n\
             status: todo\n\
             priority: {priority}\n\
             source: security-audit\n\
             finding_id: {finding_id}\n\
             finding_severity: {severity_str}\n\
             finding_category: {category}\n\
             finding_file: {file_path}\n\
             source_pr: {pr_number}\n\
             cwe: \"{cwe}\"\n\
             dod: \"patched + verified no regression\"\n\
             ---\n\n\
             ## Security Finding — {severity_str} severity\n\n\
             **Category**: {category}  \n\
             **File**: `{file_path}`  \n\
             **Source PR**: #{pr_number}  \n\
             **CWE**: {cwe}  \n\n\
             {description}\n\n\
             ## Acceptance\n\n\
             - [ ] Finding confirmed as genuine vulnerability\n\
             - [ ] Fix applied and passes all tests\n\
             - [ ] No regression in related functionality\n\
             - [ ] Security test added to prevent recurrence\n",
            description = self.description,
            priority = self.priority(),
            finding_id = self.finding_id,
            severity_str = self.severity_str,
            category = self.category,
            file_path = self.file_path,
            pr_number = self.pr_number,
            cwe = self.cwe,
        )
    }

    fn frontmatter_json(&self) -> String {
        serde_json::json!({
            "type": "pbi",
            "title": self.title(),
            "status": "todo",
            "priority": self.priority(),
            "source": "security-audit",
            "finding_id": self.finding_id,
            "finding_severity": self.severity_str,
            "finding_category": self.category,
            "finding_file": self.file_path,
            "source_pr": self.pr_number,
            "cwe": self.cwe,
            "dod": "patched + verified no regression",
        })
        .to_string()
    }
}

/// Insert the PBI row for `fields` into the `entries` table. Returns the number
/// of rows inserted (0 if it already existed).
///
/// CO-388 (security hardening): the `entries` table is keyed by
/// (universe_key, path) and has NO `id` column — the previous INSERT referenced
/// a nonexistent `entries.id`, so every PBI INSERT failed and the error was
/// swallowed by a `warn!` (PBIs were never created). This matches the real
/// schema: it provides `frontmatter_json` and the NOT NULL `body_hash`.
fn insert_pbi(storage: &Mutex<Storage>, fields: &PbiFields) -> rusqlite::Result<usize> {
    let now = chrono::Utc::now().to_rfc3339();
    let pbi_body = fields.body();
    let body_hash = format!("{:x}", Sha256::digest(pbi_body.as_bytes()));

    let storage = storage.lock();
    storage.conn().execute(
        "INSERT OR IGNORE INTO entries \
         (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
          created_at, updated_at) \
         VALUES (?1, ?2, 'pbi', ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            fields.pbi_path(),
            PBI_UNIVERSE_KEY,
            fields.title(),
            fields.frontmatter_json(),
            pbi_body,
            body_hash,
            now,
        ],
    )
}

pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let eda_bus = bus.clone();
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["security.finding_detected".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: PBIBacklogger started");
        while let Some(ev) = sub.recv().await {
            let fields = PbiFields::from_payload(&ev.payload);

            if !fields.severity.creates_pbi() {
                continue;
            }

            match insert_pbi(&storage, &fields) {
                Ok(rows) if rows > 0 => {
                    info!(
                        "security: created PBI {} for {} finding {}",
                        fields.pbi_path(),
                        fields.severity_str,
                        fields.finding_id
                    );
                    // Publish a creation event so the sprint board picks it up.
                    eda_bus.publish(Event::new(
                        "entry.created",
                        Some(PBI_UNIVERSE_KEY.to_string()),
                        None,
                        serde_json::json!({
                            "path": fields.pbi_path(),
                            "entry_type": "pbi",
                            "source": "security-audit",
                            "finding_id": fields.finding_id,
                        }),
                        Visibility::System,
                    ));
                }
                Ok(_) => {
                    // INSERT OR IGNORE — already exists; skip.
                }
                Err(e) => {
                    warn!("security: PBIBacklogger INSERT failed (entries table may differ): {e}");
                }
            }
        }
        info!("EDA: PBIBacklogger stopped (bus closed)");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    fn finding_payload(severity: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "01HZSEC0000000000000000001",
            "pr_number": 42,
            "severity": severity,
            "category": "sql_injection",
            "file_path": "co-web/src/foo.rs",
            "description": "SQL injection via açúcar 🚨 interpolation",
            "cwe": "CWE-89",
        })
    }

    #[test]
    fn high_severity_finding_creates_pbi_row() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Mutex::new(Storage::new(dir.path()));

        let fields = PbiFields::from_payload(&finding_payload("high"));
        let rows = insert_pbi(&storage, &fields).expect("insert must succeed against real schema");
        assert_eq!(
            rows, 1,
            "a high-severity finding must insert exactly one PBI"
        );

        // Verify the row is actually present and well-formed.
        let st = storage.lock();
        let (etype, title, body_hash): (String, String, String) = st
            .conn()
            .query_row(
                "SELECT entry_type, title, body_hash FROM entries \
                 WHERE universe_key = 'co' AND path = ?1",
                params![fields.pbi_path()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("PBI row must exist");
        assert_eq!(etype, "pbi");
        assert!(title.starts_with("Security:"));
        assert!(
            !body_hash.is_empty(),
            "body_hash (NOT NULL) must be populated"
        );
    }

    #[test]
    fn duplicate_finding_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Mutex::new(Storage::new(dir.path()));
        let fields = PbiFields::from_payload(&finding_payload("critical"));
        assert_eq!(insert_pbi(&storage, &fields).unwrap(), 1);
        // Second insert for the same finding id → INSERT OR IGNORE → 0 rows.
        assert_eq!(insert_pbi(&storage, &fields).unwrap(), 0);
    }
}
