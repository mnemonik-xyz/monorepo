//! Storage layer -- trait abstractions and SQLite implementation for attestation persistence.

pub mod mode;
pub mod sqlite;
pub mod traits;
// Local-cache backend for verifiable trajectories. Per the storage decision
// (work/verifiable-trajectories/decisions.md) the canonical store is Arweave
// bundles; this SQLite-backed `TrajectoryStore` is the sanctioned optional
// local/offline cache, never the canonical source.
#[cfg(feature = "trajectory-experimental")]
pub mod trajectory_sqlite;

pub use mode::{Visibility, WriteMode};
pub use sqlite::{PublicStats, SqliteStore};
pub use traits::{AttestationRow, SearchResult};
pub use traits::{AttestationStore, LineageStore};
