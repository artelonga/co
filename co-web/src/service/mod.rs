//! Service layer — business rules, independent of HTTP and database.
//!
//! CO-390 spike: proof-of-concept layered architecture.
//! Services contain rules that are unit-testable without HTTP setup.

pub mod entry_service;

pub use entry_service::EntryService;
