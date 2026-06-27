//! Backend seam for trajectory steps + verdicts.
//!
//! `verify_chain` / `verdict_coverage` take `&dyn TrajectoryStore`, so the same
//! verification runs against an in-memory store (tests, client cache) or an
//! `ArweaveStore` reading data items from the permaweb (stateless MCP). The
//! canonical path holds no database.

use super::VerdictStatus;

/// A reconstructed step. In a real backend these fields are derived from the
/// stored COSE envelope (payload = `canonical_cbor`; `producer` = COSE `kid`;
/// `seq`/`prev_hash`/`trajectory_id` parsed from the CBOR payload).
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub trajectory_id: String,
    pub seq: u64,
    pub content_hash: String,
    pub prev_hash: Option<String>,
    /// COSE signer pubkey (base58) — the producing identity.
    pub producer: String,
    /// Serialized COSE_Sign1 envelope (verified during chain check).
    pub cose_bytes: Vec<u8>,
    /// Canonical CBOR payload (re-hashed to confirm `content_hash`).
    pub canonical_cbor: Vec<u8>,
}

/// A reconstructed verdict over a step.
#[derive(Debug, Clone)]
pub struct VerdictRecord {
    /// `content_hash` of the step this verdict judges.
    pub step_hash: String,
    pub status: VerdictStatus,
    /// COSE signer pubkey (base58) of the judge.
    pub judge: String,
    /// The verdict's own `content_hash`.
    pub content_hash: String,
    pub cose_bytes: Vec<u8>,
}

/// Read side of trajectory storage. Authorization is the caller's concern.
pub trait TrajectoryStore {
    /// Steps for a trajectory, **ordered by `seq` ascending**.
    fn steps_for_trajectory(&self, trajectory_id: &str) -> anyhow::Result<Vec<StepRecord>>;

    /// Verdicts attached to a step (by the step's `content_hash`).
    fn verdicts_for_step(&self, step_hash: &str) -> anyhow::Result<Vec<VerdictRecord>>;

    /// Current head (highest `seq`) of a trajectory, for `prev_hash` auto-fill.
    fn trajectory_head(&self, trajectory_id: &str) -> anyhow::Result<Option<StepRecord>> {
        Ok(self.steps_for_trajectory(trajectory_id)?.pop())
    }
}

/// In-memory store: the reference backend for tests and client-side caches.
#[derive(Debug, Default)]
pub struct InMemoryTrajectoryStore {
    steps: Vec<StepRecord>,
    verdicts: Vec<VerdictRecord>,
}

impl InMemoryTrajectoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_step(&mut self, step: StepRecord) {
        self.steps.push(step);
    }

    pub fn insert_verdict(&mut self, verdict: VerdictRecord) {
        self.verdicts.push(verdict);
    }
}

impl TrajectoryStore for InMemoryTrajectoryStore {
    fn steps_for_trajectory(&self, trajectory_id: &str) -> anyhow::Result<Vec<StepRecord>> {
        let mut out: Vec<StepRecord> = self
            .steps
            .iter()
            .filter(|s| s.trajectory_id == trajectory_id)
            .cloned()
            .collect();
        out.sort_by_key(|s| s.seq);
        Ok(out)
    }

    fn verdicts_for_step(&self, step_hash: &str) -> anyhow::Result<Vec<VerdictRecord>> {
        Ok(self
            .verdicts
            .iter()
            .filter(|v| v.step_hash == step_hash)
            .cloned()
            .collect())
    }
}
