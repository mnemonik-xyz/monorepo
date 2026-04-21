//! Storage layer -- trait abstractions and SQLite implementation for attestation persistence.

pub mod traits;
pub mod sqlite;

pub use traits::{AttestationStore, LineageStore};
pub use sqlite::SqliteStore;
pub use traits::{AttestationRow, SearchResult};
