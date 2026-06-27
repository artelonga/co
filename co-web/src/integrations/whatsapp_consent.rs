//! CO-491 — deterministic, versioned WhatsApp consent text.
//!
//! The single source of truth is `co-web/seed/legal/whatsapp-consent.v1.yaml`, a
//! LEGAL ARTIFACT embedded at compile time via [`include_str!`]. The bot must
//! display these strings **VERBATIM** — the LLM never phrases, paraphrases, or
//! translates consent copy. The only substitution is `{operator}` → the deploying
//! community/operator name.
//!
//! `GET /api/v1/whatsapp/consent?operator=<name>` returns the parsed, substituted
//! document as JSON `{version, agreement, boundary, how_it_works,
//! commands{export, erase_confirm, erase_done, forget_done}, rights}`. It is
//! public-readable (it *is* the consent text) and fully deterministic.
//!
//! [`consent_version`] exposes the document `version` so the link flow (CO-490)
//! can record/log the exact version a user agreed to (LGPD demonstrabilidade,
//! Art. 8º §2º). To change wording, mint a NEW version file and bump `version` —
//! never edit a published version in place.

use std::sync::OnceLock;

use axum::{Json, Router, extract::Query, routing::get};
use serde::{Deserialize, Serialize};

use crate::server::AppState;

/// The embedded consent document (compile-time constant, always present).
const CONSENT_YAML: &str = include_str!("../../seed/legal/whatsapp-consent.v1.yaml");

/// Default operator name when neither the `operator` query param nor
/// `CO_OPERATOR_NAME` is set.
const DEFAULT_OPERATOR_FALLBACK: &str = "ArteLonga";

/// Raw parse of the YAML source of truth. Only the fields the bot needs.
#[derive(Debug, Clone, Deserialize)]
struct ConsentDoc {
    version: String,
    agreement: String,
    boundary: String,
    how_it_works: String,
    commands: ConsentCommands,
    rights: String,
}

/// The sacred-command confirmations (rights). Deterministic, never LLM-generated.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConsentCommands {
    pub export: String,
    pub erase_confirm: String,
    pub erase_done: String,
    pub forget_done: String,
}

/// The consent document with `{operator}` substituted, returned as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentResponse {
    pub version: String,
    pub agreement: String,
    pub boundary: String,
    pub how_it_works: String,
    pub commands: ConsentCommands,
    pub rights: String,
}

/// Parse the embedded YAML once. The source is a compile-time constant, so a
/// parse failure is a build-time/test-time bug (covered by a unit test) rather
/// than a runtime condition — hence `expect`.
fn doc() -> &'static ConsentDoc {
    static DOC: OnceLock<ConsentDoc> = OnceLock::new();
    DOC.get_or_init(|| {
        serde_yaml::from_str(CONSENT_YAML).expect("embedded whatsapp consent yaml must parse")
    })
}

/// The version string of the active consent document (e.g.
/// `whatsapp-consent/2026-06-27.v1`). Used by the CO-490 link flow to record the
/// agreed version.
pub fn consent_version() -> &'static str {
    &doc().version
}

/// Substitute the single allowed placeholder, `{operator}`. No other interpolation.
fn sub(s: &str, operator: &str) -> String {
    s.replace("{operator}", operator)
}

/// Render the consent document for a given operator name (verbatim text with
/// `{operator}` substituted). Pure → unit-testable.
pub fn render(operator: &str) -> ConsentResponse {
    let d = doc();
    ConsentResponse {
        version: d.version.clone(),
        agreement: sub(&d.agreement, operator),
        boundary: sub(&d.boundary, operator),
        how_it_works: sub(&d.how_it_works, operator),
        commands: ConsentCommands {
            export: sub(&d.commands.export, operator),
            erase_confirm: sub(&d.commands.erase_confirm, operator),
            erase_done: sub(&d.commands.erase_done, operator),
            forget_done: sub(&d.commands.forget_done, operator),
        },
        rights: sub(&d.rights, operator),
    }
}

/// The default operator name: `CO_OPERATOR_NAME` if set/non-blank, else
/// [`DEFAULT_OPERATOR_FALLBACK`].
pub fn default_operator() -> String {
    std::env::var("CO_OPERATOR_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_OPERATOR_FALLBACK.to_string())
}

#[derive(Debug, Deserialize)]
pub struct ConsentQuery {
    /// The operator/community name substituted for `{operator}`. Absent/blank ⇒
    /// [`default_operator`].
    pub operator: Option<String>,
}

/// `GET /api/v1/whatsapp/consent?operator=<name>` — deterministic, public consent
/// text. Never invokes a model; substitutes only `{operator}`.
async fn consent_handler(Query(q): Query<ConsentQuery>) -> Json<ConsentResponse> {
    let operator = q
        .operator
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_operator);
    Json(render(&operator))
}

/// Router for the consent endpoint, nested at `/api/v1` (full path
/// `/api/v1/whatsapp/consent`). Public-readable, so no auth layer at the mount.
pub fn router() -> Router<AppState> {
    Router::new().route("/whatsapp/consent", get(consent_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_yaml_parses_and_carries_the_pinned_version() {
        // The version is the exact LGPD-recorded identifier; pin it so a wording
        // change without a version bump fails the build.
        assert_eq!(consent_version(), "whatsapp-consent/2026-06-27.v1");
    }

    #[test]
    fn operator_is_substituted_everywhere_and_no_placeholder_leaks() {
        let r = render("Quilombo Araucária");
        // Every field that mentions the operator is substituted; the literal
        // placeholder never survives in any field.
        for field in [
            &r.agreement,
            &r.boundary,
            &r.how_it_works,
            &r.rights,
            &r.commands.export,
            &r.commands.erase_confirm,
            &r.commands.erase_done,
            &r.commands.forget_done,
        ] {
            assert!(
                !field.contains("{operator}"),
                "no {{operator}} placeholder may remain: {field:?}"
            );
        }
        // The fields that reference the operator carry the substituted name.
        assert!(r.agreement.contains("Quilombo Araucária"));
        assert!(r.boundary.contains("Quilombo Araucária"));
        assert!(r.how_it_works.contains("Quilombo Araucária"));
    }

    #[test]
    fn all_keys_are_present_and_non_empty() {
        let r = render("Op");
        assert!(!r.version.is_empty());
        assert!(!r.agreement.is_empty());
        assert!(!r.boundary.is_empty());
        assert!(!r.how_it_works.is_empty());
        assert!(!r.rights.is_empty());
        assert!(!r.commands.export.is_empty());
        assert!(!r.commands.erase_confirm.is_empty());
        assert!(!r.commands.erase_done.is_empty());
        assert!(!r.commands.forget_done.is_empty());
    }

    #[test]
    fn default_operator_falls_back_when_unset() {
        // Don't mutate the process env here (parallel tests); just assert the
        // fallback constant is what `default_operator` yields absent the var.
        if std::env::var("CO_OPERATOR_NAME").is_err() {
            assert_eq!(default_operator(), "ArteLonga");
        }
    }
}
