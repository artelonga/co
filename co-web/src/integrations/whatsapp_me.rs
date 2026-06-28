//! CO-492 — LGPD data-rights endpoints for the WhatsApp companion (Art. 18).
//!
//! Three caller-scoped endpoints let a user exercise their LGPD rights over the
//! data the bot curates for them. Every endpoint binds to the AUTHENTICATED
//! CALLER (`Scoped<EntriesRead>`/`Scoped<EntriesWrite>`, CO-448) — exactly like
//! the CO-499 lead-digest fix — so a user can only ever touch **their own** data;
//! the resolved `user_id` is the only identity any of these handlers operate on.
//!
//! - `GET  /api/v1/whatsapp/me/export` — Art. 18 V (portabilidade): the caller's
//!   curated content (entries in the universes the caller OWNS, where the bot
//!   writes on their behalf) + their consent record + profile, as a portable
//!   JSON bundle that also carries a markdown rendering.
//! - `POST /api/v1/whatsapp/me/erase`  — Art. 18 VI (eliminação): erase the
//!   caller's curated content. REQUIRES an explicit confirmation token in the
//!   body — there is no single-call destructive erase.
//! - `POST /api/v1/whatsapp/me/forget` — Art. 18 + revogação do consentimento:
//!   erase curated content AND clear the WhatsApp link (`users.whatsapp*`) + the
//!   consent record, returning `clear_local_identity: true` so the bot also wipes
//!   its local identity store. Idempotent.
//!
//! ## DESTRUCTIVE-OPS RULE (the sacred boundary)
//!
//! `erase`/`forget` are never automatic and never one-step: each requires an
//! explicit, deterministic confirmation phrase in the body, mirroring the
//! consent YAML (`commands.erase_confirm` → "apagar tudo"; the rights line →
//! "esquece de mim"). The bot writes/erases the user's OWN data only on explicit
//! request; it never sends or publishes on their behalf.
//!
//! ## What "curated content" means (no new migration, existing tables)
//!
//! Entries carry no per-row creator column, and the bot acts AS the user (CO-490
//! token) writing into the universes that user OWNS. So the caller's curated
//! content is deterministically defined as **every entry in the universes the
//! caller owns** (`Storage::list_owned_universes`). Export reads those rows;
//! erase/forget remove them via [`SqliteEntryRepository::unindex_entry`] (entries
//! row + every projection: dates, relations, references, embeddings) — clearing
//! existing rows, never adding a schema/migration.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::auth::extractors::{EntriesRead, EntriesWrite, Scoped};
use crate::platform::error::AppError;
use crate::repository::SqliteEntryRepository;
use crate::server::AppState;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Deterministic confirmation phrases (mirror the consent YAML — never LLM copy)
// ---------------------------------------------------------------------------

/// The phrase the user must echo to confirm a destructive erase. Mirrors
/// `commands.erase_confirm` in `whatsapp-consent.v1.yaml` ("Responde 'apagar
/// tudo' pra confirmar").
const ERASE_CONFIRM_PHRASE: &str = "apagar tudo";

/// The phrase the user must echo to confirm a full forget. Mirrors the `rights`
/// line in `whatsapp-consent.v1.yaml` ("me fazer esquecer como entrar ('esquece
/// de mim')").
const FORGET_CONFIRM_PHRASE: &str = "esquece de mim";

// ---------------------------------------------------------------------------
// Output DTOs
// ---------------------------------------------------------------------------

/// The caller's profile (no secrets — never the password hash).
#[derive(Debug, Clone, Serialize)]
pub struct ProfileDto {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub tier: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usuario: Option<String>,
}

/// The recorded WhatsApp consent (CO-491), when present.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentDto {
    pub version: String,
    pub at: String,
    pub sha: String,
}

/// Per-owned-universe summary in the export.
#[derive(Debug, Clone, Serialize)]
pub struct UniverseSummary {
    pub key: String,
    pub entry_count: usize,
}

