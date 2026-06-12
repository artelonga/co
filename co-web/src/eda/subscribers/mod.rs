//! CO-380: EDA subscriber implementations.
//!
//! Each subscriber is a Tokio task that consumes from the `EdaBus` and
//! performs domain-specific side effects (persistence, fanout, indexing).

pub mod analytics;
pub mod atividades;
pub mod billing;
pub mod comunicacao_live;
pub mod degradation_alerter;
pub mod delivery_pipeline;
pub mod kb;
pub mod sala;
pub mod timeline;
pub mod yggdrasil_notes;
