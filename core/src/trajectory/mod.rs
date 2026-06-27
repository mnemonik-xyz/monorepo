//! Verifiable trajectories — prove an agent's sequence of steps was *good*.
//!
//! Two guarantees, both backend-agnostic and client-verifiable:
//! - **Chain integrity** ([`verify::verify_chain`]): steps are ordered
//!   (`seq` dense `0..n`), hash-linked (`prev_hash[i] == content_hash[i-1]`,
//!   inside the signed payload), and each COSE_Sign1 signature checks out.
//! - **Verdict binding** ([`verify::verdict_coverage`]): each step carries a
//!   verdict signed by an **independent** judge (`judge != producer`).
//!
//! Mnemonic owns the commitment + lineage layer; it *binds* external correctness
//! proofs (PRM / zkML / opML / TEE / OCP) by hash via the verdict's `proof_ref`,
//! it never produces them. The order-preserving Merkle root over the steps'
//! content hashes ([`crate::merkle::trajectory_root`]) is the trajectory's
//! `batch_root` — and, by construction, the Arweave bundle manifest root.
//!
//! Pure module: depends only on `codec` + `merkle`, no storage/network, so the
//! identical verification runs in a browser/wasm client or against an
//! `ArweaveStore` on a stateless server.

use serde::{Deserialize, Serialize};

pub mod build;
pub mod store;
pub mod verify;

pub use build::{
    build_step, build_trajectory_summary, build_verdict, StepInput, SummaryInput, VerdictInput,
};
pub use store::{InMemoryTrajectoryStore, StepRecord, TrajectoryStore, VerdictRecord};
pub use verify::{
    build_report, trajectory_proofs, verdict_coverage, verify_chain, verify_checkpoint_chain,
    ChainVerification, CoverageReport, StepProof, TrajectoryReport,
};

#[cfg(test)]
mod tests;

/// A judge's verdict over a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerdictStatus {
    /// Step is a valid/quality move.
    Pass,
    /// Step is usable but flagged — counts toward coverage, not a blocker.
    Concern,
    /// Step is wrong — blocks `safe_to_settle`.
    Reject,
}

impl VerdictStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Concern => "concern",
            Self::Reject => "reject",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pass" => Some(Self::Pass),
            "concern" => Some(Self::Concern),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}
