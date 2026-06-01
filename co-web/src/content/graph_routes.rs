//! CO-335: Universe graph API.
//!
//! GET /api/v1/universes/:slug/graph
//!   ?include_types=type1,type2   — filter by entry type (all if absent)
//!   ?relation=rel1,rel2          — filter by relation kind (all if absent)
//!   ?root=<path>                 — BFS root (full graph if absent)
//!   ?max_depth=3                 — BFS hop limit (default 3; ignored without root)
//!   ?published_only=true         — filter to published entries only
//!
//! Returns { nodes: [...], edges: [...] } where each node is a universe entry
//! and each edge is an entry_relations row with both endpoints in the node set.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::AppError;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Request params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    /// Comma-separated entry types to include (all types if absent).
    pub include_types: Option<String>,
    /// Comma-separated relation kinds to include (all if absent).
    pub relation: Option<String>,
    /// BFS root entry path; when set, only entries reachable from here are returned.
    pub root: Option<String>,
    /// Maximum BFS hops from root (default 3; ignored when root is absent).
    pub max_depth: Option<usize>,
    /// When `true`, only entries with `published: true` frontmatter field are returned.
    pub published_only: Option<bool>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub title: Option<String>,
    pub frontmatter: JsonValue,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn get_graph(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<GraphQuery>,
) -> Result<Json<GraphResponse>, AppError> {
    let include_types: Option<HashSet<String>> = q.include_types.as_ref().map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });
    let filter_relations: Option<HashSet<String>> = q.relation.as_ref().map(|s| {
        s.split(',')
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect()
    });
    let max_depth = q.max_depth.unwrap_or(3);
    let published_only = q.published_only.unwrap_or(false);

    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(&slug)
    };
    let conn = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;

    // ── 1. Load all matching entries into a HashMap ─────────────────────────
    let mut all_nodes: HashMap<String, GraphNode> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT path, entry_type, title, frontmatter_json \
             FROM entries WHERE universe_key = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map(params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        for row in rows {
            let (path, entry_type, title, fm_json) = row?;

            if include_types
                .as_ref()
                .is_some_and(|types| !types.contains(&entry_type))
            {
                continue;
            }

            let frontmatter: JsonValue =
                serde_json::from_str(&fm_json).unwrap_or(JsonValue::Object(Default::default()));

            if published_only
                && frontmatter.get("published").and_then(|v| v.as_bool()) != Some(true)
            {
                continue;
            }

            all_nodes.insert(
                path.clone(),
                GraphNode {
                    id: path,
                    node_type: entry_type,
                    title,
                    frontmatter,
                },
            );
        }
    }

    // ── 2. Load all same-universe relations where both endpoints are nodes ───
    let all_edges: Vec<GraphEdge> = {
        let mut stmt = conn.prepare(
            "SELECT from_path, to_path, relation_type \
             FROM entry_relations \
             WHERE universe_key = ?1 AND to_universe IS NULL \
             ORDER BY from_path, to_path",
        )?;
        let rows = stmt.query_map(params![slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        rows.filter_map(|r| r.ok())
            .filter(|(from, to, kind)| {
                all_nodes.contains_key(from.as_str())
                    && all_nodes.contains_key(to.as_str())
                    && filter_relations
                        .as_ref()
                        .is_none_or(|f| f.contains(kind.as_str()))
            })
            .map(|(from, to, kind)| GraphEdge {
                source: from,
                target: to,
                kind,
            })
            .collect()
    };

    // ── 3. BFS traversal when a root entry is given ─────────────────────────
    let (nodes, edges) = if let Some(root) = q.root.as_deref() {
        // Build undirected adjacency for BFS
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in &all_edges {
            adj.entry(e.source.as_str())
                .or_default()
                .push(e.target.as_str());
            adj.entry(e.target.as_str())
                .or_default()
                .push(e.source.as_str());
        }

        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        if all_nodes.contains_key(root) {
            queue.push_back((root.to_string(), 0));
            visited.insert(root.to_string());
        }
        while let Some((cur, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            if let Some(neighbors) = adj.get(cur.as_str()) {
                for &nb in neighbors {
                    if !visited.contains(nb) {
                        visited.insert(nb.to_string());
                        queue.push_back((nb.to_string(), depth + 1));
                    }
                }
            }
        }

        let nodes: Vec<GraphNode> = visited
            .iter()
            .filter_map(|id| all_nodes.get(id).cloned())
            .collect();
        let edges: Vec<GraphEdge> = all_edges
            .into_iter()
            .filter(|e| visited.contains(&e.source) && visited.contains(&e.target))
            .collect();
        (nodes, edges)
    } else {
        (all_nodes.into_values().collect(), all_edges)
    };

    Ok(Json(GraphResponse { nodes, edges }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new().route("/{slug}/graph", get(get_graph))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE entries (
                path TEXT NOT NULL,
                universe_key TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                title TEXT,
                frontmatter_json TEXT NOT NULL DEFAULT '{}',
                payload TEXT NOT NULL DEFAULT '{}',
                body TEXT NOT NULL DEFAULT '',
                body_hash TEXT NOT NULL DEFAULT '',
                created_at TEXT,
                updated_at TEXT,
                PRIMARY KEY (universe_key, path)
            );
            CREATE TABLE entry_relations (
                universe_key  TEXT NOT NULL,
                from_path     TEXT NOT NULL,
                to_path       TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                created_at    TEXT NOT NULL,
                to_universe   TEXT,
                PRIMARY KEY (universe_key, from_path, to_path, relation_type)
            );",
        )
        .unwrap();
        conn
    }

    fn insert_entry(
        conn: &Connection,
        universe: &str,
        path: &str,
        entry_type: &str,
        title: &str,
        fm: serde_json::Value,
    ) {
        conn.execute(
            "INSERT INTO entries \
             (path, universe_key, entry_type, title, frontmatter_json, payload, body_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, '{}', 'x')",
            params![path, universe, entry_type, title, fm.to_string()],
        )
        .unwrap();
    }

    fn insert_relation(conn: &Connection, universe: &str, from: &str, to: &str, kind: &str) {
        conn.execute(
            "INSERT INTO entry_relations \
             (universe_key, from_path, to_path, relation_type, created_at) \
             VALUES (?1, ?2, ?3, ?4, '2026-01-01')",
            params![universe, from, to, kind],
        )
        .unwrap();
    }

    // Helper: build all_nodes from DB
    fn load_nodes(
        conn: &Connection,
        universe: &str,
        include_types: Option<&HashSet<String>>,
        published_only: bool,
    ) -> HashMap<String, GraphNode> {
        let mut stmt = conn
            .prepare(
                "SELECT path, entry_type, title, frontmatter_json \
                 FROM entries WHERE universe_key = ?1 ORDER BY path",
            )
            .unwrap();
        let rows: Vec<_> = stmt
            .query_map(params![universe], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        let mut map = HashMap::new();
        for (path, entry_type, title, fm_json) in rows {
            if let Some(types) = include_types {
                if !types.contains(&entry_type) {
                    continue;
                }
            }
            let frontmatter: serde_json::Value =
                serde_json::from_str(&fm_json).unwrap_or(json!({}));
            if published_only
                && frontmatter.get("published").and_then(|v| v.as_bool()) != Some(true)
            {
                continue;
            }
            map.insert(
                path.clone(),
                GraphNode {
                    id: path,
                    node_type: entry_type,
                    title,
                    frontmatter,
                },
            );
        }
        map
    }

    // Helper: build edges from DB
    fn load_edges(
        conn: &Connection,
        universe: &str,
        all_nodes: &HashMap<String, GraphNode>,
        filter_relations: Option<&HashSet<String>>,
    ) -> Vec<GraphEdge> {
        let mut stmt = conn
            .prepare(
                "SELECT from_path, to_path, relation_type \
                 FROM entry_relations \
                 WHERE universe_key = ?1 AND to_universe IS NULL",
            )
            .unwrap();
        stmt.query_map(params![universe], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .filter(|(from, to, kind)| {
            all_nodes.contains_key(from.as_str())
                && all_nodes.contains_key(to.as_str())
                && filter_relations.map_or(true, |f| f.contains(kind.as_str()))
        })
        .map(|(from, to, kind)| GraphEdge {
            source: from,
            target: to,
            kind,
        })
        .collect()
    }

    #[test]
    fn test_full_graph_returns_all_entries_and_edges() {
        let conn = setup_db();
        insert_entry(&conn, "u1", "a.md", "song", "Song A", json!({}));
        insert_entry(&conn, "u1", "b.md", "song", "Song B", json!({}));
        insert_entry(&conn, "u1", "c.md", "nota", "Nota C", json!({}));
        insert_relation(&conn, "u1", "a.md", "b.md", "references");

        let nodes = load_nodes(&conn, "u1", None, false);
        assert_eq!(nodes.len(), 3);

        let edges = load_edges(&conn, "u1", &nodes, None);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source, "a.md");
        assert_eq!(edges[0].target, "b.md");
        assert_eq!(edges[0].kind, "references");
    }

    #[test]
    fn test_type_filter_excludes_other_types() {
        let conn = setup_db();
        insert_entry(&conn, "u1", "a.md", "song", "Song A", json!({}));
        insert_entry(&conn, "u1", "b.md", "nota", "Nota B", json!({}));

        let types: HashSet<String> = ["song".to_string()].into_iter().collect();
        let nodes = load_nodes(&conn, "u1", Some(&types), false);
        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains_key("a.md"));
        assert!(!nodes.contains_key("b.md"));
    }

    #[test]
    fn test_published_filter_only_returns_published() {
        let conn = setup_db();
        insert_entry(
            &conn,
            "u1",
            "pub.md",
            "nota",
            "Pub",
            json!({"published": true}),
        );
        insert_entry(
            &conn,
            "u1",
            "priv.md",
            "nota",
            "Priv",
            json!({"published": false}),
        );
        insert_entry(&conn, "u1", "none.md", "nota", "None", json!({}));

        let nodes = load_nodes(&conn, "u1", None, true);
        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains_key("pub.md"));
    }

    #[test]
    fn test_relation_filter_excludes_other_kinds() {
        let conn = setup_db();
        insert_entry(&conn, "u1", "a.md", "nota", "A", json!({}));
        insert_entry(&conn, "u1", "b.md", "nota", "B", json!({}));
        insert_relation(&conn, "u1", "a.md", "b.md", "references");
        insert_relation(&conn, "u1", "b.md", "a.md", "attendees");

        let nodes = load_nodes(&conn, "u1", None, false);
        let filter: HashSet<String> = ["references".to_string()].into_iter().collect();
        let edges = load_edges(&conn, "u1", &nodes, Some(&filter));
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, "references");
    }

    #[test]
    fn test_bfs_depth_1_excludes_two_hop_nodes() {
        // a → b → c; d isolated
        // BFS from a at max_depth=1 reaches a and b, not c or d
        let edges = vec![
            GraphEdge {
                source: "a".into(),
                target: "b".into(),
                kind: "references".into(),
            },
            GraphEdge {
                source: "b".into(),
                target: "c".into(),
                kind: "references".into(),
            },
        ];
        let all_nodes: HashMap<String, GraphNode> = ["a", "b", "c", "d"]
            .iter()
            .map(|&id| {
                (
                    id.to_string(),
                    GraphNode {
                        id: id.into(),
                        node_type: "nota".into(),
                        title: None,
                        frontmatter: json!({}),
                    },
                )
            })
            .collect();

        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in &edges {
            adj.entry(e.source.as_str())
                .or_default()
                .push(e.target.as_str());
            adj.entry(e.target.as_str())
                .or_default()
                .push(e.source.as_str());
        }

        let root = "a";
        let max_depth = 1usize;
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((root.into(), 0));
        visited.insert(root.into());
        while let Some((cur, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            if let Some(neighbors) = adj.get(cur.as_str()) {
                for &nb in neighbors {
                    if !visited.contains(nb) {
                        visited.insert(nb.into());
                        queue.push_back((nb.into(), depth + 1));
                    }
                }
            }
        }

        assert!(visited.contains("a"), "root must be in visited");
        assert!(visited.contains("b"), "1-hop neighbor must be in visited");
        assert!(
            !visited.contains("c"),
            "2-hop node must not be visited at depth=1"
        );
        assert!(!visited.contains("d"), "isolated node must not be visited");

        let result_nodes: Vec<_> = visited.iter().filter_map(|id| all_nodes.get(id)).collect();
        assert_eq!(result_nodes.len(), 2);
    }

    #[test]
    fn test_bfs_unknown_root_returns_empty() {
        let all_nodes: HashMap<String, GraphNode> = [(
            "a".to_string(),
            GraphNode {
                id: "a".into(),
                node_type: "nota".into(),
                title: None,
                frontmatter: json!({}),
            },
        )]
        .into_iter()
        .collect();

        // Root not in node set → BFS queue never starts
        let root = "does_not_exist";
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        if all_nodes.contains_key(root) {
            queue.push_back((root.into(), 0));
            visited.insert(root.into());
        }

        assert!(
            visited.is_empty(),
            "unknown root must produce empty visited set"
        );
    }

    #[test]
    fn test_cross_universe_edges_excluded_from_graph() {
        let conn = setup_db();
        insert_entry(&conn, "u1", "a.md", "nota", "A", json!({}));
        insert_entry(&conn, "u1", "b.md", "nota", "B", json!({}));

        // Cross-universe edge (to_universe IS NOT NULL)
        conn.execute(
            "INSERT INTO entry_relations \
             (universe_key, from_path, to_path, relation_type, created_at, to_universe) \
             VALUES ('u1', 'a.md', 'b.md', 'references', '2026-01-01', 'other')",
            [],
        )
        .unwrap();

        let nodes = load_nodes(&conn, "u1", None, false);
        let edges = load_edges(&conn, "u1", &nodes, None);
        assert!(
            edges.is_empty(),
            "cross-universe edges must not appear in graph response"
        );
    }
}
