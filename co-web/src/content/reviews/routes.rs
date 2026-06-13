//! CO-354: suggest / review pipeline — entry lifecycle.
//!
//! A *suggestion* is any entry submitted by someone who is not the universe
//! owner — a logged-in contributor or an anonymous visitor. It lands with
//! `review_status: draft`, hidden from public listings, and waits in an
//! owner-facing review queue until the owner approves (→ `published`) or
//! rejects (deletes) it.
//!
//! Endpoints (all under `/api/v1/universes/{slug}`):
//!
//! | Method | Route | Auth | Notes |
//! |---|---|---|---|
//! | `POST`  | `/suggest`        | optional (anon ok) | Creates a `draft` entry. Rate-limited + honeypot. |
//! | `GET`   | `/review`         | owner | Lists draft + reviewed entries, newest first. |
//! | `POST`  | `/review/approve` | owner | Body `{path}` → sets `review_status: published`. |
//! | `POST`  | `/review/reject`  | owner | Body `{path}` → deletes the entry. |
//! | `PATCH` | `/review`         | owner | Body `{path, frontmatter?, body?}` → edit-then-approve. |
//!
//! The lifecycle is stored both in the `entries` columns (authoritative for
//! the review-queue query + its partial index) and mirrored into frontmatter
//! (so the read-path filter in `entry_routes` — which only reads
//! `frontmatter_json` — can hide non-published entries). Every transition
//! publishes an EDA event (`entry.suggest` / `entry.review.approve` /
//! `entry.review.reject` / `entry.review.edit`) so the atividades log (CO-351 /
//! CO-380) records it, and notifies the owner (on suggest) or the submitter
//! (on approve/reject) via the CO-329 notification system.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::entry_index::{EntryRow, make_entry};
use crate::error::AppError;
use crate::server::AppState;

/// Resolved owner context for an owner-only review action.
struct Owner {
    user_id: String,
    slug: String,
}

