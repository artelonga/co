//! CO-45: UAT → dev change promotion — state tracking + version control backend
//!
//! Endpoints (auth required; handlers return 404 outside UAT mode):
//!   GET  /api/v1/uat/changes?since={snapshot_version}
//!   POST /api/v1/uat/export-patch

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::auth::UserId;
use crate::error::AppError;
use crate::models::UatMutation;
use crate::server::AppState;
use crate::storage::Storage;

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UatSnapshot {
    /// Human-readable version label, e.g. "v1", "v2".
    pub version: String,
    pub snapshot_number: i64,
    pub timestamp: String,
    pub schema_version: i64,
    /// Max `uat_mutations.id` at snapshot time; mutations with id > watermark are "new".
    pub mutation_watermark: i64,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ContentDiff {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UatChangesResponse {
    pub snapshot: String,
    pub mutations: Vec<UatMutation>,
    pub schema_changes: Vec<String>,
    pub content_diff: ContentDiff,
}

// ---------------------------------------------------------------------------
// Snapshot management
// ---------------------------------------------------------------------------

fn snapshots_dir(data_dir: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(data_dir).join("uat-snapshots")
}

/// Count existing snapshot files and return the next version number.
fn next_snapshot_number(data_dir: &str) -> i64 {
    let dir = snapshots_dir(data_dir);
    if !dir.exists() {
        return 1;
    }
    let count = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name();
                    let s = name.to_string_lossy();
                    s.starts_with('v') && s.ends_with(".json")
                })
                .count() as i64
        })
        .unwrap_or(0);
    count + 1
}

