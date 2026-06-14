//! CO-353 — Sala realtime lobby: shared room state for workspace canvases.
//!
//! A [`SalaLobby`] holds one [`Room`] per `(universe_key, workspace_slug)`. Every
//! connected client of a workspace shares that room: cursor positions, node/edge
//! mutations and suggest/publish notifications are broadcast to the whole room and
//! the authoritative layout is kept in memory (server-as-arbiter, last-write-wins).
//!
//! Persistence reuses CO-352's `workspace_states` table via an internal storage
//! call (not HTTP): the room flushes its layout under the synthetic shared user
//! [`SHARED_USER`] so it survives reconnects and full server restarts.
//!
//! The lobby lives on [`crate::server::CoreState`] (`sala_lobby`). Locks here are
//! `parking_lot::Mutex` and are NEVER held across an `.await`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Capacity of the per-room broadcast ring. Lagging subscribers receive a
/// `RecvError::Lagged` that closes their connection (they reconnect + re-snapshot).
pub const BROADCAST_CAPACITY: usize = 512;

/// Minimum interval between forwarded cursor frames per connection (~20 Hz).
pub const CURSOR_MIN_INTERVAL_MS: u64 = 50;

/// Synthetic user id the shared room layout is persisted under in
/// `workspace_states`. Distinct from any real user id so per-user saves
/// (CO-352) and the shared realtime layout never collide.
pub const SHARED_USER: &str = "__sala_shared__";

/// `"{universe_key}/{workspace_slug}"` — the room key.
pub type WorkspaceId = String;

/// Build a [`WorkspaceId`] from its parts.
pub fn workspace_id(universe_key: &str, workspace_slug: &str) -> WorkspaceId {
    format!("{universe_key}/{workspace_slug}")
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

/// A user visible in the room roster.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PresenceUser {
    pub user_id: String,
    pub name: String,
    /// `#rrggbb` deterministic colour hashed from `user_id`.
    pub color: String,
    /// True for read-only visitors (no persisted writes).
    pub anon: bool,
}

/// Deterministic `#rrggbb` colour from a stable id. Anonymous users get a
/// desaturated variant so visitors read as muted next to signed-in authors.
pub fn color_for(user_id: &str, anon: bool) -> String {
    // FNV-1a — small, stable, no deps.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in user_id.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let hue = (hash % 360) as f32;
    let (sat, light) = if anon { (0.20, 0.62) } else { (0.65, 0.55) };
    hsl_to_hex(hue, sat, light)
}

fn hsl_to_hex(h: f32, s: f32, l: f32) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", to_byte(r1), to_byte(g1), to_byte(b1))
}

// ---------------------------------------------------------------------------
// Wire events (server → client) and client → server messages
// ---------------------------------------------------------------------------

/// Server → client events. Serialised to JSON with a `type` tag.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SalaEvent {
    #[serde(rename = "snapshot")]
    Snapshot {
        state: serde_json::Value,
        users: Vec<PresenceUser>,
    },
    #[serde(rename = "cursor")]
    Cursor { user_id: String, x: f32, y: f32 },
    #[serde(rename = "node_move")]
    NodeMove { entry_path: String, x: f32, y: f32 },
    #[serde(rename = "node_add")]
    NodeAdd { entry_path: String, x: f32, y: f32 },
    #[serde(rename = "node_remove")]
    NodeRemove { entry_path: String },
    #[serde(rename = "edge_add")]
    EdgeAdd {
        from: String,
        to: String,
        edge_type: String,
    },
    #[serde(rename = "edge_remove")]
    EdgeRemove { from: String, to: String },
    #[serde(rename = "suggest")]
    Suggest { entry_id: String, status: String },
    #[serde(rename = "publish")]
    Publish { entry_id: String },
    #[serde(rename = "user_join")]
    UserJoin { user: PresenceUser },
    #[serde(rename = "user_leave")]
    UserLeave { user_id: String },
    /// Server rejected a client op (e.g. anonymous write). The originating
    /// client matches `op_id` and rolls its optimistic change back.
    #[serde(rename = "revert")]
    Revert { op_id: String },
}

