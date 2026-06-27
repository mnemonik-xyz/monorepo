//! MCP tool handlers for verifiable trajectories.
//!
//! Three tools, mirroring the ERC-8301 step/prove split:
//! - `mnemonic_attest_step` — commit a step now (auto-links `prev_hash`/`seq`).
//! - `mnemonic_attest_verdict` — an independent judge attaches a verdict;
//!   rejects self-judging (judge == producer).
//! - `mnemonic_verify_trajectory` — chain validity + coverage + batch root +
//!   inclusion proofs + the `safe_to_settle` gate.
//!
//! These are pure handler functions over a `SqliteTrajectoryStore` (the local
//! cache) + caller keypair, returning JSON. They contain the tool business
//! logic and are unit-tested here; wiring into the live JSON-RPC dispatch and
//! the canonical Arweave-bundle write path is the remaining integration step
//! (see work/verifiable-trajectories/tasks/5.md).

use mnemonic_core::merkle::to_hex32;
use mnemonic_core::storage::trajectory_sqlite::SqliteTrajectoryStore;
use mnemonic_core::trajectory::{
    build_report, build_step, build_verdict, trajectory_proofs, StepInput, StepRecord,
    TrajectoryStore, VerdictInput, VerdictRecord, VerdictStatus,
};
use serde_json::{json, Value};
use solana_sdk::signature::{Keypair, Signer};

fn err(msg: &str) -> Value {
    json!({ "status": "error", "error": msg })
}

/// `mnemonic_attest_step` — sign + persist a step. `seq` and `prev_hash` are
/// auto-derived from the trajectory head when omitted, so callers just stream
/// content.
pub fn attest_step(
    store: &SqliteTrajectoryStore,
    keypair: &Keypair,
    trajectory_id: &str,
    content: &str,
    seq: Option<u64>,
    created_at: &str,
) -> Value {
    let head = match store.trajectory_head(trajectory_id) {
        Ok(h) => h,
        Err(e) => return err(&format!("head lookup failed: {e}")),
    };
    let next_seq = seq.unwrap_or_else(|| head.as_ref().map(|h| h.seq + 1).unwrap_or(0));
    let prev_hash = head.as_ref().map(|h| h.content_hash.clone());

    let signed = match build_step(
        &StepInput {
            trajectory_id,
            seq: next_seq,
            content,
            prev_hash: prev_hash.as_deref(),
            created_at,
        },
        keypair,
    ) {
        Ok(s) => s,
        Err(e) => return err(&format!("sign failed: {e}")),
    };

    let record = StepRecord {
        trajectory_id: trajectory_id.to_string(),
        seq: next_seq,
        content_hash: signed.content_hash.clone(),
        prev_hash: prev_hash.clone(),
        producer: keypair.pubkey().to_string(),
        cose_bytes: signed.cose_bytes,
        canonical_cbor: signed.canonical_cbor,
    };
    if let Err(e) = store.insert_step(&record) {
        return err(&format!("persist failed: {e}"));
    }

    json!({
        "status": "ok",
        "trajectory_id": trajectory_id,
        "seq": next_seq,
        "content_hash": signed.content_hash,
        "prev_hash": prev_hash,
        "write_mode": "local",
    })
}

/// `mnemonic_attest_verdict` — an independent judge signs a verdict over a step.
/// Rejects a verdict whose judge equals the step producer (a self-judged verdict
/// is worthless as a correctness signal).
#[allow(clippy::too_many_arguments)]
pub fn attest_verdict(
    store: &SqliteTrajectoryStore,
    judge: &Keypair,
    step_hash: &str,
    status: &str,
    score: Option<f32>,
    proof_ref: Option<&str>,
    proof_kind: Option<&str>,
    rationale: Option<&str>,
    created_at: &str,
) -> Value {
    let Some(status) = VerdictStatus::from_str(status) else {
        return err("status must be one of: pass, concern, reject");
    };
    let judge_pk = judge.pubkey().to_string();

    match store.step_producer(step_hash) {
        Ok(Some(producer)) if producer == judge_pk => {
            return err("judge must differ from step producer (self-judging is not allowed)");
        }
        Ok(Some(_)) => {}
        Ok(None) => return err("unknown step_hash"),
        Err(e) => return err(&format!("producer lookup failed: {e}")),
    }

    let signed = match build_verdict(
        &VerdictInput {
            step_hash,
            status,
            score,
            proof_ref,
            proof_kind,
            rationale,
            created_at,
        },
        judge,
    ) {
        Ok(s) => s,
        Err(e) => return err(&format!("sign failed: {e}")),
    };

    let record = VerdictRecord {
        step_hash: step_hash.to_string(),
        status,
        judge: judge_pk.clone(),
        content_hash: signed.content_hash.clone(),
        cose_bytes: signed.cose_bytes,
    };
    if let Err(e) = store.insert_verdict(&record) {
        return err(&format!("persist failed: {e}"));
    }

    json!({
        "status": "ok",
        "step_hash": step_hash,
        "verdict_hash": signed.content_hash,
        "judge": judge_pk,
        "verdict_status": status.as_str(),
    })
}

