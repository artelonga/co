//! CO-388: `ReleaseBlocker` — logs and alerts on Critical/High security findings.
//!
//! Subscribes to `security.finding_detected` events. When severity is High or
//! Critical, logs an error-level alert and publishes a `security.release_blocked`
//! event so the release gate can detect unresolved blockers.
//!
//! The actual GitHub PR status check is set by the CI workflow step (pr-route.yml
//! step 11). This subscriber handles the server-side alerting and audit trail.

use async_trait::async_trait;
use tracing::error;

use crate::eda::bus::Filter;
use crate::eda::event::{Event, Visibility};
use crate::eda::subscriber_registry::{EdaSubscriber, SubscriberCtx};
use crate::security::audit::Severity;

/// CO-435: logs + publishes `security.release_blocked` for Critical/High findings.
pub struct ReleaseBlocker;

#[async_trait]
impl EdaSubscriber for ReleaseBlocker {
    fn name(&self) -> &'static str {
        "ReleaseBlocker"
    }

    fn filter(&self) -> Filter {
        Filter {
            event_types: Some(vec!["security.finding_detected".into()]),
            ..Default::default()
        }
    }

    async fn handle(&self, ev: &Event, ctx: &SubscriberCtx) {
        let p = &ev.payload;
        let severity_str = p["severity"].as_str().unwrap_or("info");
        let severity = Severity::parse(severity_str);

        if !severity.blocks_merge() {
            return;
        }

        let finding_id = p["id"].as_str().unwrap_or("").to_string();
        let pr_number = p["pr_number"].as_i64().unwrap_or(0);
        let description = p["description"].as_str().unwrap_or("").to_string();
        let category = p["category"].as_str().unwrap_or("other").to_string();

        error!(
            "security: RELEASE BLOCKED — {} finding {} in PR #{} ({}): {}",
            severity_str.to_uppercase(),
            finding_id,
            pr_number,
            category,
            description,
        );

        // Publish the block event — visible in /agora for admins (System visibility).
        ctx.bus.publish(Event::new(
            "security.release_blocked",
            None,
            None,
            serde_json::json!({
                "finding_id": finding_id,
                "pr_number": pr_number,
                "severity": severity_str,
                "category": category,
                "description": description,
                "icon": "🚨",
            }),
            Visibility::System,
        ));
    }
}
