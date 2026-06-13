//! co-pipeline — CO-88 end-to-end content pipeline UAT runner.
//!
//! Exercises the markdown ↔ `CoFile` ↔ (compressed/encrypted/signed) bytes ↔
//! transport pipeline across four paths (local, local-api, uat, prod), five
//! layer combos, and three universes, then emits a per-universe report with
//! file counts, byte sizes, compression ratios, and encode/decode/transfer
//! timings.
//!
//! Developer tooling — NOT part of the `co` CLI scaffold.

pub mod combo;
pub mod corpus;
pub mod gate;
pub mod matrix;
pub mod pipeline;
pub mod report;
pub mod transport;

pub use combo::Combo;
pub use report::PipelineReport;
pub use transport::PipePath;