/// `mnemonic_verify_trajectory` — full verifier output: chain validity, verdict
/// coverage, batch root, per-step inclusion proofs, and the settle gate.
pub fn verify_trajectory(store: &SqliteTrajectoryStore, trajectory_id: &str) -> Value {
    let report = match build_report(store, trajectory_id) {
        Ok(r) => r,
        Err(e) => return err(&format!("verify failed: {e}")),
    };
    let proofs = match trajectory_proofs(store, trajectory_id) {
        Ok(p) => p,
        Err(e) => return err(&format!("proofs failed: {e}")),
    };
    let proofs_json: Vec<Value> = proofs
        .iter()
        .map(|p| {
            json!({
                "seq": p.seq,
                "content_hash": p.content_hash,
                "proof": p.proof.iter().map(|s| json!({
                    "sibling": to_hex32(&s.sibling),
                    "sibling_is_right": s.sibling_is_right,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    json!({
        "status": "ok",
        "trajectory_id": report.trajectory_id,
        "chain_valid": report.chain_valid,
        "broken_at": report.broken_at,
        "covered_steps": report.covered_steps,
        "step_count": report.step_count,
        "has_reject": report.has_reject,
        "batch_root": report.batch_root,
        "safe_to_settle": report.safe_to_settle,
        "proofs": proofs_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: &str = "2026-06-27T00:00:00Z";

    fn store() -> SqliteTrajectoryStore {
        SqliteTrajectoryStore::in_memory().unwrap()
    }

    #[test]
    fn attest_step_auto_links_prev_hash_and_seq() {
        let s = store();
        let kp = Keypair::new();
        let r0 = attest_step(&s, &kp, "t", "plan", None, TS);
        assert_eq!(r0["seq"], 0);
        assert!(r0["prev_hash"].is_null());
        let h0 = r0["content_hash"].as_str().unwrap().to_string();

        let r1 = attest_step(&s, &kp, "t", "act", None, TS);
        assert_eq!(r1["seq"], 1);
        assert_eq!(r1["prev_hash"].as_str().unwrap(), h0);
    }

    #[test]
    fn attest_verdict_rejects_self_judging() {
        let s = store();
        let kp = Keypair::new();
        let step = attest_step(&s, &kp, "t", "plan", None, TS);
        let h = step["content_hash"].as_str().unwrap();
        // Producer judging its own step → rejected.
        let bad = attest_verdict(&s, &kp, h, "pass", None, None, None, None, TS);
        assert_eq!(bad["status"], "error");
        // Independent judge → ok.
        let judge = Keypair::new();
        let good = attest_verdict(&s, &judge, h, "pass", None, None, None, None, TS);
        assert_eq!(good["status"], "ok");
    }

    #[test]
    fn attest_verdict_unknown_step() {
        let s = store();
        let judge = Keypair::new();
        let r = attest_verdict(&s, &judge, "deadbeef", "pass", None, None, None, None, TS);
        assert_eq!(r["status"], "error");
    }

    #[test]
    fn verify_trajectory_full_flow_is_safe_to_settle() {
        let s = store();
        let producer = Keypair::new();
        let judge = Keypair::new();
        let mut hashes = vec![];
        for c in ["plan", "act", "synthesize"] {
            let r = attest_step(&s, &producer, "t", c, None, TS);
            hashes.push(r["content_hash"].as_str().unwrap().to_string());
        }
        for h in &hashes {
            let v = attest_verdict(
                &s,
                &judge,
                h,
                "pass",
                Some(0.9),
                None,
                Some("prm"),
                None,
                TS,
            );
            assert_eq!(v["status"], "ok");
        }
        let report = verify_trajectory(&s, "t");
        assert_eq!(report["chain_valid"], true);
        assert_eq!(report["covered_steps"], 3);
        assert_eq!(report["safe_to_settle"], true);
        assert_eq!(report["proofs"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn reject_verdict_blocks_settle() {
        let s = store();
        let producer = Keypair::new();
        let judge = Keypair::new();
        let r = attest_step(&s, &producer, "t", "plan", None, TS);
        let h = r["content_hash"].as_str().unwrap();
        attest_verdict(&s, &judge, h, "reject", None, None, None, Some("wrong"), TS);
        let report = verify_trajectory(&s, "t");
        assert_eq!(report["has_reject"], true);
        assert_eq!(report["safe_to_settle"], false);
    }
}
