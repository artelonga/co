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
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::eda::event::{Event, Visibility};
use crate::security::audit::Severity;
use crate::storage::Storage;

pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let eda_bus = bus.clone();
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["security.finding_detected".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: PBIBacklogger started");
        while let Some(ev) = sub.recv().await {
            let p = &ev.payload;
            let severity_str = p["severity"].as_str().unwrap_or("info");
            let severity = Severity::parse(severity_str);

            if !severity.creates_pbi() {
                continue;
            }

            let finding_id = p["id"].as_str().unwrap_or("").to_string();
            let pr_number = p["pr_number"].as_i64().unwrap_or(0);
            let category = p["category"].as_str().unwrap_or("other").to_string();
            let file_path = p["file_path"].as_str().unwrap_or("").to_string();
            let description = p["description"].as_str().unwrap_or("").to_string();
            let cwe = p["cwe"].as_str().unwrap_or("").to_string();

            let pbi_path = format!("work/co/security/SEC-{finding_id}.md");
            let pbi_body = format!(
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
                priority = if severity.blocks_merge() {
                    "critical"
                } else {
                    "high"
                },
            );

            // Write the PBI entry to the `co` universe via the DB.
            let universe_key = "co";
            let entry_id = crate::eda::event::new_ulid();
            let now = chrono::Utc::now().to_rfc3339();

            let storage = storage.lock();
            // Try to insert the PBI as a universe entry — graceful if co universe absent.
            let insert_result = storage.conn().execute(
                "INSERT OR IGNORE INTO entries \
                 (id, universe_key, path, entry_type, title, body, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'pbi', ?4, ?5, ?6, ?6)",
                params![
                    entry_id,
                    universe_key,
                    pbi_path,
                    format!("Security: {description}"),
                    pbi_body,
                    now,
                ],
            );
            drop(storage);

            match insert_result {
                Ok(rows) if rows > 0 => {
                    info!(
                        "security: created PBI {pbi_path} for {severity_str} finding {finding_id}"
                    );
                    // Publish a creation event so the sprint board picks it up.
                    eda_bus.publish(Event::new(
                        "entry.created",
                        Some(universe_key.to_string()),
                        None,
                        serde_json::json!({
                            "path": pbi_path,
                            "entry_type": "pbi",
                            "source": "security-audit",
                            "finding_id": finding_id,
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
