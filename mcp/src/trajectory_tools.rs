//! MCP tool handlers for verifiable trajectories — **non-custodial**.
//!
//! The server never holds a signing key for user content. Clients sign STEP /
//! VERDICT artifacts locally (via the SDK / `mnemonic_core::trajectory::build_*`)
//! and submit the COSE_Sign1 envelope as hex; the server only **verifies and
//! stores** it. The producing/judging identity comes from the COSE `kid`, never
//! from a server key.
//!
//! - `mnemonic_attest_step` — verify a client-signed step, enforce dense
//!   `seq`/`prev_hash` linkage to the trajectory head, store.
//! - `mnemonic_attest_verdict` — verify a client-signed verdict, enforce judge
//!   ≠ producer (independence), store.
//! - `mnemonic_verify_trajectory` — chain validity + coverage + batch root +
//!   inclusion proofs + the `safe_to_settle` gate (read-only).

use mnemonic_core::merkle::to_hex32;
use mnemonic_core::storage::trajectory_sqlite::SqliteTrajectoryStore;
use mnemonic_core::trajectory::{
    build_report, step_from_cose, trajectory_proofs, verdict_from_cose, TrajectoryStore,
};
use serde_json::{json, Value};

fn err(msg: &str) -> Value {
    json!({ "status": "error", "error": msg })
}

fn decode_hex(s: &str) -> Result<Vec<u8>, Value> {
    hex::decode(s).map_err(|_| err("`signed` must be hex-encoded COSE_Sign1 bytes"))
}

/// `mnemonic_attest_step` — store a client-signed step after verifying its
/// signature and that it densely links to the trajectory head. The server signs
/// nothing; `producer` is the COSE signer.
pub fn attest_step(store: &SqliteTrajectoryStore, signed_hex: &str) -> Value {
    let cose = match decode_hex(signed_hex) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let record = match step_from_cose(&cose) {
        Ok(r) => r,
        Err(e) => return err(&format!("invalid step envelope: {e}")),
    };

    // Enforce dense, hash-linked append against the current head.
    let head = match store.trajectory_head(&record.trajectory_id) {
        Ok(h) => h,
        Err(e) => return err(&format!("head lookup failed: {e}")),
    };
    let expected_seq = head.as_ref().map(|h| h.seq + 1).unwrap_or(0);
    if record.seq != expected_seq {
        return err(&format!(
            "seq {} does not append to head (expected {expected_seq})",
            record.seq
        ));
    }
    let expected_prev = head.as_ref().map(|h| h.content_hash.clone());
    if record.prev_hash != expected_prev {
        return err("prev_hash does not link to the trajectory head");
    }

    if let Err(e) = store.insert_step(&record) {
        return err(&format!("persist failed: {e}"));
    }
    json!({
        "status": "ok",
        "trajectory_id": record.trajectory_id,
        "seq": record.seq,
        "content_hash": record.content_hash,
        "producer": record.producer,
        "write_mode": "local",
    })
}

