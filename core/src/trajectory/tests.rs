//! End-to-end tests for chain integrity, verdict coverage, and the settle gate.
//! Everything runs against the in-memory store — proving the verifier is
//! backend-agnostic and needs no database.

use super::*;
use solana_sdk::signature::{Keypair, Signer};

const TS: &str = "2026-06-27T00:00:00Z";

/// Build a step, sign it, and turn it into a `StepRecord` for the store.
fn step_record(
    tid: &str,
    seq: u64,
    content: &str,
    prev_hash: Option<&str>,
    kp: &Keypair,
) -> StepRecord {
    let signed = build_step(
        &StepInput {
            trajectory_id: tid,
            seq,
            content,
            prev_hash,
            created_at: TS,
        },
        kp,
    )
    .expect("sign step");
    StepRecord {
        trajectory_id: tid.to_string(),
        seq,
        content_hash: signed.content_hash,
        prev_hash: prev_hash.map(str::to_string),
        producer: kp.pubkey().to_string(),
        cose_bytes: signed.cose_bytes,
        canonical_cbor: signed.canonical_cbor,
    }
}

fn verdict_record(step_hash: &str, status: VerdictStatus, judge: &Keypair) -> VerdictRecord {
    let signed = build_verdict(
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
    .expect("sign verdict");
    VerdictRecord {
        step_hash: step_hash.to_string(),
        status,
        judge: judge.pubkey().to_string(),
        content_hash: signed.content_hash,
        cose_bytes: signed.cose_bytes,
    }
}

/// A 3-step linked trajectory by one producer.
fn three_step_store(kp: &Keypair) -> (InMemoryTrajectoryStore, Vec<String>) {
    let mut store = InMemoryTrajectoryStore::new();
    let s0 = step_record("t1", 0, "plan", None, kp);
    let s1 = step_record("t1", 1, "tool call", Some(&s0.content_hash), kp);
    let s2 = step_record("t1", 2, "synthesis", Some(&s1.content_hash), kp);
    let hashes = vec![
        s0.content_hash.clone(),
        s1.content_hash.clone(),
        s2.content_hash.clone(),
    ];
    store.insert_step(s0);
    store.insert_step(s1);
    store.insert_step(s2);
    (store, hashes)
}

#[test]
fn valid_chain_verifies() {
    let kp = Keypair::new();
    let (store, _) = three_step_store(&kp);
    let v = verify_chain(&store, "t1").unwrap();
    assert!(v.chain_valid);
    assert_eq!(v.broken_at, None);
    assert_eq!(v.verified_steps, 3);
}

#[test]
fn reordered_chain_fails() {
    // Swap the prev_hash linkage so seq 2 points at seq 0's hash.
    let kp = Keypair::new();
    let mut store = InMemoryTrajectoryStore::new();
    let s0 = step_record("t1", 0, "a", None, &kp);
    let s1 = step_record("t1", 1, "b", Some(&s0.content_hash), &kp);
    let s2 = step_record("t1", 2, "c", Some(&s0.content_hash), &kp); // wrong prev
    store.insert_step(s0);
    store.insert_step(s1);
    store.insert_step(s2);
    let v = verify_chain(&store, "t1").unwrap();
    assert!(!v.chain_valid);
    assert_eq!(v.broken_at, Some(2));
}

#[test]
fn tampered_content_fails() {
    let kp = Keypair::new();
    let (store, _) = three_step_store(&kp);
    // Corrupt the stored canonical_cbor of seq 1 so its hash no longer matches.
    let mut steps = store.steps_for_trajectory("t1").unwrap();
    steps[1].canonical_cbor[0] ^= 0xff;
    let mut tampered = InMemoryTrajectoryStore::new();
    for s in steps {
        tampered.insert_step(s);
    }
    let v = verify_chain(&tampered, "t1").unwrap();
    assert!(!v.chain_valid);
    assert_eq!(v.broken_at, Some(1));
}

#[test]
fn missing_seq_fails() {
    let kp = Keypair::new();
    let mut store = InMemoryTrajectoryStore::new();
    let s0 = step_record("t1", 0, "a", None, &kp);
    let s2 = step_record("t1", 2, "c", Some(&s0.content_hash), &kp); // gap at 1
    store.insert_step(s0);
    store.insert_step(s2);
    let v = verify_chain(&store, "t1").unwrap();
    assert!(!v.chain_valid);
    assert_eq!(v.broken_at, Some(1));
}

#[test]
fn genesis_with_prev_hash_fails() {
    let kp = Keypair::new();
    let mut store = InMemoryTrajectoryStore::new();
    // seq 0 carrying a prev_hash is illegal.
    let s0 = step_record("t1", 0, "a", Some("deadbeef"), &kp);
    store.insert_step(s0);
    let v = verify_chain(&store, "t1").unwrap();
    assert!(!v.chain_valid);
    assert_eq!(v.broken_at, Some(0));
}

#[test]
fn verifies_against_in_memory_store_no_db() {
    // The whole suite uses InMemoryTrajectoryStore — this asserts the contract
    // explicitly: chain verification touches no SQLite/network.
    let kp = Keypair::new();
    let (store, _) = three_step_store(&kp);
    assert!(verify_chain(&store, "t1").unwrap().chain_valid);
}

#[test]
fn attest_step_links_prev_hash() {
    let kp = Keypair::new();
    let (_store, hashes) = three_step_store(&kp);
    // Rebuild seq 1 and confirm its prev_hash equals seq 0's content hash.
    let s1 = step_record("t1", 1, "tool call", Some(&hashes[0]), &kp);
    assert_eq!(s1.prev_hash.as_deref(), Some(hashes[0].as_str()));
}

#[test]
fn verdict_by_producer_rejected_from_coverage() {
    let kp = Keypair::new();
    let (mut store, hashes) = three_step_store(&kp);
    // Self-judged "pass" on every step — must NOT count.
    for h in &hashes {
        store.insert_verdict(verdict_record(h, VerdictStatus::Pass, &kp));
    }
    let cov = verdict_coverage(&store, "t1").unwrap();
    assert_eq!(cov.covered_steps, 0, "self-judged verdicts are ignored");
    assert!(!cov.full());
}

#[test]
fn independent_verdicts_give_full_coverage() {
    let kp = Keypair::new();
    let judge = Keypair::new();
    let (mut store, hashes) = three_step_store(&kp);
    for h in &hashes {
        store.insert_verdict(verdict_record(h, VerdictStatus::Pass, &judge));
    }
    let cov = verdict_coverage(&store, "t1").unwrap();
    assert!(cov.full());
    assert!(!cov.has_reject);
}

#[test]
fn verify_trajectory_happy_is_safe_to_settle() {
    let kp = Keypair::new();
    let judge = Keypair::new();
    let (mut store, hashes) = three_step_store(&kp);
    for h in &hashes {
        store.insert_verdict(verdict_record(h, VerdictStatus::Pass, &judge));
    }
    let report = build_report(&store, "t1").unwrap();
    assert!(report.chain_valid);
    assert_eq!(report.covered_steps, 3);
    assert!(report.safe_to_settle);
    assert_eq!(report.batch_root, merkle_root_hex(&hashes));
    // Every step has a valid inclusion proof against the batch root.
    for p in trajectory_proofs(&store, "t1").unwrap() {
        assert!(crate::merkle::verify(
            &p.content_hash,
            &p.proof,
            &crate::merkle::trajectory_root(&hashes)
        ));
    }
}

#[test]
fn reject_verdict_blocks_settle() {
    let kp = Keypair::new();
    let judge = Keypair::new();
    let (mut store, hashes) = three_step_store(&kp);
    // Full pass coverage…
    for h in &hashes {
        store.insert_verdict(verdict_record(h, VerdictStatus::Pass, &judge));
    }
    // …but one independent reject is a hard blocker.
    store.insert_verdict(verdict_record(&hashes[1], VerdictStatus::Reject, &judge));
    let report = build_report(&store, "t1").unwrap();
    assert!(report.chain_valid);
    assert!(report.has_reject);
    assert!(!report.safe_to_settle);
}

#[test]
fn broken_checkpoint_chain_fails() {
    let good = vec![
        ("root0".to_string(), None),
        ("root1".to_string(), Some("root0".to_string())),
        ("root2".to_string(), Some("root1".to_string())),
    ];
    assert!(verify_checkpoint_chain(&good));

    let broken = vec![
        ("root0".to_string(), None),
        ("root1".to_string(), Some("root0".to_string())),
        ("root2".to_string(), Some("WRONG".to_string())),
    ];
    assert!(!verify_checkpoint_chain(&broken));
}

fn merkle_root_hex(hashes: &[String]) -> String {
    crate::merkle::to_hex32(&crate::merkle::trajectory_root(hashes))
}
