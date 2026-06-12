//! Service layer — business rules, independent of HTTP and database.
//!
//! CO-390 spike: proof-of-concept layered architecture.
//! Services contain rules that are unit-testable without HTTP setup.

pub mod entry_service;
pub mod reference_service;
pub mod relation_service;

pub use entry_service::EntryService;
pub use reference_service::ReferenceService;
pub use relation_service::RelationService;