/// `mnemonic_attest_verdict` — store a client-signed verdict after verifying its
/// signature and that the judge differs from the step producer.
pub fn attest_verdict(store: &SqliteTrajectoryStore, signed_hex: &str) -> Value {
    let cose = match decode_hex(signed_hex) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let record = match verdict_from_cose(&cose) {
        Ok(r) => r,
        Err(e) => return err(&format!("invalid verdict envelope: {e}")),
    };

    match store.step_producer(&record.step_hash) {
        Ok(Some(producer)) if producer == record.judge => {
            return err("judge must differ from step producer (self-judging is not allowed)");
        }
        Ok(Some(_)) => {}
        Ok(None) => return err("unknown step_hash"),
        Err(e) => return err(&format!("producer lookup failed: {e}")),
    }

    if let Err(e) = store.insert_verdict(&record) {
        return err(&format!("persist failed: {e}"));
    }
    json!({
        "status": "ok",
        "step_hash": record.step_hash,
        "verdict_hash": record.content_hash,
        "judge": record.judge,
        "verdict_status": record.status.as_str(),
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
    use mnemonic_core::trajectory::{
        build_step, build_verdict, StepInput, VerdictInput, VerdictStatus,
    };
    use solana_sdk::signature::{Keypair, Signer};

    const TS: &str = "2026-06-27T00:00:00Z";

    fn store() -> SqliteTrajectoryStore {
        SqliteTrajectoryStore::in_memory().unwrap()
    }

    /// Client-side: build + sign a step, return its hex envelope.
    fn signed_step(tid: &str, seq: u64, prev: Option<&str>, kp: &Keypair) -> (String, String) {
        let s = build_step(
            &StepInput {
                trajectory_id: tid,
                seq,
                content: "c",
                prev_hash: prev,
                created_at: TS,
            },
            kp,
        )
        .unwrap();
        (hex::encode(&s.cose_bytes), s.content_hash)
    }

    fn signed_verdict(step_hash: &str, status: VerdictStatus, judge: &Keypair) -> String {
        let v = build_verdict(
            &VerdictInput {
                step_hash,
                status,
                score: None,
                proof_ref: None,
                proof_kind: None,
                rationale: None,
                created_at: TS,
            },
            judge,
        )
        .unwrap();
        hex::encode(&v.cose_bytes)
    }

    #[test]
    fn server_never_signs_stores_client_envelope() {
        let s = store();
        let user = Keypair::new();
        let (env0, h0) = signed_step("t", 0, None, &user);
        let r0 = attest_step(&s, &env0);
        assert_eq!(r0["status"], "ok");
        assert_eq!(r0["producer"].as_str().unwrap(), user.pubkey().to_string());
        assert_eq!(r0["content_hash"].as_str().unwrap(), h0);
    }

    #[test]
    fn rejects_non_appending_seq() {
        let s = store();
        let user = Keypair::new();
        // Submit seq 1 first (no head yet → expected 0).
        let (env1, _) = signed_step("t", 1, Some("abc"), &user);
        assert_eq!(attest_step(&s, &env1)["status"], "error");
    }

    #[test]
    fn rejects_wrong_prev_hash_link() {
        let s = store();
        let user = Keypair::new();
        let (env0, _) = signed_step("t", 0, None, &user);
        assert_eq!(attest_step(&s, &env0)["status"], "ok");
        // seq 1 with a bogus prev_hash must be refused.
        let (env_bad, _) = signed_step("t", 1, Some("deadbeef"), &user);
        assert_eq!(attest_step(&s, &env_bad)["status"], "error");
    }

    #[test]
    fn rejects_tampered_envelope() {
        let s = store();
        let user = Keypair::new();
        let (env0, _) = signed_step("t", 0, None, &user);
        let mut bytes = hex::decode(&env0).unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;
        assert_eq!(attest_step(&s, &hex::encode(bytes))["status"], "error");
    }

    #[test]
    fn verdict_independence_enforced() {
        let s = store();
        let user = Keypair::new();
        let judge = Keypair::new();
        let (env0, h0) = signed_step("t", 0, None, &user);
        attest_step(&s, &env0);
        // Producer judging own step → rejected.
        let self_v = signed_verdict(&h0, VerdictStatus::Pass, &user);
        assert_eq!(attest_verdict(&s, &self_v)["status"], "error");
        // Independent judge → ok.
        let ind_v = signed_verdict(&h0, VerdictStatus::Pass, &judge);
        assert_eq!(attest_verdict(&s, &ind_v)["status"], "ok");
    }

    #[test]
    fn full_flow_safe_to_settle() {
        let s = store();
        let user = Keypair::new();
        let judge = Keypair::new();
        let mut prev: Option<String> = None;
        for seq in 0..3u64 {
            let (env, h) = signed_step("t", seq, prev.as_deref(), &user);
            assert_eq!(attest_step(&s, &env)["status"], "ok");
            let v = signed_verdict(&h, VerdictStatus::Pass, &judge);
            assert_eq!(attest_verdict(&s, &v)["status"], "ok");
            prev = Some(h);
        }
        let report = verify_trajectory(&s, "t");
        assert_eq!(report["chain_valid"], true);
        assert_eq!(report["covered_steps"], 3);
        assert_eq!(report["safe_to_settle"], true);
    }
}