/// Client → server messages.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ClientMsg {
    #[serde(rename = "cursor")]
    Cursor { x: f32, y: f32 },
    #[serde(rename = "node_move")]
    NodeMove {
        entry_path: String,
        x: f32,
        y: f32,
        #[serde(default)]
        op_id: Option<String>,
    },
    #[serde(rename = "node_add")]
    NodeAdd {
        entry_path: String,
        x: f32,
        y: f32,
        #[serde(default)]
        op_id: Option<String>,
    },
    #[serde(rename = "node_remove")]
    NodeRemove {
        entry_path: String,
        #[serde(default)]
        op_id: Option<String>,
    },
    #[serde(rename = "edge_add")]
    EdgeAdd {
        from: String,
        to: String,
        #[serde(default)]
        edge_type: String,
        #[serde(default)]
        op_id: Option<String>,
    },
    #[serde(rename = "edge_remove")]
    EdgeRemove {
        from: String,
        to: String,
        #[serde(default)]
        op_id: Option<String>,
    },
    #[serde(rename = "suggest")]
    Suggest {
        entry_id: String,
        #[serde(default)]
        status: String,
    },
    #[serde(rename = "publish")]
    Publish { entry_id: String },
    /// Re-request the full snapshot (e.g. after a reconnect).
    #[serde(rename = "snapshot")]
    Snapshot,
    #[serde(rename = "ping")]
    Ping,
}

impl ClientMsg {
    /// True for messages that mutate persisted room state. These are rejected
    /// for anonymous (read-only) connections. Cursor + ping + snapshot are not
    /// writes (cursors are ephemeral presence, allowed for visitors).
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            ClientMsg::NodeMove { .. }
                | ClientMsg::NodeAdd { .. }
                | ClientMsg::NodeRemove { .. }
                | ClientMsg::EdgeAdd { .. }
                | ClientMsg::EdgeRemove { .. }
                | ClientMsg::Suggest { .. }
                | ClientMsg::Publish { .. }
        )
    }

    /// The `op_id` carried by mutating messages (for revert addressing).
    pub fn op_id(&self) -> Option<&str> {
        match self {
            ClientMsg::NodeMove { op_id, .. }
            | ClientMsg::NodeAdd { op_id, .. }
            | ClientMsg::NodeRemove { op_id, .. }
            | ClientMsg::EdgeAdd { op_id, .. }
            | ClientMsg::EdgeRemove { op_id, .. } => op_id.as_deref(),
            _ => None,
        }
    }

    /// Translate a relayed client op into the server→client event broadcast to
    /// the rest of the room. Non-relayed messages (`Ping`, `Snapshot`, `Cursor`
    /// is handled separately) return `None`.
    pub fn into_broadcast(self, user_id: &str) -> Option<SalaEvent> {
        match self {
            ClientMsg::Cursor { x, y } => Some(SalaEvent::Cursor {
                user_id: user_id.to_string(),
                x,
                y,
            }),
            ClientMsg::NodeMove {
                entry_path, x, y, ..
            } => Some(SalaEvent::NodeMove { entry_path, x, y }),
            ClientMsg::NodeAdd {
                entry_path, x, y, ..
            } => Some(SalaEvent::NodeAdd { entry_path, x, y }),
            ClientMsg::NodeRemove { entry_path, .. } => Some(SalaEvent::NodeRemove { entry_path }),
            ClientMsg::EdgeAdd {
                from,
                to,
                edge_type,
                ..
            } => Some(SalaEvent::EdgeAdd {
                from,
                to,
                edge_type,
            }),
            ClientMsg::EdgeRemove { from, to, .. } => Some(SalaEvent::EdgeRemove { from, to }),
            ClientMsg::Suggest { entry_id, status } => {
                Some(SalaEvent::Suggest { entry_id, status })
            }
            ClientMsg::Publish { entry_id } => Some(SalaEvent::Publish { entry_id }),
            ClientMsg::Snapshot | ClientMsg::Ping => None,
        }
    }
}

/// Throttle decision for an outbound cursor frame. Pure so it is unit-testable.
/// Returns `true` (and the new "last" timestamp should become `now_ms`) when at
/// least `min_interval_ms` has elapsed since the last forwarded frame.
pub fn cursor_allowed(last_ms: Option<u64>, now_ms: u64, min_interval_ms: u64) -> bool {
    match last_ms {
        None => true,
        Some(last) => now_ms.saturating_sub(last) >= min_interval_ms,
    }
}

