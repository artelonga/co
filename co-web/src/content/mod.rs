//! Content — platform content entities, one folder per entity (CO-432).
//!
//! Each entity folder owns its routes (and, where applicable, its SQLite
//! index). Indexes are accessed by handlers through the `crate::repository`
//! layer, following the CO-390 layering template.
//!
//! The `pub use … as …` aliases preserve the pre-CO-432 flat module paths
//! (`crate::entry_routes`, `crate::vault_routes`, …) per the MODULES.md
//! re-export rule, so call sites did not need to change.

pub mod agent_sessions;
pub mod assets;
pub mod delivery;
pub mod entries;
pub mod graph;
pub mod iceberg;
pub mod models;
pub mod obsidian_tasks;
pub mod openapi;
pub mod proposals;
pub mod references;
pub mod relations;
pub mod reviews;
pub mod search;
pub mod translate;
pub mod universe;
pub mod universo;
pub mod usage;
pub mod vault;
pub mod versioning;
pub mod workspace;

// Compat aliases — pre-CO-432 module paths.
pub use agent_sessions::routes as agent_session_routes;
pub use assets::blobs as blob_routes;
pub use assets::crypto as asset_crypto;
pub use assets::routes as asset_routes;
pub use delivery::routes as delivery_routes;
pub use entries::index as entry_index;
pub use entries::query_dsl;
pub use entries::routes as entry_routes;
pub use graph::routes as graph_routes;
pub use graph::view_routes as graph_view_routes;
pub use openapi::routes as openapi_routes;
pub use proposals::routes as proposal_routes;
pub use references::index as reference_index;
pub use references::routes as reference_routes;
pub use relations::index as relation_index;
pub use relations::routes as relation_routes;
pub use reviews::routes as review_routes;
pub use search::query as query_routes;
pub use search::routes as search_routes;
pub use translate::routes as translate_routes;
pub use universe::routes as universe_routes;
pub use usage::routes as usage_routes;
pub use vault::routes as vault_routes;
pub use versioning::branches as branch_routes;
pub use versioning::op_log as op_log_routes;
pub use versioning::states as state_routes;
pub use workspace::lobby as workspace_lobby;
pub use workspace::routes as workspace_routes;
pub use workspace::template_routes as workspace_template_routes;
pub use workspace::ws as workspace_ws;
