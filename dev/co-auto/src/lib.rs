//! co-auto — automated task execution pipeline.
//!
//! Developer tool extracted from co-cli (CO-84). NOT part of the user-facing
//! CO scaffold. Pipeline picks the next unblocked task, builds context,
//! launches an executor (Claude Code by default), reviews against acceptance
//! criteria, and finalizes (status update, commit, branch).
//!
//! Future composability work in this same crate will introduce traits
//! (TaskSource, TaskSelector, ContextBuilder, Executor, Reviewer, Finalizer)
//! so the procedural pipeline can be swapped piece-by-piece.

pub mod auto;

pub use auto::{AutoConfig, resolve_task_id, run};