// ---------------------------------------------------------------------------
// Broadcast envelope
// ---------------------------------------------------------------------------

/// A pre-serialised event fanned out to a room. `origin` is the connection id
/// that produced it (`0` = server). The writer skips frames whose `origin`
/// matches its own connection so a client never receives an echo of its own op
/// (it already applied it optimistically).
#[derive(Clone, Debug)]
pub struct RoomMsg {
    pub origin: u64,
    pub json: Arc<str>,
}

impl RoomMsg {
    pub fn server(event: &SalaEvent) -> Self {
        Self {
            origin: 0,
            json: serde_json::to_string(event).unwrap_or_default().into(),
        }
    }
    pub fn from_conn(origin: u64, event: &SalaEvent) -> Self {
        Self {
            origin,
            json: serde_json::to_string(event).unwrap_or_default().into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Room
// ---------------------------------------------------------------------------

/// Mutable per-room state guarded by [`Room::inner`].
#[derive(Debug)]
pub struct RoomInner {
    /// Authoritative layout — `{nodes:[...], edges:[...], ...}`.
    pub layout: serde_json::Value,
    /// Live connections: connection id → its presence entry.
    pub connections: HashMap<u64, PresenceUser>,
    /// Set when `layout` diverged from what is persisted.
    pub dirty: bool,
}

impl RoomInner {
    /// Roster of distinct users (deduped by `user_id`, first connection wins).
    pub fn roster(&self) -> Vec<PresenceUser> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for u in self.connections.values() {
            if seen.insert(u.user_id.clone()) {
                out.push(u.clone());
            }
        }
        out
    }

    /// Number of connections currently held by `user_id`.
    pub fn conn_count_for(&self, user_id: &str) -> usize {
        self.connections
            .values()
            .filter(|u| u.user_id == user_id)
            .count()
    }

    /// Apply a relayed op to the authoritative layout (last-write-wins).
    /// Cursor/suggest/publish events don't touch persisted layout. Returns
    /// whether the layout changed (to drive the `dirty` flag).
    pub fn apply(&mut self, event: &SalaEvent) -> bool {
        match event {
            SalaEvent::NodeMove { entry_path, x, y } | SalaEvent::NodeAdd { entry_path, x, y } => {
                upsert_node(&mut self.layout, entry_path, *x, *y);
                true
            }
            SalaEvent::NodeRemove { entry_path } => remove_node(&mut self.layout, entry_path),
            SalaEvent::EdgeAdd {
                from,
                to,
                edge_type,
            } => add_edge(&mut self.layout, from, to, edge_type),
            SalaEvent::EdgeRemove { from, to } => remove_edge(&mut self.layout, from, to),
            _ => false,
        }
    }
}

fn nodes_array(layout: &mut serde_json::Value) -> &mut Vec<serde_json::Value> {
    let obj = layout
        .as_object_mut()
        .expect("layout must be a JSON object");
    obj.entry("nodes")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.get_mut("nodes")
        .and_then(|v| v.as_array_mut())
        .expect("nodes must be an array")
}

fn edges_array(layout: &mut serde_json::Value) -> &mut Vec<serde_json::Value> {
    let obj = layout
        .as_object_mut()
        .expect("layout must be a JSON object");
    obj.entry("edges")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.get_mut("edges")
        .and_then(|v| v.as_array_mut())
        .expect("edges must be an array")
}

fn upsert_node(layout: &mut serde_json::Value, entry_path: &str, x: f32, y: f32) {
    let nodes = nodes_array(layout);
    if let Some(node) = nodes
        .iter_mut()
        .find(|n| n.get("entry_path").and_then(|v| v.as_str()) == Some(entry_path))
    {
        node["x"] = serde_json::json!(x);
        node["y"] = serde_json::json!(y);
    } else {
        nodes.push(serde_json::json!({ "entry_path": entry_path, "x": x, "y": y }));
    }
}

fn remove_node(layout: &mut serde_json::Value, entry_path: &str) -> bool {
    let nodes = nodes_array(layout);
    let before = nodes.len();
    nodes.retain(|n| n.get("entry_path").and_then(|v| v.as_str()) != Some(entry_path));
    nodes.len() != before
}

fn add_edge(layout: &mut serde_json::Value, from: &str, to: &str, edge_type: &str) -> bool {
    let edges = edges_array(layout);
    let exists = edges.iter().any(|e| {
        e.get("from").and_then(|v| v.as_str()) == Some(from)
            && e.get("to").and_then(|v| v.as_str()) == Some(to)
    });
    if exists {
        return false;
    }
    edges.push(serde_json::json!({ "from": from, "to": to, "type": edge_type }));
    true
}

fn remove_edge(layout: &mut serde_json::Value, from: &str, to: &str) -> bool {
    let edges = edges_array(layout);
    let before = edges.len();
    edges.retain(|e| {
        !(e.get("from").and_then(|v| v.as_str()) == Some(from)
            && e.get("to").and_then(|v| v.as_str()) == Some(to))
    });
    edges.len() != before
}

/// A shared workspace room.
#[derive(Debug)]
pub struct Room {
    pub tx: broadcast::Sender<RoomMsg>,
    pub inner: Mutex<RoomInner>,
}

impl Room {
    fn new(layout: serde_json::Value) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            inner: Mutex::new(RoomInner {
                layout,
                connections: HashMap::new(),
                dirty: false,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Lobby
// ---------------------------------------------------------------------------

/// All live rooms, keyed by [`WorkspaceId`].
#[derive(Default)]
pub struct SalaLobby {
    rooms: Mutex<HashMap<WorkspaceId, Arc<Room>>>,
    next_conn: AtomicU64,
}

impl std::fmt::Debug for SalaLobby {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SalaLobby")
            .field("rooms", &self.rooms.lock().len())
            .finish()
    }
}

impl SalaLobby {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh, non-zero connection id (`0` is reserved for the server).
    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Get the room for `id`, creating it with `initial_layout` if absent.
    /// `initial_layout` is only used on creation (the caller loads it from
    /// storage); for an existing room the live layout wins.
    pub fn get_or_create(&self, id: &str, initial_layout: serde_json::Value) -> Arc<Room> {
        let mut rooms = self.rooms.lock();
        rooms
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Room::new(initial_layout)))
            .clone()
    }

    /// Existing room for `id`, if any (does not create).
    pub fn get(&self, id: &str) -> Option<Arc<Room>> {
        self.rooms.lock().get(id).cloned()
    }

    /// Drop a room (called when its last connection leaves).
    pub fn remove(&self, id: &str) {
        self.rooms.lock().remove(id);
    }

    /// Number of live rooms (diagnostics / tests).
    pub fn room_count(&self) -> usize {
        self.rooms.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_is_deterministic_and_hex() {
        let a = color_for("user-1", false);
        let b = color_for("user-1", false);
        assert_eq!(a, b);
        assert_eq!(a.len(), 7);
        assert!(a.starts_with('#'));
        // Different users get different colours (with overwhelming probability).
        assert_ne!(color_for("user-1", false), color_for("user-2", false));
        // Anonymous variant differs from the signed-in one.
        assert_ne!(color_for("user-1", false), color_for("user-1", true));
    }

    #[test]
    fn cursor_throttle_enforces_min_interval() {
        assert!(cursor_allowed(None, 0, 50));
        assert!(!cursor_allowed(Some(0), 10, 50));
        assert!(!cursor_allowed(Some(0), 49, 50));
        assert!(cursor_allowed(Some(0), 50, 50));
        assert!(cursor_allowed(Some(0), 1000, 50));
    }

    #[test]
    fn is_write_classifies_messages() {
        assert!(
            ClientMsg::NodeMove {
                entry_path: "a".into(),
                x: 0.0,
                y: 0.0,
                op_id: None
            }
            .is_write()
        );
        assert!(
            ClientMsg::Publish {
                entry_id: "e".into()
            }
            .is_write()
        );
        assert!(!ClientMsg::Cursor { x: 1.0, y: 2.0 }.is_write());
        assert!(!ClientMsg::Ping.is_write());
        assert!(!ClientMsg::Snapshot.is_write());
    }

    #[test]
    fn lww_node_move_upserts_then_overwrites() {
        let mut inner = RoomInner {
            layout: serde_json::json!({"nodes": [], "edges": []}),
            connections: HashMap::new(),
            dirty: false,
        };
        assert!(inner.apply(&SalaEvent::NodeAdd {
            entry_path: "notes/a.md".into(),
            x: 1.0,
            y: 2.0
        }));
        // Last write wins: a second move overwrites the same node in place.
        assert!(inner.apply(&SalaEvent::NodeMove {
            entry_path: "notes/a.md".into(),
            x: 9.0,
            y: 9.0
        }));
        let nodes = inner.layout["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["x"], serde_json::json!(9.0));
        assert_eq!(nodes[0]["y"], serde_json::json!(9.0));
    }

    #[test]
    fn lww_node_remove_and_edges() {
        let mut inner = RoomInner {
            layout: serde_json::json!({"nodes": [], "edges": []}),
            connections: HashMap::new(),
            dirty: false,
        };
        inner.apply(&SalaEvent::NodeAdd {
            entry_path: "a".into(),
            x: 0.0,
            y: 0.0,
        });
        assert!(inner.apply(&SalaEvent::NodeRemove {
            entry_path: "a".into()
        }));
        // Removing a missing node is a no-op (no change).
        assert!(!inner.apply(&SalaEvent::NodeRemove {
            entry_path: "a".into()
        }));

        assert!(inner.apply(&SalaEvent::EdgeAdd {
            from: "a".into(),
            to: "b".into(),
            edge_type: "ref".into()
        }));
        // Duplicate edge is ignored.
        assert!(!inner.apply(&SalaEvent::EdgeAdd {
            from: "a".into(),
            to: "b".into(),
            edge_type: "ref".into()
        }));
        assert!(inner.apply(&SalaEvent::EdgeRemove {
            from: "a".into(),
            to: "b".into()
        }));
    }

    #[test]
    fn roster_dedupes_by_user_and_counts_connections() {
        let mut inner = RoomInner {
            layout: serde_json::json!({}),
            connections: HashMap::new(),
            dirty: false,
        };
        let u = |id: &str| PresenceUser {
            user_id: id.into(),
            name: id.into(),
            color: "#000000".into(),
            anon: false,
        };
        inner.connections.insert(1, u("alice"));
        inner.connections.insert(2, u("alice")); // second tab
        inner.connections.insert(3, u("bob"));
        assert_eq!(inner.roster().len(), 2);
        assert_eq!(inner.conn_count_for("alice"), 2);
        assert_eq!(inner.conn_count_for("bob"), 1);
    }

    #[test]
    fn get_or_create_is_idempotent_per_id() {
        let lobby = SalaLobby::new();
        let id = workspace_id("mbya", "default");
        let r1 = lobby.get_or_create(&id, serde_json::json!({"nodes": []}));
        // Second call with a different initial layout returns the SAME room
        // (live layout wins; initial is ignored once the room exists).
        let r2 = lobby.get_or_create(&id, serde_json::json!({"nodes": [{"entry_path": "x"}]}));
        assert!(Arc::ptr_eq(&r1, &r2));
        assert_eq!(lobby.room_count(), 1);
        assert!(
            r2.inner.lock().layout["nodes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        lobby.remove(&id);
        assert_eq!(lobby.room_count(), 0);
    }

    /// CO-454: presence is per-sub-sala. A folder-path slug (`default/jardim`)
    /// yields a distinct room key, so a pasta's sub-sala has its own roster —
    /// 1:1 with a Yggdrasil `/mundo` room (YG-146). `workspace_id` accepts the
    /// `/` in the slug because the room key is just an opaque string.
    #[test]
    fn workspace_id_accepts_folder_path_slug() {
        let parent = workspace_id("template", "default");
        let child = workspace_id("template", "default/jardim");
        let grandchild = workspace_id("template", "default/jardim/estufa");
        assert_eq!(child, "template/default/jardim");
        assert_eq!(grandchild, "template/default/jardim/estufa");
        assert_ne!(parent, child);
        assert_ne!(child, grandchild);

        let lobby = SalaLobby::new();
        lobby.get_or_create(&parent, serde_json::json!({"nodes": []}));
        lobby.get_or_create(&child, serde_json::json!({"nodes": []}));
        lobby.get_or_create(&grandchild, serde_json::json!({"nodes": []}));
        assert_eq!(
            lobby.room_count(),
            3,
            "parent sala + folder-sub-sala + nested sub-sala are independent rooms"
        );
    }

    #[test]
    fn conn_ids_are_unique_and_nonzero() {
        let lobby = SalaLobby::new();
        let a = lobby.next_conn_id();
        let b = lobby.next_conn_id();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }
}
