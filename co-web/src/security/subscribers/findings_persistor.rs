//! CO-388: `FindingsPersistor` — writes security findings to the DB.
//!
//! Subscribes to `security.finding_detected` events on the EDA bus and
//! INSERT-or-IGNOREs each finding into the `security_findings` table.

use async_trait::async_trait;
use tracing::warn;

use crate::eda::bus::Filter;
use crate::eda::event::Event;
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};

/// CO-435: writes `security.finding_detected` events to the `security_findings` table.
pub struct FindingsPersistor;

#[async_trait]
impl EdaSubscriber for FindingsPersistor {
    fn name(&self) -> &'static str {
        "FindingsPersistor"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["security.finding_detected".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
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

        let storage = ctx.storage.lock();
        if let Err(e) = storage.insert_security_finding(
            &id,
            pr_number,
            &severity,
            &category,
            &file_path,
            line_start,
            line_end,
            &description,
            cwe.as_deref(),
            cve_match.as_deref(),
            suggested_patch.as_deref(),
            &detected_at,
        ) {
            warn!("EDA: FindingsPersistor INSERT failed: {e}");
        }
    }
}
