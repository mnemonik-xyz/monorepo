//! A2A (Agent-to-Agent) protocol v1.0.0-rc codec bindings.
//!
//! Provides type-safe Rust mirrors of the A2A wire types with RFC 8785 JCS
//! canonicalization (`to_jcs_bytes`) and COSE_Sign1 envelope wrapping
//! (`to_canonical_envelope`), following Decision 1 and Decision 5 in
//! work/a2a-bridge/decisions.md.
//!
//! Enabled only with `--features a2a-experimental`.

pub mod artifact;
pub mod message;
pub mod part;
pub mod task;

pub use artifact::A2aArtifact;
pub use message::Message;
pub use part::Part;
pub use task::Task;
