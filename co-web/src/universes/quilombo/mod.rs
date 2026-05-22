//! Quilombo context — quilomboaraucaria-specific routes, storage, and processos.
//!
//! TODO (per user feedback on CO-224): this folder mixes universe-specific
//! content with platform code. Should be extracted into a separate concern —
//! tracked in CO-265 (separate `form` / `content` / `co` / `universes` /
//! `platform`).

pub mod processos;
pub mod quilombo_models;
pub mod quilombo_permissoes;
pub mod quilombo_routes;
pub mod quilombo_storage;
pub mod quilombo_telemetria;
