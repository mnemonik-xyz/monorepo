//! Storage layer -- trait abstractions and SQLite implementation for attestation persistence.

pub mod mode;
pub mod sqlite;
pub mod traits;

pub use mode::{Visibility, WriteMode};
pub use sqlite::{BlogPost, PublicArtifact, PublicStats, SqliteStore, TimelineBucket};
pub use traits::{AttestationRow, SearchResult};
pub use traits::{AttestationStore, LineageStore};
