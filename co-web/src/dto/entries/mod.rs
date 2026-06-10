//! Entry DTO family — one type per API surface.
//!
//! CO-390 spike: six DTOs following the library-manager pattern.
//! See `docs/spikes/library-manager-case-study.md` Pattern 1 for rationale.

pub mod basic;
pub mod create_request;
pub mod create_response;
pub mod filter;
pub mod info;
pub mod update_request;

pub use basic::EntryBasicDto;
pub use create_request::CreateEntryRequest;
pub use create_response::CreateEntryResponse;
pub use filter::EntryFilter;
pub use info::EntryInfoDto;
pub use update_request::UpdateEntryRequest;
