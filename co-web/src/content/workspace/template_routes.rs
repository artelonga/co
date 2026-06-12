//! CO-355: Workspace template registry.
//!
//! Reads `_workspace.yaml` (slug="default") and `_workspaces/*.yaml` from a
//! universe's content root and exposes them via:
//!
//! - `GET  /api/v1/universes/{key}/workspace-templates`
//! - `GET  /api/v1/universes/{key}/workspace-templates/{slug}`
//! - `POST /api/v1/universes/{key}/workspaces/{ws}/from-template/{tpl}`

use std::path::{Path, PathBuf};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::AppError;
use crate::server::{AppState, lock_storage};

// ─── Schema types ─────────────────────────────────────────────────────────────

/// A workspace template parsed from `_workspace.yaml` or `_workspaces/<slug>.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTemplate {
    /// Derived from the filename stem; not present in the YAML body.
    #[serde(skip_deserializing, default)]
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_layout: Option<TemplateLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_mode: Option<StudyMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_entry_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TemplateMetadata>,
}

impl WorkspaceTemplate {
    pub fn blank() -> Self {
        WorkspaceTemplate {
            slug: "blank".into(),
            name: "Blank".into(),
            description: Some("Empty canvas — build from scratch.".into()),
            default_layout: None,
            study_mode: None,
            allowed_entry_types: Vec::new(),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateLayout {
    #[serde(default)]
    pub nodes: Vec<TemplateNode>,
    #[serde(default)]
    pub edges: Vec<TemplateEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateNode {
    pub entry_path: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEdge {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub edge_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyMode {
    pub algorithm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_interval_hours: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ease_factor: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<i64>,
}

// ─── Template loader ──────────────────────────────────────────────────────────

/// List all templates for a universe root.
/// Always includes the synthetic "blank" template first.
/// Adds `_workspace.yaml` (slug="default") and any `_workspaces/*.yaml`.
pub fn list_templates(root: &Path) -> Vec<WorkspaceTemplate> {
    let mut templates = vec![WorkspaceTemplate::blank()];

    let default_path = root.join("_workspace.yaml");
    if default_path.exists()
        && let Some(mut tpl) = parse_template_file(&default_path)
    {
        tpl.slug = "default".into();
        templates.push(tpl);
    }

    let dir = root.join("_workspaces");
    if dir.is_dir() {
        let mut entries: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(e) => {
                warn!(path = %dir.display(), "workspace_templates: cannot read _workspaces dir: {e}");
                return templates;
            }
        };
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let slug = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            if let Some(mut tpl) = parse_template_file(&path) {
                tpl.slug = slug;
                templates.push(tpl);
            }
        }
    }

    templates
}

/// Load one template by slug from the given universe root.
pub fn load_template(root: &Path, slug: &str) -> Option<WorkspaceTemplate> {
    match slug {
        "blank" => Some(WorkspaceTemplate::blank()),
        "default" => {
            let mut tpl = parse_template_file(&root.join("_workspace.yaml"))?;
            tpl.slug = "default".into();
            Some(tpl)
        }
        _ => {
            let path = root.join("_workspaces").join(format!("{slug}.yaml"));
            let mut tpl = parse_template_file(&path)?;
            tpl.slug = slug.to_string();
            Some(tpl)
        }
    }
}

/// Parse a single YAML file. Returns `None` if the file is missing or invalid.
fn parse_template_file(path: &Path) -> Option<WorkspaceTemplate> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_yaml::from_str::<WorkspaceTemplate>(&raw) {
        Ok(tpl) => Some(tpl),
        Err(e) => {
            warn!(path = %path.display(), "workspace_template: invalid YAML — {e}");
            None
        }
    }
}

/// Resolve the content-root directory for a universe.
/// Prefers `local_repo_path` from the DB (CO-330); falls back to storage default.
pub(crate) fn resolve_universe_content_root(state: &AppState, universe_key: &str) -> PathBuf {
    let (local_repo_path, fallback) = {
        let storage = lock_storage(state);
        let path: Option<String> = storage
            .conn()
            .query_row(
                "SELECT local_repo_path FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let fallback = storage.universe_root(universe_key);
        (path, fallback)
    };

    if let Some(path_str) = local_repo_path {
        let expanded = expand_home(&path_str);
        if expanded.exists() {
            return expanded;
        }
    }
    fallback
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Build the seeded `layout_json` for a template.
/// Nodes whose `entry_path` is absent from the universe are dropped with a warning.
/// Edges referencing a dropped node are also dropped.
fn build_seeded_layout(
    state: &AppState,
    universe_key: &str,
    template: &WorkspaceTemplate,
) -> String {
    let Some(ref layout) = template.default_layout else {
        return r#"{"nodes":[],"edges":[]}"#.to_string();
    };
    if layout.nodes.is_empty() {
        return r#"{"nodes":[],"edges":[]}"#.to_string();
    }

    let existing: std::collections::HashSet<String> = {
        let storage = lock_storage(state);
        let conn = storage.conn();
        let mut stmt = match conn.prepare("SELECT path FROM entries WHERE universe_key = ?1") {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    universe_key,
                    "workspace_template: entries query failed: {e}"
                );
                return r#"{"nodes":[],"edges":[]}"#.to_string();
            }
        };
        stmt.query_map(params![universe_key], |row| row.get::<_, String>(0))
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    };

    let valid_nodes: Vec<&TemplateNode> = layout
        .nodes
        .iter()
        .filter(|n| {
            if existing.contains(&n.entry_path) {
                true
            } else {
                warn!(
                    universe_key,
                    entry_path = %n.entry_path,
                    "workspace_template: entry_path not found — dropping node"
                );
                false
            }
        })
        .collect();

    let valid_set: std::collections::HashSet<&str> =
        valid_nodes.iter().map(|n| n.entry_path.as_str()).collect();

    let valid_edges: Vec<&TemplateEdge> = layout
        .edges
        .iter()
        .filter(|e| valid_set.contains(e.from.as_str()) && valid_set.contains(e.to.as_str()))
        .collect();

    let out = serde_json::json!({
        "nodes": valid_nodes.iter().map(|n| serde_json::json!({
            "entry_path": n.entry_path,
            "x": n.x,
            "y": n.y,
        })).collect::<Vec<_>>(),
        "edges": valid_edges.iter().map(|e| serde_json::json!({
            "from": e.from,
            "to": e.to,
            "type": e.edge_type,
        })).collect::<Vec<_>>(),
    });

    serde_json::to_string(&out).unwrap_or_else(|_| r#"{"nodes":[],"edges":[]}"#.to_string())
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn list_workspace_templates_handler(
    State(state): State<AppState>,
    AxumPath(universe_slug): AxumPath<String>,
) -> impl IntoResponse {
    let root = resolve_universe_content_root(&state, &universe_slug);
    Json(list_templates(&root))
}

async fn get_workspace_template_handler(
    State(state): State<AppState>,
    AxumPath((universe_slug, template_slug)): AxumPath<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let root = resolve_universe_content_root(&state, &universe_slug);
    load_template(&root, &template_slug)
        .map(|t| Json(t).into_response())
        .ok_or_else(|| AppError::NotFound(format!("template '{template_slug}' not found")))
}

async fn create_from_template_handler(
    State(state): State<AppState>,
    AxumPath((universe_slug, workspace_slug, template_slug)): AxumPath<(String, String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let user_id = {
        let token = crate::auth::extract_bearer_or_cookie(&headers);
        let secret = crate::auth::jwt_secret();
        token
            .and_then(|t| crate::auth::decode_user_id(&t, &secret).ok())
            .unwrap_or_else(|| format!("anon:{}", uuid::Uuid::new_v4()))
    };

    let root = resolve_universe_content_root(&state, &universe_slug);
    let template = load_template(&root, &template_slug)
        .ok_or_else(|| AppError::NotFound(format!("template '{template_slug}' not found")))?;

    let layout_json = build_seeded_layout(&state, &universe_slug, &template);
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    {
        let storage = lock_storage(&state);
        storage
            .conn()
            .execute(
                "INSERT INTO workspace_states \
                 (id, universe_key, workspace_slug, user_id, layout_json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(universe_key, workspace_slug, user_id) DO UPDATE SET \
                   layout_json = excluded.layout_json, \
                   updated_at  = excluded.updated_at",
                params![
                    id,
                    universe_slug,
                    workspace_slug,
                    user_id,
                    layout_json,
                    now,
                    now
                ],
            )
            .map_err(|e| AppError::Internal(format!("workspace_states insert: {e}")))?;
    }

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "universe_key": universe_slug,
            "workspace_slug": workspace_slug,
            "user_id": user_id,
            "template_slug": template_slug,
            "layout_json": layout_json,
        })),
    ))
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{slug}/workspace-templates",
            get(list_workspace_templates_handler),
        )
        .route(
            "/{slug}/workspace-templates/{template_slug}",
            get(get_workspace_template_handler),
        )
        .route(
            "/{slug}/workspaces/{workspace_slug}/from-template/{template_slug}",
            post(create_from_template_handler),
        )
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, rel: &str, content: &str) {
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target, content).unwrap();
    }

    const DEFAULT_YAML: &str = "name: \"Padrão\"\ndescription: \"Template padrão.\"\n";

    const TPL_A: &str = "name: \"Básico\"\ndescription: \"Layout simples.\"\n\
default_layout:\n  nodes:\n    - entry_path: \"termos/a.md\"\n      x: 100.0\n      y: 200.0\n  edges: []\n";

    const TPL_B: &str =
        "name: \"Avançado\"\nstudy_mode:\n  algorithm: \"spaced-repetition\"\n  ease_factor: 2.5\n";

    #[test]
    fn list_templates_empty_root_gives_blank() {
        let dir = tempdir().unwrap();
        let tpls = list_templates(dir.path());
        assert_eq!(tpls.len(), 1);
        assert_eq!(tpls[0].slug, "blank");
    }

    #[test]
    fn list_templates_with_default_and_two_named() {
        let dir = tempdir().unwrap();
        write(dir.path(), "_workspace.yaml", DEFAULT_YAML);
        write(dir.path(), "_workspaces/tpl-a.yaml", TPL_A);
        write(dir.path(), "_workspaces/tpl-b.yaml", TPL_B);

        let tpls = list_templates(dir.path());
        assert_eq!(tpls.len(), 4); // blank + default + tpl-a + tpl-b

        let slugs: Vec<&str> = tpls.iter().map(|t| t.slug.as_str()).collect();
        assert!(slugs.contains(&"blank"));
        assert!(slugs.contains(&"default"));
        assert!(slugs.contains(&"tpl-a"));
        assert!(slugs.contains(&"tpl-b"));
    }

    #[test]
    fn load_template_blank_always_available() {
        let dir = tempdir().unwrap();
        let tpl = load_template(dir.path(), "blank").unwrap();
        assert_eq!(tpl.slug, "blank");
        assert_eq!(tpl.name, "Blank");
        assert!(tpl.default_layout.is_none());
    }

    #[test]
    fn load_template_default_parses_correctly() {
        let dir = tempdir().unwrap();
        write(dir.path(), "_workspace.yaml", DEFAULT_YAML);
        let tpl = load_template(dir.path(), "default").unwrap();
        assert_eq!(tpl.slug, "default");
        assert_eq!(tpl.name, "Padrão");
    }

    #[test]
    fn load_template_named_parses_layout() {
        let dir = tempdir().unwrap();
        write(dir.path(), "_workspaces/tpl-a.yaml", TPL_A);
        let tpl = load_template(dir.path(), "tpl-a").unwrap();
        assert_eq!(tpl.slug, "tpl-a");
        let layout = tpl.default_layout.unwrap();
        assert_eq!(layout.nodes.len(), 1);
        assert_eq!(layout.edges.len(), 0);
    }

    #[test]
    fn load_template_missing_returns_none() {
        let dir = tempdir().unwrap();
        assert!(load_template(dir.path(), "nonexistent").is_none());
    }

    #[test]
    fn invalid_yaml_rejected_with_warning() {
        let dir = tempdir().unwrap();
        write(dir.path(), "_workspace.yaml", "{ invalid: [yaml");
        let tpls = list_templates(dir.path());
        // only blank survives; invalid YAML dropped
        assert_eq!(tpls.len(), 1);
        assert_eq!(tpls[0].slug, "blank");
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "_workspaces/exotic.yaml",
            "name: \"Exotic\"\nunknown_field_xyz: \"some value\"\n",
        );
        let tpl = load_template(dir.path(), "exotic").unwrap();
        assert_eq!(tpl.name, "Exotic");
    }

    #[test]
    fn missing_required_name_rejected() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "_workspaces/no-name.yaml",
            "description: \"No name field here\"\n",
        );
        // `name` is required; serde fails → parse_template_file returns None
        assert!(load_template(dir.path(), "no-name").is_none());
    }
}