/// Require that the caller is authenticated AND owns `slug`. Returns the
/// owner's `user_id`. Mirrors the `OwnerOf` extractor but without the
/// `MatchedPath` dependency (which is unreliable under nested routers).
fn require_owner(state: &AppState, headers: &HeaderMap, slug: &str) -> Result<Owner, AppError> {
    let user_id = crate::auth::resolve_user_id(state, headers)
        .ok_or_else(|| AppError::Unauthorized("Login required".into()))?;
    let universe = state
        .core
        .storage_trait
        .get_universe(slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
    if universe.owner_id != user_id {
        return Err(AppError::Forbidden(
            "Not authorized: you do not own this universe".into(),
        ));
    }
    Ok(Owner {
        user_id,
        slug: slug.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SuggestBody {
    /// Entry type for the suggested entry (e.g. `note`, `term`, `event`).
    #[serde(default = "default_entry_type")]
    pub entry_type: String,
    /// Optional explicit vault path. When absent a `suggestions/<id>.md` path
    /// is generated. Must stay inside the universe (no `..` / leading `/`).
    pub path: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Extra frontmatter merged into the entry (type/title/review_* are owned
    /// by the server and overwrite any caller-supplied values).
    #[serde(default)]
    pub frontmatter: JsonValue,
    /// Optional display name for an anonymous submitter (kept in frontmatter).
    pub name: Option<String>,
    /// Optional contact email — kept private, used only to notify on
    /// approve/reject. Never exposed in listings.
    pub email: Option<String>,
    /// Honeypot — real users leave this empty; bots fill it. Non-empty ⇒ the
    /// submission is silently accepted but discarded.
    #[serde(default)]
    pub honeypot: String,
}

fn default_entry_type() -> String {
    "note".to_string()
}

#[derive(Debug, Serialize)]
pub struct SuggestResponse {
    pub path: String,
    pub review_status: String,
    pub submitted_by: String,
}

#[derive(Debug, Serialize)]
pub struct ReviewQueueResponse {
    pub entries: Vec<EntryRow>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct ReviewActionBody {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct EditApproveBody {
    pub path: String,
    /// Partial frontmatter patch applied before publishing.
    pub frontmatter: Option<JsonValue>,
    /// Replacement body applied before publishing.
    pub body: Option<String>,
}

/// CO-418: render-review-publish-with-traceback.
///
/// Publishes an entry (typically one imported via CO-417's `source: github`
/// adapter) as a *conventional, semver-aware* publication that traces back to
/// (a) the original source and (b) the task that requested the publish.
#[derive(Debug, Deserialize)]
pub struct PublishBody {
    /// Vault path of the entry to publish.
    pub path: String,
    /// The task that asked for this publish, e.g. `CO-419`. Stamped into
    /// frontmatter as `requested_by` and surfaced as a typed `requested_by`
    /// relation (CO-74).
    pub requested_by: String,
    /// Conventional-commit type: `feat` | `fix` | `docs` | `chore` | `refactor`.
    /// Defaults to `docs` (content publication is a docs-level change).
    pub commit_type: Option<String>,
    /// Optional conventional-commit scope, e.g. `pipeline`.
    pub commit_scope: Option<String>,
    /// Commit subject line. Defaults to a generated `publish <path>`.
    pub commit_subject: Option<String>,
    /// Optional frontmatter patch applied before publishing.
    pub frontmatter: Option<JsonValue>,
    /// Optional replacement body applied before publishing.
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub path: String,
    pub review_status: String,
    /// The conventional commit message recorded for this publish.
    pub commit_message: String,
    /// The semver bump implied by `commit_type` (`minor` | `patch` | `none`).
    pub semver_bump: String,
    /// Content hash that gates idempotency. Re-publishing an unchanged entry
    /// keeps the same `published_sha` and records no new commit.
    pub published_sha: String,
    /// `true` when this call recorded a NEW publish; `false` when it was a
    /// no-op re-publish of unchanged content (idempotent).
    pub published: bool,
    pub source: Option<String>,
    pub requested_by: String,
}

/// Map a conventional-commit type to its semver bump intent (CO-258: we record
/// the *intended* bump but never touch `Cargo.toml` — the release commit owns
/// the actual version mutation).
fn semver_bump_for(commit_type: &str) -> &'static str {
    match commit_type {
        "feat" => "minor",
        "fix" | "docs" | "refactor" | "perf" => "patch",
        _ => "none",
    }
}

/// Build a conventional-commit message: `type(scope): subject`.
fn conventional_commit_message(
    commit_type: &str,
    commit_scope: Option<&str>,
    subject: &str,
) -> String {
    match commit_scope.filter(|s| !s.trim().is_empty()) {
        Some(scope) => format!("{commit_type}({}): {subject}", scope.trim()),
        None => format!("{commit_type}: {subject}"),
    }
}

/// Idempotency key for a publish: a stable hash over the published content +
/// provenance. Re-publishing the same body/source/requested_by/commit yields
/// the same hash, so no new commit is recorded.
fn publish_sha(body: &str, source: &str, requested_by: &str, commit_message: &str) -> String {
    let material = format!("{body}\n{source}\n{requested_by}\n{commit_message}");
    co::entry::Entry::hash_body(&material)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stable identity for the submitter: the authenticated `user_id`, else an
/// `anon:<session>` key derived from the visitor cookie, else `anon:<ip>`.
/// The same function is used at read time so an anon submitter can see their
/// own draft across requests in the same session.
pub(crate) fn submitter_key(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(uid) = crate::auth::resolve_user_id(state, headers) {
        return uid;
    }
    if let Some(sid) = crate::telemetry::extract_session_id(headers) {
        return format!("anon:{sid}");
    }
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    format!("anon:{ip}")
}

/// Reject paths that would escape the universe root or collide with reserved
/// metadata folders.
fn sanitize_suggested_path(path: &str) -> Result<String, AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Entry path cannot be empty".into()));
    }
    if trimmed.starts_with('/')
        || trimmed.contains("..")
        || trimmed.contains('\\')
        || trimmed.contains('\0')
    {
        return Err(AppError::BadRequest(format!("Invalid entry path '{path}'")));
    }
    Ok(trimmed.to_string())
}

/// True when the test rate-limit bypass is active (CO-208 fixtures).
fn rate_limit_bypassed(state: &AppState) -> bool {
    state.core.config.bypass_rate_limit && state.core.config.co_env == "test"
}

/// Write an entry to disk + index it, mirroring the core of `create_entry`
/// (file write, index upsert, semantic dates, relations, event bus). Returns
/// the resulting `body_hash`.
fn write_and_index(state: &AppState, slug: &str, entry: &co::entry::Entry) -> Result<(), AppError> {
    let universe_root = {
        let storage = state.core.storage.lock();
        storage.universe_root(slug)
    };
    co::write_entry(&universe_root, entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(slug)
    };
    let repo = crate::repository::SqliteEntryRepository::new(uc.clone());
    repo.upsert_entry(slug, entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    repo.upsert_dates(slug, entry, None)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let _ = crate::repository::SqliteRelationRepository::new(uc).sync_entry_relations(
        slug,
        &entry.path,
        &entry.entry_type,
        &entry.frontmatter,
        &entry.body,
        None,
    );
    Ok(())
}

/// Persist the review-status columns AND keep `frontmatter.review_status` in
/// sync (the read path only sees frontmatter).
#[allow(clippy::too_many_arguments)]
fn set_review_status(
    state: &AppState,
    slug: &str,
    path: &str,
    status: &str,
    submitted_by: Option<&str>,
    submitted_at: Option<&str>,
    reviewed_by: Option<&str>,
    reviewed_at: Option<&str>,
) -> Result<(), AppError> {
    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(slug)
    };
    crate::repository::SqliteEntryRepository::new(uc)
        .set_review_fields(
            slug,
            path,
            status,
            submitted_by,
            submitted_at,
            reviewed_by,
            reviewed_at,
        )
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ---------------------------------------------------------------------------
// POST /suggest
// ---------------------------------------------------------------------------

pub async fn suggest(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SuggestBody>,
) -> Result<impl IntoResponse, AppError> {
    // Honeypot: pretend success, write nothing.
    if !body.honeypot.trim().is_empty() {
        return Ok((StatusCode::ACCEPTED, Json(json!({ "status": "accepted" }))).into_response());
    }

    if body.title.trim().is_empty() {
        return Err(AppError::BadRequest("Suggestion title is required".into()));
    }

    let universe = state
        .core
        .storage_trait
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;

    let submitter = submitter_key(&state, &headers);
    let is_anon = crate::auth::resolve_user_id(&state, &headers).is_none();

    // Anti-spam: rate-limit anonymous submissions (reuses the CO-329 token
    // bucket — 5 burst). Authenticated submitters are not throttled here.
    if is_anon && !rate_limit_bypassed(&state) {
        let key = format!("suggest:{submitter}");
        let mut limiter = state
            .integrations
            .rate_limiter
            .lock()
            .map_err(|_| AppError::Internal("rate limiter lock".into()))?;
        if let Err(retry) = limiter.check(&key, 5) {
            return Err(AppError::RateLimited {
                retry_after_secs: retry,
            });
        }
    }

    // Resolve the entry path (generated or caller-supplied + sanitized).
    let path = match body.path.as_deref() {
        Some(p) => sanitize_suggested_path(p)?,
        None => format!("suggestions/{}.md", nanoid::nanoid!(12)),
    };

    let now = chrono::Utc::now().to_rfc3339();

    // Build frontmatter: caller fields first, then server-owned overrides.
    let mut fm = match body.frontmatter {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    fm.insert("type".into(), json!(body.entry_type));
    fm.insert("title".into(), json!(body.title.trim()));
    fm.insert("review_status".into(), json!("draft"));
    fm.insert("submitted_by".into(), json!(submitter));
    fm.insert("submitted_at".into(), json!(now));
    if let Some(name) = body.name.as_deref().filter(|s| !s.trim().is_empty()) {
        fm.insert("submitted_name".into(), json!(name.trim()));
    }
    if let Some(email) = body.email.as_deref().filter(|s| !s.trim().is_empty()) {
        // Private — used only for approve/reject notification. Not surfaced in
        // listings (the review queue is owner-only).
        fm.insert("submitted_email".into(), json!(email.trim()));
    }
    let frontmatter = JsonValue::Object(fm);

    let entry = make_entry(&path, frontmatter, &body.body);
    write_and_index(&state, &slug, &entry)?;
    // Override the default 'published' column → 'draft' + provenance.
    set_review_status(
        &state,
        &slug,
        &path,
        "draft",
        Some(&submitter),
        Some(&now),
        None,
        None,
    )?;

    {
        let mut storage = state.core.storage.lock();
        storage.increment_universe_content_count(&slug);
    }

    // CO-329: notify the owner (in-app + email via the notification worker).
    if !universe.owner_id.starts_with("anon-") && universe.owner_id != "system" {
        let storage = state.core.storage.lock();
        let _ = storage.create_notification(
            &universe.owner_id,
            "entry.suggested",
            Some(&slug),
            None,
            &submitter,
            &path,
            "notif.entry_suggested",
            json!({ "title": body.title.trim(), "universe": slug }),
        );
    }

    // CO-351 / CO-380: atividades log via the EDA bus.
    emit_lifecycle(
        &state,
        "entry.suggest",
        &slug,
        &path,
        &submitter,
        "none",
        "draft",
    );

    // Invalidate cached queries for this universe.
    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

    Ok((
        StatusCode::CREATED,
        Json(SuggestResponse {
            path,
            review_status: "draft".into(),
            submitted_by: submitter,
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// GET /review  (owner-only review queue)
// ---------------------------------------------------------------------------

pub async fn review_queue(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let owner = require_owner(&state, &headers, &slug)?;
    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&owner.slug)
    };
    let entries = crate::repository::SqliteEntryRepository::new(uc)
        .list_review_queue(&owner.slug)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let total = entries.len();
    Ok(Json(ReviewQueueResponse { entries, total }))
}

// ---------------------------------------------------------------------------
// POST /review/approve
// ---------------------------------------------------------------------------

pub async fn approve(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ReviewActionBody>,
) -> Result<impl IntoResponse, AppError> {
    let owner = require_owner(&state, &headers, &slug)?;
    publish_entry(&state, &owner, &body.path, None, None).await?;
    Ok(Json(json!({
        "path": body.path,
        "review_status": "published",
    })))
}

// ---------------------------------------------------------------------------
// PATCH /review  (edit-then-approve)
// ---------------------------------------------------------------------------

pub async fn edit_approve(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<EditApproveBody>,
) -> Result<impl IntoResponse, AppError> {
    let owner = require_owner(&state, &headers, &slug)?;
    publish_entry(
        &state,
        &owner,
        &body.path,
        body.frontmatter.as_ref(),
        body.body.as_deref(),
    )
    .await?;
    Ok(Json(json!({
        "path": body.path,
        "review_status": "published",
    })))
}

// ---------------------------------------------------------------------------
// POST /publish  (CO-418: render-review-publish with traceback)
// ---------------------------------------------------------------------------

/// Publish an entry as a conventional, semver-aware publication that carries a
/// traceback to its source (CO-417 `source` frontmatter) and the task that
/// requested it (`requested_by`). Owner-only. Idempotent: re-publishing the
/// same content records no new commit.
pub async fn publish(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PublishBody>,
) -> Result<impl IntoResponse, AppError> {
    let owner = require_owner(&state, &headers, &slug)?;
    let slug = &owner.slug;

    if body.requested_by.trim().is_empty() {
        return Err(AppError::BadRequest(
            "`requested_by` is required (the task that asked for the publish)".into(),
        ));
    }

    let existing = state
        .core
        .storage_trait
        .get_entry(slug, &body.path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", body.path)))?;

    let from_status = existing
        .frontmatter
        .get("review_status")
        .and_then(|v| v.as_str())
        .unwrap_or("published")
        .to_string();

    // Build the conventional commit message + semver intent.
    let commit_type = body.commit_type.as_deref().unwrap_or("docs").trim();
    let commit_type = if commit_type.is_empty() {
        "docs"
    } else {
        commit_type
    };
    let subject = body
        .commit_subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("publish {}", body.path));
    let commit_message =
        conventional_commit_message(commit_type, body.commit_scope.as_deref(), &subject);
    let semver_bump = semver_bump_for(commit_type);

    // Assemble the published frontmatter: existing + optional patch + provenance.
    let mut fm = existing
        .frontmatter
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(JsonValue::Object(patch)) = body.frontmatter.as_ref() {
        for (k, v) in patch {
            fm.insert(k.clone(), v.clone());
        }
    }

    let new_body = body.body.as_deref().unwrap_or(&existing.body);
    let source = fm
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let new_sha = publish_sha(new_body, &source, body.requested_by.trim(), &commit_message);

    // Idempotency: an unchanged re-publish records no new commit.
    let prev_sha = existing
        .frontmatter
        .get("published_sha")
        .and_then(|v| v.as_str());
    let already_published = prev_sha == Some(new_sha.as_str());

    if already_published {
        return Ok(Json(PublishResponse {
            path: body.path.clone(),
            review_status: "published".into(),
            commit_message,
            semver_bump: semver_bump.into(),
            published_sha: new_sha,
            published: false,
            source: (!source.is_empty()).then_some(source),
            requested_by: body.requested_by.trim().to_string(),
        })
        .into_response());
    }

    let now = chrono::Utc::now().to_rfc3339();
    // Provenance + publish-record stamps. `source`/`source_path`/`source_kind`
    // (from CO-417) are preserved untouched; we ADD `requested_by` + the
    // publish record so the traceback is complete.
    fm.insert("requested_by".into(), json!(body.requested_by.trim()));
    fm.insert("review_status".into(), json!("published"));
    fm.insert("reviewed_by".into(), json!(owner.user_id));
    fm.insert("reviewed_at".into(), json!(now));
    fm.insert("published_at".into(), json!(now));
    fm.insert("published_commit".into(), json!(commit_message));
    fm.insert("published_semver".into(), json!(semver_bump));
    fm.insert("published_sha".into(), json!(new_sha));
    let frontmatter = JsonValue::Object(fm);

    let entry = make_entry(&body.path, frontmatter, new_body);
    // write_and_index re-syncs relations → the `origin` + `requested_by` typed
    // edges (CO-418 / CO-74) are derived from frontmatter here.
    write_and_index(&state, slug, &entry)?;
    let submitter = existing
        .frontmatter
        .get("submitted_by")
        .and_then(|v| v.as_str());
    let submitted_at = existing
        .frontmatter
        .get("submitted_at")
        .and_then(|v| v.as_str());
    set_review_status(
        &state,
        slug,
        &body.path,
        "published",
        submitter,
        submitted_at,
        Some(&owner.user_id),
        Some(&now),
    )?;

    // EDA: a publish event carrying the commit metadata + traceback so the
    // atividades/timeline subscribers (CO-351/CO-380) can record it.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "entry.published",
        Some(slug.to_string()),
        Some(owner.user_id.clone()),
        json!({
            "entry_id": body.path,
            "actor": owner.user_id,
            "from_status": from_status,
            "to_status": "published",
            "commit_message": commit_message,
            "semver_bump": semver_bump,
            "source": source,
            "requested_by": body.requested_by.trim(),
        }),
        crate::eda::Visibility::UniverseOwner,
    ));

    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

    Ok(Json(PublishResponse {
        path: body.path.clone(),
        review_status: "published".into(),
        commit_message,
        semver_bump: semver_bump.into(),
        published_sha: new_sha,
        published: true,
        source: (!source.is_empty()).then_some(source),
        requested_by: body.requested_by.trim().to_string(),
    })
    .into_response())
}

/// Shared approve / edit-then-approve path: optionally patch the entry, then
/// flip `review_status` → `published`, reindex, notify, and log.
async fn publish_entry(
    state: &AppState,
    owner: &Owner,
    path: &str,
    fm_patch: Option<&JsonValue>,
    body_patch: Option<&str>,
) -> Result<(), AppError> {
    let slug = &owner.slug;
    let existing = state
        .core
        .storage_trait
        .get_entry(slug, path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{path}' not found")))?;

    let from_status = existing
        .frontmatter
        .get("review_status")
        .and_then(|v| v.as_str())
        .unwrap_or("published")
        .to_string();

    let now = chrono::Utc::now().to_rfc3339();

    // Start from existing frontmatter, apply the optional patch.
    let mut fm = existing
        .frontmatter
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(JsonValue::Object(patch)) = fm_patch {
        for (k, v) in patch {
            fm.insert(k.clone(), v.clone());
        }
    }
    let submitter = fm
        .get("submitted_by")
        .and_then(|v| v.as_str())
        .map(String::from);
    fm.insert("review_status".into(), json!("published"));
    fm.insert("reviewed_by".into(), json!(owner.user_id));
    fm.insert("reviewed_at".into(), json!(now));
    let frontmatter = JsonValue::Object(fm);

    let new_body = body_patch.unwrap_or(&existing.body);
    let entry = make_entry(path, frontmatter, new_body);
    write_and_index(state, slug, &entry)?;
    let submitted_at = existing
        .frontmatter
        .get("submitted_at")
        .and_then(|v| v.as_str());
    set_review_status(
        state,
        slug,
        path,
        "published",
        submitter.as_deref(),
        submitted_at,
        Some(&owner.user_id),
        Some(&now),
    )?;

    let kind = if body_patch.is_some() || fm_patch.is_some() {
        "entry.review.edit"
    } else {
        "entry.review.approve"
    };
    notify_submitter(
        state,
        submitter.as_deref(),
        existing
            .frontmatter
            .get("submitted_email")
            .and_then(|v| v.as_str()),
        slug,
        path,
        "approved",
    );
    emit_lifecycle(
        state,
        kind,
        slug,
        path,
        &owner.user_id,
        &from_status,
        "published",
    );

    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /review/reject
// ---------------------------------------------------------------------------

pub async fn reject(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ReviewActionBody>,
) -> Result<impl IntoResponse, AppError> {
    let owner = require_owner(&state, &headers, &slug)?;
    let slug = &owner.slug;
    let existing = state
        .core
        .storage_trait
        .get_entry(slug, &body.path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{}' not found", body.path)))?;

    let from_status = existing
        .frontmatter
        .get("review_status")
        .and_then(|v| v.as_str())
        .unwrap_or("published")
        .to_string();
    let submitter = existing
        .frontmatter
        .get("submitted_by")
        .and_then(|v| v.as_str())
        .map(String::from);
    let submitter_email = existing
        .frontmatter
        .get("submitted_email")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Reject = delete (documented choice; archival is a Phase-2 concern).
    let universe_root = {
        let storage = state.core.storage.lock();
        storage.universe_root(slug)
    };
    co::delete_entry(&universe_root, &body.path).map_err(|e| AppError::Internal(e.to_string()))?;
    {
        let uc = {
            let storage = state.core.storage.lock();
            storage.universe_conn(slug)
        };
        use crate::repository::RelationRepository;
        let repo = crate::repository::SqliteEntryRepository::new(uc.clone());
        repo.remove(slug, &body.path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let _ = repo.remove_dates(slug, &body.path);
        let _ =
            crate::repository::SqliteRelationRepository::new(uc).delete_for_entry(slug, &body.path);
    }
    {
        let mut storage = state.core.storage.lock();
        storage.decrement_universe_content_count(slug, 1);
    }

    notify_submitter(
        &state,
        submitter.as_deref(),
        submitter_email.as_deref(),
        slug,
        &body.path,
        "rejected",
    );
    emit_lifecycle(
        &state,
        "entry.review.reject",
        slug,
        &body.path,
        &owner.user_id,
        &from_status,
        "rejected",
    );

    state
        .index
        .cache
        .query
        .invalidate_prefix(&format!("{slug}:"));

    Ok(Json(
        json!({ "path": body.path, "review_status": "rejected" }),
    ))
}

// ---------------------------------------------------------------------------
// Notification + atividades helpers
// ---------------------------------------------------------------------------

/// Notify the submitter of an approve/reject outcome. Logged-in submitters get
/// an in-app notification (CO-329); anon submitters who left an email get a
/// direct mail. No-op when neither is available.
fn notify_submitter(
    state: &AppState,
    submitter: Option<&str>,
    submitter_email: Option<&str>,
    slug: &str,
    path: &str,
    outcome: &str,
) {
    if let Some(uid) = submitter.filter(|s| !s.starts_with("anon:")) {
        let storage = state.core.storage.lock();
        let _ = storage.create_notification(
            uid,
            "entry.reviewed",
            Some(slug),
            None,
            "owner",
            path,
            "notif.entry_reviewed",
            json!({ "outcome": outcome, "universe": slug, "path": path }),
        );
    } else if let Some(email) = submitter_email {
        let subject = format!("Your suggestion was {outcome}");
        let body =
            format!("Your suggestion to the '{slug}' universe ({path}) was {outcome}.\n\n— CO");
        let _ = state.integrations.mail.send(email, &subject, &body);
    }
}

/// Publish a lifecycle transition to the EDA bus so the atividades log
/// (CO-351 / CO-380) records it.
fn emit_lifecycle(
    state: &AppState,
    kind: &str,
    slug: &str,
    path: &str,
    actor: &str,
    from_status: &str,
    to_status: &str,
) {
    let actor_user = if actor.starts_with("anon:") {
        None
    } else {
        Some(actor.to_string())
    };
    state.core.eda_bus.publish(crate::eda::Event::new(
        kind,
        Some(slug.to_string()),
        actor_user,
        json!({
            "entry_id": path,
            "actor": actor,
            "from_status": from_status,
            "to_status": to_status,
        }),
        crate::eda::Visibility::UniverseOwner,
    ));
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{slug}/suggest", post(suggest))
        .route("/{slug}/review", get(review_queue).patch(edit_approve))
        .route("/{slug}/review/approve", post(approve))
        .route("/{slug}/review/reject", post(reject))
        // CO-418: render-review-publish with traceback.
        .route("/{slug}/publish", post(publish))
}

#[cfg(test)]
mod tests;
