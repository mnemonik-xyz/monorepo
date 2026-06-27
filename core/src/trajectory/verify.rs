//! Backend-agnostic chain integrity + verdict coverage verification.

use serde::Serialize;

use super::store::TrajectoryStore;
use super::VerdictStatus;
use crate::codec::hash::hash_bytes;
use crate::codec::sign::verify_artifact;
use crate::merkle::{self, ProofStep};

/// Result of ordered-chain verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainVerification {
    pub chain_valid: bool,
    /// `seq` of the first step that failed a check (`None` if all valid).
    pub broken_at: Option<u64>,
    pub verified_steps: usize,
    pub total_steps: usize,
}

/// Verify ordering, linkage, content hash, and signature for a trajectory.
/// Backend-agnostic: on the canonical path `store` is an Arweave-backed store,
/// so a stateless server verifies straight from the permaweb.
pub fn verify_chain(
    store: &dyn TrajectoryStore,
    trajectory_id: &str,
) -> anyhow::Result<ChainVerification> {
    let steps = store.steps_for_trajectory(trajectory_id)?;
    let total = steps.len();

    for (i, s) in steps.iter().enumerate() {
        let broken = ChainVerification {
            chain_valid: false,
            broken_at: Some(i as u64),
            verified_steps: i,
            total_steps: total,
        };

        // 1. Dense `0..n` ordering — a gap/dup/out-of-order breaks the chain.
        if s.seq != i as u64 {
            return Ok(broken);
        }
        // 2. Hash linkage. Genesis has no prev_hash; others link to prior hash.
        let expected_prev = if i == 0 {
            None
        } else {
            Some(steps[i - 1].content_hash.as_str())
        };
        if s.prev_hash.as_deref() != expected_prev {
            return Ok(broken);
        }
        // 3. Content integrity — recompute blake3 over the signed payload.
        if hash_bytes(&s.canonical_cbor) != s.content_hash {
            return Ok(broken);
        }
        // 4. Signature, and the signer is the claimed producer.
        match verify_artifact(&s.cose_bytes, Some(&s.content_hash)) {
            Ok(v) if v.valid && v.signer == s.producer => {}
            _ => return Ok(broken),
        }
    }

    Ok(ChainVerification {
        chain_valid: true,
        broken_at: None,
        verified_steps: total,
        total_steps: total,
    })
}

/// Verdict coverage over a trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageReport {
    pub covered_steps: usize,
    pub total_steps: usize,
    /// At least one independent, valid `reject` verdict exists.
    pub has_reject: bool,
}

impl CoverageReport {
    /// Fraction of steps with an independent passing/concern verdict, 0.0..=1.0.
    pub fn coverage(&self) -> f32 {
        if self.total_steps == 0 {
            0.0
        } else {
            self.covered_steps as f32 / self.total_steps as f32
        }
    }

    /// Every step covered and at least one step present.
    pub fn full(&self) -> bool {
        self.total_steps > 0 && self.covered_steps == self.total_steps
    }
}

/// A step counts as covered iff it has ≥1 verdict whose judge differs from the
/// step producer, whose signature verifies, and whose status is pass/concern.
/// An independent, valid `reject` sets `has_reject`.
pub fn verdict_coverage(
    store: &dyn TrajectoryStore,
    trajectory_id: &str,
) -> anyhow::Result<CoverageReport> {
    let steps = store.steps_for_trajectory(trajectory_id)?;
    let total = steps.len();
    let mut covered = 0usize;
    let mut has_reject = false;

    for s in &steps {
        let mut step_covered = false;
        for v in store.verdicts_for_step(&s.content_hash)? {
            // Independence: a self-judged verdict is worthless as a signal.
            if v.judge == s.producer {
                continue;
            }
            // Signature must verify and bind to the claimed judge.
            let ok = matches!(
                verify_artifact(&v.cose_bytes, Some(&v.content_hash)),
                Ok(r) if r.valid && r.signer == v.judge
            );
            if !ok {
                continue;
            }
            match v.status {
                VerdictStatus::Reject => has_reject = true,
                VerdictStatus::Pass | VerdictStatus::Concern => step_covered = true,
            }
        }
        if step_covered {
            covered += 1;
        }
    }

    Ok(CoverageReport {
        covered_steps: covered,
        total_steps: total,
        has_reject,
    })
}

/// An inclusion proof for one step against the trajectory `batch_root`.
#[derive(Debug, Clone)]
pub struct StepProof {
    pub seq: u64,
    pub content_hash: String,
    pub proof: Vec<ProofStep>,
}

/// Full verifier output for a trajectory.
#[derive(Debug, Clone, Serialize)]
pub struct TrajectoryReport {
    pub trajectory_id: String,
    pub chain_valid: bool,
    pub broken_at: Option<u64>,
    pub covered_steps: usize,
    pub step_count: usize,
    pub has_reject: bool,
    /// Order-preserving Merkle root hex (== bundle manifest root).
    pub batch_root: String,
    /// `chain_valid && full coverage && no reject && non-empty`. The ERC-8301
    /// "every preceding stage has a valid proof before a high-value action"
    /// invariant, evaluated at verify time.
    pub safe_to_settle: bool,
}

/// Compute the full report: chain validity, coverage, batch root, settle gate.
pub fn build_report(
    store: &dyn TrajectoryStore,
    trajectory_id: &str,
) -> anyhow::Result<TrajectoryReport> {
    let chain = verify_chain(store, trajectory_id)?;
    let cov = verdict_coverage(store, trajectory_id)?;
    let steps = store.steps_for_trajectory(trajectory_id)?;
    let hashes: Vec<String> = steps.iter().map(|s| s.content_hash.clone()).collect();
    let batch_root = merkle::to_hex32(&merkle::trajectory_root(&hashes));
    let safe_to_settle = chain.chain_valid && cov.full() && !cov.has_reject;

    Ok(TrajectoryReport {
        trajectory_id: trajectory_id.to_string(),
        chain_valid: chain.chain_valid,
        broken_at: chain.broken_at,
        covered_steps: cov.covered_steps,
        step_count: steps.len(),
        has_reject: cov.has_reject,
        batch_root,
        safe_to_settle,
    })
}

/// Verify the checkpoint root-of-roots chain: each checkpoint's `prev_root`
/// must equal the prior checkpoint's `batch_root` (the first carries `None`).
/// Items are `(batch_root, prev_root)` in checkpoint order.
pub fn verify_checkpoint_chain(checkpoints: &[(String, Option<String>)]) -> bool {
    for (i, (_root, prev)) in checkpoints.iter().enumerate() {
        let expected = if i == 0 {
            None
        } else {
            Some(&checkpoints[i - 1].0)
        };
        if prev.as_ref() != expected {
            return false;
        }
    }
    true
}

/// Inclusion proofs for every step against the trajectory `batch_root`.
pub fn trajectory_proofs(
    store: &dyn TrajectoryStore,
    trajectory_id: &str,
) -> anyhow::Result<Vec<StepProof>> {
    let steps = store.steps_for_trajectory(trajectory_id)?;
    let hashes: Vec<String> = steps.iter().map(|s| s.content_hash.clone()).collect();
    let mut out = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        if let Some((_root, proof)) = merkle::trajectory_prove(&hashes, i) {
            out.push(StepProof {
                seq: s.seq,
                content_hash: s.content_hash.clone(),
                proof,
            });
        }
    }
    Ok(out)
}
