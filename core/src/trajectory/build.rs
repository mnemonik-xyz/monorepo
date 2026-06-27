//! Typed builders that produce signed STEP/VERDICT/TRAJECTORY artifacts.
//!
//! Each returns a [`SignedArtifact`] (`cose_bytes` + `content_hash` +
//! `canonical_cbor`) — the bytes a backend persists and a verifier re-checks.

use solana_sdk::signature::{Keypair, Signer};

use super::VerdictStatus;
use crate::codec::schema::{STEP_V1, TRAJECTORY_V1, VERDICT_V1};
use crate::codec::sign::{sign_artifact, SignedArtifact};

/// Inputs for one trajectory step.
pub struct StepInput<'a> {
    pub trajectory_id: &'a str,
    pub seq: u64,
    pub content: &'a str,
    /// `content_hash` of the prior step; `None` only for the genesis step.
    pub prev_hash: Option<&'a str>,
    pub created_at: &'a str,
}

/// Build + sign a `STEP_V1`. `prev_hash` is part of the signed payload, so any
/// reordering or relinking is detectable. `producer` is the signer's pubkey.
pub fn build_step(input: &StepInput, keypair: &Keypair) -> Result<SignedArtifact, String> {
    let producer = keypair.pubkey().to_string();
    let mut obj = serde_json::json!({
        "artifact_id": format!("step:{}:{}", input.trajectory_id, input.seq),
        "type": "step",
        "schema_version": 1,
        "content": input.content,
        "trajectory_id": input.trajectory_id,
        "seq": input.seq,
        "created_at": input.created_at,
        "producer": producer,
    });
    if let Some(ph) = input.prev_hash {
        obj["prev_hash"] = serde_json::json!(ph);
    }
    sign_artifact(&obj, &STEP_V1, keypair)
}

/// Inputs for a verdict over a step.
pub struct VerdictInput<'a> {
    /// `content_hash` of the step being judged.
    pub step_hash: &'a str,
    pub status: VerdictStatus,
    pub score: Option<f32>,
    /// Hash of an external correctness proof (zkML/TEE/opML/OCP), bound by ref.
    pub proof_ref: Option<&'a str>,
    pub proof_kind: Option<&'a str>,
    pub rationale: Option<&'a str>,
    pub created_at: &'a str,
}

/// Build + sign a `VERDICT_V1` with the **judge's** keypair. The judge must
/// differ from the step producer; that invariant is enforced at attest time
/// (and re-checked during coverage), not here.
pub fn build_verdict(input: &VerdictInput, judge: &Keypair) -> Result<SignedArtifact, String> {
    let judge_pk = judge.pubkey().to_string();
    let mut obj = serde_json::json!({
        "artifact_id": format!("verdict:{}:{}", input.step_hash, judge_pk),
        "type": "verdict",
        "schema_version": 1,
        "step_hash": input.step_hash,
        "status": input.status.as_str(),
        "created_at": input.created_at,
        "judge": judge_pk,
    });
    if let Some(s) = input.score {
        obj["score"] = serde_json::json!(s);
    }
    if let Some(p) = input.proof_ref {
        obj["proof_ref"] = serde_json::json!(p);
    }
    if let Some(k) = input.proof_kind {
        obj["proof_kind"] = serde_json::json!(k);
    }
    if let Some(r) = input.rationale {
        obj["rationale"] = serde_json::json!(r);
    }
    sign_artifact(&obj, &VERDICT_V1, judge)
}

/// Inputs for an anchored trajectory (or checkpoint) summary.
pub struct SummaryInput<'a> {
    pub trajectory_id: &'a str,
    pub step_count: u64,
    /// Order-preserving Merkle root hex (== bundle manifest root).
    pub batch_root: &'a str,
    /// Prior checkpoint's `batch_root` (root-of-roots); `None` for the first.
    pub prev_root: Option<&'a str>,
    pub verdict_coverage: Option<f32>,
    pub chain_valid: Option<bool>,
    pub is_final: Option<bool>,
    pub created_at: &'a str,
}

/// Build + sign a `TRAJECTORY_V1` summary — the artifact whose `batch_root`
/// gets anchored.
pub fn build_trajectory_summary(
    input: &SummaryInput,
    keypair: &Keypair,
) -> Result<SignedArtifact, String> {
    let producer = keypair.pubkey().to_string();
    let mut obj = serde_json::json!({
        "artifact_id": format!("trajectory:{}", input.trajectory_id),
        "type": "trajectory",
        "schema_version": 1,
        "trajectory_id": input.trajectory_id,
        "step_count": input.step_count,
        "batch_root": input.batch_root,
        "created_at": input.created_at,
        "producer": producer,
    });
    if let Some(p) = input.prev_root {
        obj["prev_root"] = serde_json::json!(p);
    }
    if let Some(c) = input.verdict_coverage {
        obj["verdict_coverage"] = serde_json::json!(c);
    }
    if let Some(c) = input.chain_valid {
        obj["chain_valid"] = serde_json::json!(c);
    }
    if let Some(f) = input.is_final {
        obj["is_final"] = serde_json::json!(f);
    }
    sign_artifact(&obj, &TRAJECTORY_V1, keypair)
}
