// Context modules
pub mod admin;
pub mod auth;
pub mod content;
pub mod eda;
pub mod infra;
pub mod integrations;
pub mod platform;
pub mod server;
pub mod social;
pub mod storage;
pub mod sync;

// Universe-specific extensions
pub mod universes;

// Re-exports — every crate::module_name path that existed before the reorganization
// continues to resolve, so no call sites need to change.

// auth context
pub use auth::extractors;
pub use auth::onboarding_routes;
pub use auth::recovery_crypto;
pub use auth::recovery_routes;

// content context
pub use content::agent_session_routes;
pub use content::asset_crypto;
pub use content::asset_routes;
pub use content::blob_routes;
pub use content::branch_routes;
pub use content::entry_index;
pub use content::entry_routes;
pub use content::graph_routes;
pub use content::graph_view_routes;
pub use content::iceberg;
pub use content::models;
pub use content::obsidian_tasks;
pub use content::op_log_routes;
pub use content::openapi_routes;
pub use content::proposal_routes;
pub use content::query_dsl;
pub use content::query_routes;
pub use content::reference_index;
pub use content::reference_routes;
pub use content::relation_index;
pub use content::relation_routes;
pub use content::search_routes;
pub use content::state_routes;
pub use content::universe_routes;
pub use content::universo;
pub use content::vault_routes;

// social context
pub use social::chat;
pub use social::contact_routes;
pub use social::dm_routes;
pub use social::invitation_routes;
pub use social::notification_email_worker;
pub use social::notification_providers;
pub use social::notification_push_worker;
pub use social::notification_routes;
pub use social::push_routes;
pub use social::sync_ws;
pub use social::ws;

// admin context
pub use admin::admin_routes;
pub use admin::analytics_public;
pub use admin::analytics_routes;
pub use admin::backup_routes;
pub use admin::changelog_parser;
pub use admin::changelog_routes;
pub use admin::deployment_dashboard;
pub use admin::dev_board;
pub use admin::gestao_routes;
pub use admin::interactions;
pub use admin::lead_routes;
pub use admin::repl_routes;
pub use admin::resumo_routes;
pub use admin::storage_dashboard;
pub use admin::uat_routes;

// integrations context
pub use integrations::ai_routes;
pub use integrations::chat_routes;
pub use integrations::email_smtp;
pub use integrations::feedback_routes;
pub use integrations::github_auth;
pub use integrations::log_drain_routes;
pub use integrations::oauth_google;
pub use integrations::oidc_routes;
pub use integrations::webhook;
pub use integrations::webhook_routes;
pub use integrations::webhook_worker;

// platform context
pub use platform::ab;
pub use platform::ab_routes;
pub use platform::atividade;
pub use platform::baseline;
pub use platform::cache;
pub use platform::config;
pub use platform::deployment_snapshot_worker;
pub use platform::desktop_notify;
pub use platform::doc_gen;
pub use platform::embedding;
pub use platform::embedding_index;
pub use platform::embedding_worker;
pub use platform::error;
pub use platform::events;
pub use platform::experiment;
pub use platform::geo;
pub use platform::index_manager;
pub use platform::job_queue;
pub use platform::live_routes;
pub use platform::observability;
pub use platform::plugin_loader;
pub use platform::pretty_urls;
pub use platform::rate_limit;
pub use platform::telemetry;
pub use platform::theme_engine;
pub use platform::universe_pool;
pub use platform::vcs;
pub use platform::wae;
pub use platform::worker_supervisor;
pub use platform::workers;

// universes — quilombo (quilomboaraucaria.org backend)
// storage — backup backend trait + implementations
pub use storage::backup;

pub use universes::quilombo::processos;
pub use universes::quilombo::quilombo_models;
pub use universes::quilombo::quilombo_permissoes;
pub use universes::quilombo::quilombo_routes;
pub use universes::quilombo::quilombo_storage;
pub use universes::quilombo::quilombo_telemetria;

// universes — game (yggdrasil leaderboard)
pub use universes::game::game_models;
pub use universes::game::game_routes;
