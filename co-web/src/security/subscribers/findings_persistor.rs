//! CO-388: `FindingsPersistor` — writes security findings to the DB.
//!
//! Subscribes to `security.finding_detected` events on the EDA bus and
//! INSERT-or-IGNOREs each finding into the `security_findings` table.

use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::params;
use tracing::{info, warn};

use crate::eda::bus::{EdaBus, Filter};
use crate::storage::Storage;

pub fn spawn(bus: Arc<dyn EdaBus>, storage: Arc<Mutex<Storage>>) {
    let mut sub = bus.subscribe(Filter {
        event_types: Some(vec!["security.finding_detected".into()]),
        ..Default::default()
    });

    tokio::spawn(async move {
        info!("EDA: FindingsPersistor started");
        while let Some(ev) = sub.recv().await {
            let p = &ev.payload;

            let id = p["id"].as_str().unwrap_or("").to_string();
            let pr_number = p["pr_number"].as_i64().unwrap_or(0);
            let severity = p["severity"].as_str().unwrap_or("info").to_string();
            let category = p["category"].as_str().unwrap_or("other").to_string();
            let file_path = p["file_path"].as_str().unwrap_or("").to_string();
            let line_start = p["line_start"].as_i64();
            let line_end = p["line_end"].as_i64();
            let description = p["description"].as_str().unwrap_or("").to_string();
            let cwe = p["cwe"].as_str().map(str::to_string);
            let cve_match = p["cve_match"].as_str().map(str::to_string);
            let suggested_patch = p["suggested_patch"].as_str().map(str::to_string);
            let detected_at = ev.created_at.to_rfc3339();

            let storage = storage.lock();
            if let Err(e) = storage.conn().execute(
                "INSERT OR IGNORE INTO security_findings \
                 (id, pr_number, severity, category, file_path, line_start, line_end, \
                  description, cwe, cve_match, suggested_patch, detected_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    pr_number,
                    severity,
                    category,
                    file_path,
                    line_start,
                    line_end,
                    description,
                    cwe,
                    cve_match,
                    suggested_patch,
                    detected_at,
                ],
            ) {
                warn!("EDA: FindingsPersistor INSERT failed: {e}");
            }
        }
        info!("EDA: FindingsPersistor stopped (bus closed)");
    });
}