/// Create a new snapshot of the current UAT state.
/// Called from `uat_startup()` on every UAT deploy.
pub fn create_snapshot(data_dir: &str, storage: &Storage) -> anyhow::Result<UatSnapshot> {
    let dir = snapshots_dir(data_dir);
    std::fs::create_dir_all(&dir)?;

    let n = next_snapshot_number(data_dir);
    let watermark = storage.get_uat_mutations_watermark();
    let schema_ver = storage.schema_version();

    let snapshot = UatSnapshot {
        version: format!("v{n}"),
        snapshot_number: n,
        timestamp: chrono::Utc::now().to_rfc3339(),
        schema_version: schema_ver,
        mutation_watermark: watermark,
    };

    let path = dir.join(format!("v{n}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&snapshot)?)?;

    tracing::info!(
        "UAT: snapshot {} created (watermark={}, schema_v={})",
        snapshot.version,
        watermark,
        schema_ver
    );
    Ok(snapshot)
}

/// Load a snapshot by version label ("v1", "v2", …).
fn load_snapshot(data_dir: &str, version: &str) -> anyhow::Result<UatSnapshot> {
    let path = snapshots_dir(data_dir).join(format!("{version}.json"));
    let json = std::fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("Snapshot '{}' not found", version))?;
    Ok(serde_json::from_str(&json)?)
}

/// Return the latest snapshot (highest version number).
fn latest_snapshot(data_dir: &str) -> anyhow::Result<UatSnapshot> {
    let dir = snapshots_dir(data_dir);
    if !dir.exists() {
        return Err(anyhow::anyhow!("No snapshots found"));
    }
    let mut versions: Vec<i64> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            if s.starts_with('v') && s.ends_with(".json") {
                s[1..s.len() - 5].parse::<i64>().ok()
            } else {
                None
            }
        })
        .collect();
    versions.sort_unstable();
    let n = *versions
        .last()
        .ok_or_else(|| anyhow::anyhow!("No snapshots found"))?;
    load_snapshot(data_dir, &format!("v{n}"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive content diff from a list of mutations.
fn build_content_diff(mutations: &[UatMutation]) -> ContentDiff {
    let mut added: Vec<String> = vec![];
    let mut modified: Vec<String> = vec![];
    let mut deleted: Vec<String> = vec![];

    for m in mutations {
        match m.action.as_str() {
            "entry.create" => added.push(m.target.clone()),
            "entry.update" => {
                if !modified.contains(&m.target) {
                    modified.push(m.target.clone());
                }
            }
            "entry.delete" => deleted.push(m.target.clone()),
            _ => {}
        }
    }

    ContentDiff {
        added,
        modified,
        deleted,
    }
}

/// Resolve `since` query param to (label, watermark id).
///
/// Priority:
/// 1. If `since` matches a snapshot version (e.g. "v1") → use that snapshot's watermark.
/// 2. If `since` is a plain integer → use as raw id.
/// 3. If absent → use latest snapshot's watermark (or 0 if none).
fn resolve_since(data_dir: &str, since: Option<&str>) -> (String, i64) {
    if let Some(v) = since {
        // Try as snapshot version
        if let Ok(snap) = load_snapshot(data_dir, v) {
            return (snap.version, snap.mutation_watermark);
        }
        // Try as raw integer id
        if let Ok(id) = v.parse::<i64>() {
            return (format!("id:{id}"), id);
        }
    }
    // Fall back to latest snapshot
    match latest_snapshot(data_dir) {
        Ok(snap) => (snap.version, snap.mutation_watermark),
        Err(_) => ("none".to_string(), 0),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChangesQuery {
    pub since: Option<String>,
}

/// GET /api/v1/uat/changes?since={snapshot_version}
///
/// Returns all mutations recorded after the given snapshot.
/// Returns 404 outside UAT mode.
async fn changes_handler(
    State(state): State<AppState>,
    _user: UserId,
    Query(q): Query<ChangesQuery>,
) -> Result<Json<UatChangesResponse>, AppError> {
    if !state.core.config.is_uat() {
        return Err(AppError::NotFound(
            "UAT endpoint not available in production".into(),
        ));
    }

    let data_dir = &state.core.config.data_dir;
    let (snapshot_label, since_id) = resolve_since(data_dir, q.since.as_deref());

    let mutations = {
        let storage = state.core.storage.lock();
        storage.get_uat_mutations_since(since_id)
    };

    let content_diff = build_content_diff(&mutations);

    Ok(Json(UatChangesResponse {
        snapshot: snapshot_label,
        mutations,
        schema_changes: vec![],
        content_diff,
    }))
}

/// POST /api/v1/uat/export-patch
///
/// Generates a `.tar.gz` containing:
/// - `mutations.json`      — all mutations since last snapshot
/// - `data/promoted/**`    — modified entry `.md` files
/// - `README.md`           — apply instructions
///
/// Returns 404 outside UAT mode.
async fn export_patch_handler(
    State(state): State<AppState>,
    _user: UserId,
) -> Result<Response, AppError> {
    if !state.core.config.is_uat() {
        return Err(AppError::NotFound(
            "UAT endpoint not available in production".into(),
        ));
    }

    let data_dir = state.core.config.data_dir.clone();
    let (snapshot_label, since_id) = resolve_since(&data_dir, None);

    let mutations: Vec<UatMutation> = {
        let storage = state.core.storage.lock();
        storage.get_uat_mutations_since(since_id)
    };

    let content_diff = build_content_diff(&mutations);
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();

    let tar_bytes = build_tarball(
        &data_dir,
        &snapshot_label,
        &timestamp,
        &mutations,
        &content_diff,
    )
    .map_err(|e| AppError::Internal(format!("Failed to build tarball: {e}")))?;

    let filename = format!("uat-patch-{timestamp}.tar.gz");
    let disposition = format!("attachment; filename=\"{filename}\"");

    let mut resp = Response::new(axum::body::Body::from(tar_bytes));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, "application/gzip".parse().unwrap());
    if let Ok(hv) = disposition.parse() {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, hv);
    }
    Ok(resp)
}

// ---------------------------------------------------------------------------
// Tarball builder
// ---------------------------------------------------------------------------

fn build_tarball(
    data_dir: &str,
    snapshot: &str,
    timestamp: &str,
    mutations: &[UatMutation],
    content_diff: &ContentDiff,
) -> anyhow::Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;

    let buf: Vec<u8> = Vec::new();
    let gz = GzEncoder::new(buf, Compression::default());
    let mut tar = Builder::new(gz);

    // mutations.json
    let mutations_json = serde_json::to_vec_pretty(mutations)?;
    add_bytes_to_tar(&mut tar, "mutations.json", &mutations_json)?;

    // data/promoted/ — entry .md files for create/update mutations
    let universes_dir = std::path::PathBuf::from(data_dir).join("universes");
    for target in content_diff
        .added
        .iter()
        .chain(content_diff.modified.iter())
    {
        // target format: "{universe_key}:{entry_path}"
        let Some((universe_key, entry_path)) = target.split_once(':') else {
            continue;
        };
        let file_path = universes_dir.join(universe_key).join(entry_path);
        if let Ok(content) = std::fs::read(&file_path) {
            let tar_path = format!("data/promoted/{universe_key}/{entry_path}");
            add_bytes_to_tar(&mut tar, &tar_path, &content)?;
        }
    }

    // README.md
    let readme = format!(
        "# UAT Patch — {timestamp}\n\n\
         Snapshot: {snapshot}\n\n\
         ## Changes\n\n\
         - Added: {added} entry/entries\n\
         - Modified: {modified} entry/entries\n\
         - Deleted: {deleted} entry/entries\n\n\
         ## How to apply\n\n\
         ```bash\n\
         tar -xzf uat-patch-{timestamp}.tar.gz\n\
         cp -r data/promoted/* \"<your-data-dir>/universes/\"\n\
         git add . && git commit -m \"chore(co-web): promote UAT changes from {snapshot}\"\n\
         ```\n",
        added = content_diff.added.len(),
        modified = content_diff.modified.len(),
        deleted = content_diff.deleted.len(),
    );
    add_bytes_to_tar(&mut tar, "README.md", readme.as_bytes())?;

    let gz = tar.into_inner()?;
    Ok(gz.finish()?)
}

fn add_bytes_to_tar<W: std::io::Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    data: &[u8],
) -> anyhow::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(chrono::Utc::now().timestamp() as u64);
    header.set_cksum();
    tar.append_data(&mut header, path, data)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/changes", get(changes_handler))
        .route("/export-patch", post(export_patch_handler))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::require_auth,
        ))
}
