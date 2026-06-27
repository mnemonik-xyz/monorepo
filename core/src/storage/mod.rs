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
// Canonical Arweave-bundle backend: tagged ANS-104 data items written via Irys,
// read back by GraphQL tag query. The stateless-MCP, decentralized source of
// truth (work/verifiable-trajectories/decisions.md).
#[cfg(feature = "trajectory-experimental")]
pub mod trajectory_arweave;

pub use mode::{Visibility, WriteMode};
pub use sqlite::{PublicStats, SqliteStore};
pub use traits::{AttestationRow, SearchResult};
pub use traits::{AttestationStore, LineageStore};