/// One exported entry — the full portable content of a single entry.
#[derive(Debug, Clone, Serialize)]
pub struct ExportEntry {
    pub universe_key: String,
    pub path: String,
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub frontmatter: serde_json::Value,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// The portable export bundle (Art. 18 V). JSON, with a `markdown` rendering of
/// the same data for human-portable hand-off.
#[derive(Debug, Clone, Serialize)]
pub struct ExportBundle {
    pub user_id: String,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileDto>,
    /// The linked WhatsApp number (digits), when linked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent: Option<ConsentDto>,
    pub universes: Vec<UniverseSummary>,
    pub entries: Vec<ExportEntry>,
    /// A portable markdown rendering of the whole bundle.
    pub markdown: String,
}

/// Summary returned by an erase (Art. 18 VI).
#[derive(Debug, Clone, Serialize)]
pub struct EraseSummary {
    pub erased: bool,
    pub entries_erased: usize,
    pub universes_touched: usize,
}

/// Summary returned by a forget (Art. 18 + revogação). `clear_local_identity`
/// is the signal telling the bot to also wipe its local identity store.
#[derive(Debug, Clone, Serialize)]
pub struct ForgetSummary {
    pub forgotten: bool,
    pub entries_erased: usize,
    pub universes_touched: usize,
    /// Whether a WhatsApp link existed and was cleared by this call.
    pub link_cleared: bool,
    /// Whether a consent record existed and was cleared by this call.
    pub consent_cleared: bool,
    /// Always `true` — instructs the bot to clear its local identity store.
    pub clear_local_identity: bool,
    pub user_id: String,
}

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

/// Body for the destructive endpoints. The `confirm` phrase must deterministically
/// match the endpoint's required phrase or the request is rejected (`400`).
#[derive(Debug, Deserialize)]
pub struct ConfirmBody {
    #[serde(default)]
    pub confirm: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a server)
// ---------------------------------------------------------------------------

/// Normalize a confirmation phrase for a forgiving-but-deterministic compare:
/// trim, lowercase, and collapse internal whitespace. Pure → unit-testable.
fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Does `provided` match `expected` after normalization? Pure → unit-testable.
fn confirms(provided: &str, expected: &str) -> bool {
    normalize(provided) == normalize(expected)
}

/// Gate a destructive op on its explicit confirmation phrase. `400` (never a
/// destructive action) when the token is absent or does not match.
fn require_confirmation(provided: Option<&str>, expected: &str) -> Result<(), AppError> {
    match provided {
        Some(c) if confirms(c, expected) => Ok(()),
        _ => Err(AppError::BadRequest(format!(
            "destructive operation requires explicit confirmation: send {{\"confirm\": \"{expected}\"}}"
        ))),
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Core (caller-scoped) — take `&Storage` so they are testable without a server
// ---------------------------------------------------------------------------

/// Keys of the universes the caller OWNS — the deterministic surface the bot
/// curates content into. Caller-scoped: `list_owned_universes` filters on
/// `owner_id = user_id`, so this never returns another user's universe.
fn owned_universe_keys(storage: &Storage, user_id: &str) -> Vec<String> {
    storage
        .list_owned_universes(user_id)
        .into_iter()
        .map(|u| u.universe.key)
        .collect()
}

/// Build the portable export bundle for `user_id`. Reads only.
fn collect_export(storage: &Storage, user_id: &str) -> ExportBundle {
    let profile = storage.get_user_by_id(user_id).map(|u| ProfileDto {
        id: u.id,
        email: u.email,
        display_name: u.display_name,
        tier: u.tier,
        created_at: u.created_at.to_rfc3339(),
        usuario: u.usuario,
    });
    let whatsapp = storage.get_user_whatsapp(user_id);
    let consent = storage
        .get_whatsapp_consent(user_id)
        .map(|(version, at, sha)| ConsentDto { version, at, sha });

    let mut universes = Vec::new();
    let mut entries = Vec::new();
    for key in owned_universe_keys(storage, user_id) {
        let repo = SqliteEntryRepository::new(storage.universe_conn(&key));
        // Empty prefix ⇒ every entry in the universe.
        let rows = repo
            .query_by_path_prefix(&key, "", None)
            .unwrap_or_default();
        universes.push(UniverseSummary {
            key: key.clone(),
            entry_count: rows.len(),
        });
        for r in rows {
            entries.push(ExportEntry {
                universe_key: r.universe_key,
                path: r.path,
                entry_type: r.entry_type,
                title: r.title,
                frontmatter: r.frontmatter,
                body: r.body,
                created_at: r.created_at,
                updated_at: r.updated_at,
            });
        }
    }

    let markdown = render_markdown(user_id, &profile, &consent, &entries);
    ExportBundle {
        user_id: user_id.to_string(),
        generated_at: now_rfc3339(),
        profile,
        whatsapp,
        consent,
        universes,
        entries,
        markdown,
    }
}

/// Render the export as portable markdown — a single document a user can keep.
fn render_markdown(
    user_id: &str,
    profile: &Option<ProfileDto>,
    consent: &Option<ConsentDto>,
    entries: &[ExportEntry],
) -> String {
    let mut md = String::new();
    md.push_str("# Exportação de dados (LGPD Art. 18 V)\n\n");
    if let Some(p) = profile {
        md.push_str(&format!(
            "- Usuário: {} ({})\n- Conta criada em: {}\n",
            p.display_name, p.email, p.created_at
        ));
    } else {
        md.push_str(&format!("- Usuário: {user_id}\n"));
    }
    if let Some(c) = consent {
        md.push_str(&format!("- Consentimento: {} (em {})\n", c.version, c.at));
    }
    md.push_str(&format!("\n## Conteúdo ({} entradas)\n", entries.len()));
    for e in entries {
        let title = e.title.as_deref().unwrap_or(&e.path);
        md.push_str(&format!(
            "\n### {} (`{}::{}`)\n\n",
            title, e.universe_key, e.path
        ));
        md.push_str(&e.body);
        md.push('\n');
    }
    md
}

/// Erase every entry in the caller's owned universes. Returns
/// `(universes_touched, entries_erased)`. Idempotent: a second call finds no
/// rows and reports zeros.
fn erase_owned_entries(storage: &Storage, user_id: &str) -> (usize, usize) {
    let mut universes_touched = 0;
    let mut entries_erased = 0;
    for key in owned_universe_keys(storage, user_id) {
        let repo = SqliteEntryRepository::new(storage.universe_conn(&key));
        let paths: Vec<String> = repo
            .query_by_path_prefix(&key, "", None)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.path)
            .collect();
        let mut touched = false;
        for path in paths {
            if repo.unindex_entry(&key, &path).is_ok() {
                entries_erased += 1;
                touched = true;
            }
        }
        if touched {
            universes_touched += 1;
        }
    }
    (universes_touched, entries_erased)
}

/// Full forget: erase curated content, then clear the WhatsApp link + consent.
/// Idempotent — a repeat call erases nothing more, reports the link/consent as
/// already-clear, and still returns `clear_local_identity: true`.
fn forget(storage: &Storage, user_id: &str) -> ForgetSummary {
    let link_cleared = storage.get_user_whatsapp(user_id).is_some();
    let consent_cleared = storage.get_whatsapp_consent(user_id).is_some();
    let (universes_touched, entries_erased) = erase_owned_entries(storage, user_id);
    // NULL the link + consent columns (idempotent on already-NULL columns).
    let _ = storage.clear_whatsapp_identity(user_id);
    ForgetSummary {
        forgotten: true,
        entries_erased,
        universes_touched,
        link_cleared,
        consent_cleared,
        clear_local_identity: true,
        user_id: user_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/whatsapp/me/export` — caller-scoped portable export (Art. 18 V).
async fn export_handler(
    cap: Scoped<EntriesRead>,
    State(state): State<AppState>,
) -> Result<Json<ExportBundle>, AppError> {
    let storage = state.core.storage.lock();
    Ok(Json(collect_export(&storage, &cap.user_id)))
}

/// `POST /api/v1/whatsapp/me/erase` — caller-scoped erase (Art. 18 VI). Requires
/// the deterministic confirmation phrase in the body.
async fn erase_handler(
    cap: Scoped<EntriesWrite>,
    State(state): State<AppState>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<EraseSummary>, AppError> {
    require_confirmation(body.confirm.as_deref(), ERASE_CONFIRM_PHRASE)?;
    let storage = state.core.storage.lock();
    let (universes_touched, entries_erased) = erase_owned_entries(&storage, &cap.user_id);
    Ok(Json(EraseSummary {
        erased: true,
        entries_erased,
        universes_touched,
    }))
}

/// `POST /api/v1/whatsapp/me/forget` — caller-scoped full forget (Art. 18 +
/// revogação). Requires the deterministic confirmation phrase in the body.
async fn forget_handler(
    cap: Scoped<EntriesWrite>,
    State(state): State<AppState>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<ForgetSummary>, AppError> {
    require_confirmation(body.confirm.as_deref(), FORGET_CONFIRM_PHRASE)?;
    let storage = state.core.storage.lock();
    Ok(Json(forget(&storage, &cap.user_id)))
}

/// Router for the LGPD data-rights endpoints, nested at `/api/v1` (full paths
/// `/api/v1/whatsapp/me/{export,erase,forget}`). Each handler self-gates via
/// `Scoped<…>`, so no auth middleware layer is needed at the mount site.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/whatsapp/me/export", get(export_handler))
        .route("/whatsapp/me/erase", post(erase_handler))
        .route("/whatsapp/me/forget", post(forget_handler))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- confirmation gate (pure) ----

    #[test]
    fn confirms_is_trim_case_and_whitespace_insensitive() {
        assert!(confirms("apagar tudo", ERASE_CONFIRM_PHRASE));
        assert!(confirms("  APAGAR   TUDO  ", ERASE_CONFIRM_PHRASE));
        assert!(confirms("Esquece De Mim", FORGET_CONFIRM_PHRASE));
        // Wrong phrase never confirms.
        assert!(!confirms("apaga", ERASE_CONFIRM_PHRASE));
        assert!(!confirms("", ERASE_CONFIRM_PHRASE));
        // Cross-phrase must not confirm the other op.
        assert!(!confirms(FORGET_CONFIRM_PHRASE, ERASE_CONFIRM_PHRASE));
    }

    #[test]
    fn require_confirmation_rejects_missing_or_wrong_token() {
        // Absent token → 400, no action.
        assert!(matches!(
            require_confirmation(None, ERASE_CONFIRM_PHRASE),
            Err(AppError::BadRequest(_))
        ));
        // Wrong token → 400.
        assert!(matches!(
            require_confirmation(Some("yes"), ERASE_CONFIRM_PHRASE),
            Err(AppError::BadRequest(_))
        ));
        // Correct token → Ok.
        assert!(require_confirmation(Some("apagar tudo"), ERASE_CONFIRM_PHRASE).is_ok());
    }

    // ---- storage-backed core (caller-scoped) ----

    use crate::storage::Storage;

    /// Create a user, a universe they own, and `n` entries in it. Returns the
    /// owner id and the universe key.
    fn user_with_universe(storage: &mut Storage, email: &str, key: &str, paths: &[&str]) -> String {
        let user = storage.create_user(email, "Name").expect("create user");
        storage
            .create_universe(
                crate::models::CreateUniverse {
                    key: key.into(),
                    name: key.into(),
                    description: String::new(),
                },
                &user.id,
            )
            .expect("create universe");
        let uc = storage.universe_conn(key);
        {
            let conn = uc.lock().unwrap();
            for p in paths {
                conn.execute(
                    "INSERT INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, 'note', ?3, '{}', ?4, 'h', 'now', 'now')",
                    rusqlite::params![p, key, p, format!("body of {p}")],
                )
                .unwrap();
            }
        }
        user.id
    }

    #[test]
    fn export_returns_only_the_callers_own_data_never_anothers() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        let alice = user_with_universe(&mut storage, "a@x.com", "alice-uni", &["nota1", "nota2"]);
        let _bob = user_with_universe(&mut storage, "b@x.com", "bob-uni", &["secreto"]);
        storage.set_user_whatsapp(&alice, "5541999990000").unwrap();

        let bundle = collect_export(&storage, &alice);

        // Profile + link are the caller's own.
        assert_eq!(bundle.user_id, alice);
        assert_eq!(bundle.profile.as_ref().unwrap().email, "a@x.com");
        assert_eq!(bundle.whatsapp.as_deref(), Some("5541999990000"));

        // Exactly Alice's two entries, from Alice's universe only.
        assert_eq!(bundle.entries.len(), 2);
        assert!(bundle.entries.iter().all(|e| e.universe_key == "alice-uni"));
        let paths: Vec<&str> = bundle.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"nota1") && paths.contains(&"nota2"));
        // Bob's universe / entry never leaks into Alice's export.
        assert!(!bundle.entries.iter().any(|e| e.path == "secreto"));
        assert!(!bundle.universes.iter().any(|u| u.key == "bob-uni"));
        // Markdown carries the content too.
        assert!(bundle.markdown.contains("body of nota1"));
    }

    #[test]
    fn erase_removes_only_the_callers_entries_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        let alice = user_with_universe(&mut storage, "a@x.com", "alice-uni", &["nota1", "nota2"]);
        let bob = user_with_universe(&mut storage, "b@x.com", "bob-uni", &["secreto"]);

        let (universes_touched, erased) = erase_owned_entries(&storage, &alice);
        assert_eq!(erased, 2, "both of Alice's entries erased");
        assert_eq!(universes_touched, 1);

        // Alice's export is now empty…
        assert!(collect_export(&storage, &alice).entries.is_empty());
        // …but Bob's data is untouched (a second user cannot be touched).
        assert_eq!(collect_export(&storage, &bob).entries.len(), 1);

        // Idempotent: erasing again removes nothing more.
        let (_t, again) = erase_owned_entries(&storage, &alice);
        assert_eq!(again, 0);
    }

    #[test]
    fn forget_clears_link_and_consent_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        let alice = user_with_universe(&mut storage, "a@x.com", "alice-uni", &["nota1"]);
        storage.set_user_whatsapp(&alice, "5541999990000").unwrap();
        storage
            .set_whatsapp_consent(
                &alice,
                "whatsapp-consent/2026-06-27.v1",
                "2026-06-27T00:00:00+00:00",
                "deadbeef",
            )
            .unwrap();

        // First forget: erases content, clears link + consent, signals the bot.
        let s = forget(&storage, &alice);
        assert!(s.forgotten);
        assert!(
            s.clear_local_identity,
            "bot must clear its local identity store"
        );
        assert_eq!(s.entries_erased, 1);
        assert!(s.link_cleared);
        assert!(s.consent_cleared);

        // Link + consent are gone CO-side.
        assert_eq!(storage.get_user_whatsapp(&alice), None);
        assert_eq!(storage.get_whatsapp_consent(&alice), None);
        assert!(collect_export(&storage, &alice).entries.is_empty());

        // Idempotent: a second forget still succeeds + still signals, but reports
        // nothing left to clear.
        let s2 = forget(&storage, &alice);
        assert!(s2.forgotten && s2.clear_local_identity);
        assert_eq!(s2.entries_erased, 0);
        assert!(!s2.link_cleared, "link already cleared");
        assert!(!s2.consent_cleared, "consent already cleared");
    }

    #[test]
    fn a_second_user_cannot_touch_the_firsts_data_via_forget() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());

        let alice = user_with_universe(&mut storage, "a@x.com", "alice-uni", &["nota1", "nota2"]);
        let bob = user_with_universe(&mut storage, "b@x.com", "bob-uni", &["secreto"]);
        storage.set_user_whatsapp(&alice, "5541999990000").unwrap();

        // Bob forgets HIMSELF — Alice is entirely unaffected.
        let s = forget(&storage, &bob);
        assert_eq!(s.entries_erased, 1, "only Bob's own entry");
        assert_eq!(collect_export(&storage, &alice).entries.len(), 2);
        assert_eq!(
            storage.get_user_whatsapp(&alice).as_deref(),
            Some("5541999990000"),
            "Bob's forget never clears Alice's link"
        );
    }
}
