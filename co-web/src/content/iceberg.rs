//! Iceberg-compliant metadata layer for universe content.
//!
//! Schema and catalog are committed in git (.co/catalog.json, .co/metadata/*.json).
//! Manifests are auto-generated on server startup by scanning all .md files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ---- Catalog ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub format_version: u32,
    pub universo: String,
    pub current_metadata: String,
    pub current_snapshot_id: Option<String>,
}

// ---- Schema ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaField {
    pub id: u32,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub schema_id: u32,
    pub fields: Vec<SchemaField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionField {
    pub source_id: u32,
    pub name: String,
    pub transform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub format_version: u32,
    pub schema_id: u32,
    pub schemas: Vec<Schema>,
    pub partition_spec: Vec<PartitionField>,
    #[serde(default)]
    pub properties: HashMap<String, String>,
}

// ---- Manifest ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub schema_id: u32,
    pub partition: HashMap<String, serde_json::Value>,
    pub stats: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub snapshot_id: Option<String>,
    pub entries: Vec<ManifestEntry>,
}

// ---- Reading ----

/// Load catalog from .co/catalog.json
pub fn load_catalog(universo_dir: &Path) -> Result<Catalog, String> {
    let path = universo_dir.join(".co/catalog.json");
    let content = fs::read_to_string(&path).map_err(|e| format!("Failed to read catalog: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse catalog: {e}"))
}

/// Load metadata from the path referenced in catalog
pub fn load_metadata(universo_dir: &Path, catalog: &Catalog) -> Result<Metadata, String> {
    let path = universo_dir.join(".co").join(&catalog.current_metadata);
    let content = fs::read_to_string(&path).map_err(|e| format!("Failed to read metadata: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse metadata: {e}"))
}

// ---- Manifest generation ----

/// Build a manifest by scanning all .md files in the universe directory.
/// This is called on server startup — not committed to git.
pub fn build_manifest(universo_dir: &Path, schema_id: u32) -> Manifest {
    let mut entries = Vec::new();

    let content_dirs = ["relatos", "eventos", "membros", "quadro", "jardim"];
    for dir_name in &content_dirs {
        let dir = universo_dir.join(dir_name);
        let dir_entries = fs::read_dir(&dir).into_iter().flatten().flatten();
        for entry in dir_entries {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(universo_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let (fm, body) = parse_frontmatter_simple(&content);

            // Derive partition values
            let content_type = fm
                .get("type")
                .cloned()
                .unwrap_or_else(|| infer_type_from_dir(dir_name));
            let data = fm
                .get("data")
                .or_else(|| fm.get("date"))
                .or_else(|| fm.get("criado"));
            let (year, month) = data
                .and_then(|d| parse_year_month(d))
                .unwrap_or((serde_json::Value::Null, serde_json::Value::Null));

            let mut partition = HashMap::new();
            partition.insert("type".to_string(), serde_json::Value::String(content_type));
            partition.insert("year".to_string(), year);
            partition.insert("month".to_string(), month);

            // Compute stats
            let word_count = body.split_whitespace().count();
            let mut stats = HashMap::new();
            stats.insert(
                "word_count".to_string(),
                serde_json::Value::Number(word_count.into()),
            );
            stats.insert(
                "has_body".to_string(),
                serde_json::Value::Bool(!body.trim().is_empty()),
            );
            if let Some(d) = data {
                stats.insert("created".to_string(), serde_json::Value::String(d.clone()));
            }

            entries.push(ManifestEntry {
                path: rel,
                schema_id,
                partition,
                stats,
            });
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Manifest {
        format_version: 1,
        snapshot_id: None,
        entries,
    }
}

fn infer_type_from_dir(dir: &str) -> String {
    match dir {
        "relatos" => "relato",
        "eventos" => "evento",
        "membros" => "membro",
        "quadro" => "missao",
        "jardim" => "nota",
        _ => "content",
    }
    .to_string()
}

fn parse_year_month(date_str: &str) -> Option<(serde_json::Value, serde_json::Value)> {
    let trimmed = date_str.trim().trim_matches('"');
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() >= 2 {
        let year: u32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        Some((
            serde_json::Value::Number(year.into()),
            serde_json::Value::Number(month.into()),
        ))
    } else {
        None
    }
}

/// Simple frontmatter parser (key: value) — reused from universo.rs logic.
fn parse_frontmatter_simple(content: &str) -> (HashMap<String, String>, String) {
    let mut fm = HashMap::new();
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (fm, content.to_string());
    }
    let after_opening = &trimmed[3..];
    if let Some(end_idx) = after_opening.find("\n---") {
        let fm_block = &after_opening[..end_idx];
        for line in fm_block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                fm.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        let body_start = 3 + end_idx + 4;
        let body = if body_start < trimmed.len() {
            trimmed[body_start..].trim_start_matches('\n').to_string()
        } else {
            String::new()
        };
        (fm, body)
    } else {
        (fm, content.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_build_manifest_scans_content() {
        let dir = tempdir().unwrap();

        // Create relatos/
        fs::create_dir_all(dir.path().join("relatos")).unwrap();
        fs::write(
            dir.path().join("relatos/post.md"),
            "---\ntype: relato\ntitulo: Test\ndata: 2026-03-28\n---\nBody here.\n",
        )
        .unwrap();

        // Create eventos/
        fs::create_dir_all(dir.path().join("eventos")).unwrap();
        fs::write(
            dir.path().join("eventos/ev.md"),
            "---\ntype: evento\ntitulo: Evento\ndata: 2026-04-01\nhora: 10:00\n---\nDesc.\n",
        )
        .unwrap();

        let manifest = build_manifest(dir.path(), 1);
        assert_eq!(manifest.entries.len(), 2);

        let relato = manifest
            .entries
            .iter()
            .find(|e| e.path.contains("relatos"))
            .unwrap();
        assert_eq!(
            relato.partition.get("type").unwrap(),
            &serde_json::Value::String("relato".to_string())
        );
        assert_eq!(
            relato.partition.get("year").unwrap(),
            &serde_json::Value::Number(2026.into())
        );
        assert_eq!(
            relato.partition.get("month").unwrap(),
            &serde_json::Value::Number(3.into())
        );
        assert_eq!(
            relato.stats.get("word_count").unwrap(),
            &serde_json::Value::Number(2.into())
        );
    }

    #[test]
    fn test_load_catalog() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".co")).unwrap();
        fs::write(
            dir.path().join(".co/catalog.json"),
            r#"{"format_version":1,"universo":"test","current_metadata":"metadata/v1.json","current_snapshot_id":null}"#,
        )
        .unwrap();

        let catalog = load_catalog(dir.path()).unwrap();
        assert_eq!(catalog.universo, "test");
        assert_eq!(catalog.current_metadata, "metadata/v1.json");
    }
}
