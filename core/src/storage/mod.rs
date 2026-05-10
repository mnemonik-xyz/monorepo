//! Storage layer -- trait abstractions and SQLite implementation for attestation persistence.

pub mod sqlite;
pub mod traits;

pub use sqlite::{PublicStats, SqliteStore};
pub use traits::{AttestationRow, SearchResult};
pub use traits::{AttestationStore, LineageStore};
