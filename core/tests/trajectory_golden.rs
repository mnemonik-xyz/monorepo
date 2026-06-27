//! Golden vectors for verifiable trajectories — byte-for-byte parity targets
//! for any third-party (SDK / client) implementation. A fixed Ed25519 seed
//! makes the whole pipeline deterministic (Ed25519 signatures are deterministic
//! per RFC 8032), so canonical CBOR, content hashes, and the order-preserving
//! batch root are stable known answers.
//!
//! Run: `cargo test --features trajectory-experimental -p mnemonic-core
//! --test trajectory_golden`
#![cfg(feature = "trajectory-experimental")]

use mnemonic_core::merkle;
use mnemonic_core::trajectory::{
    build_report, build_step, trajectory_proofs, InMemoryTrajectoryStore, StepInput, StepRecord,
};
use solana_sdk::signature::{Keypair, Signer};

const SEED_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const TS: &str = "2026-06-27T00:00:00Z";
const TID: &str = "traj:golden";

fn fixed_keypair() -> Keypair {
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&hex::decode(SEED_HEX).unwrap());
    Keypair::new_from_array(seed)
}

fn record(seq: u64, content: &str, prev: Option<&str>, kp: &Keypair) -> StepRecord {
    let s = build_step(
        &StepInput {
            trajectory_id: TID,
            seq,
            content,
            prev_hash: prev,
            created_at: TS,
        },
        kp,
    )
    .unwrap();
    StepRecord {
        trajectory_id: TID.to_string(),
        seq,
        content_hash: s.content_hash,
        prev_hash: prev.map(str::to_string),
        producer: kp.pubkey().to_string(),
        cose_bytes: s.cose_bytes,
        canonical_cbor: s.canonical_cbor,
    }
}

/// Known-answer hashes of the 3 golden steps. These MUST NOT change without a
/// schema-version bump — a diff here means the canonicalization or signing
/// pipeline drifted and every existing client breaks.
const STEP0_HASH: &str = "ae9d5a052083e070987843386f35b932d9dd8d55439277f64cae7f0ec52e0e43";
/// Known-answer batch root over the 3 golden steps (order-preserving).
const GOLDEN_BATCH_ROOT: &str = "c98412432036948bdf9c2ea4b2bca5be5ee4ee3664357c085fac8f2edec065e7";

#[test]
fn golden_pipeline_is_deterministic_and_consistent() {
    // Build twice — identical bytes (determinism contract).
    let kp = fixed_keypair();
    let a = record(0, "plan the task", None, &kp);
    let b = record(0, "plan the task", None, &kp);
    assert_eq!(a.content_hash, b.content_hash, "content hash deterministic");
    assert_eq!(a.cose_bytes, b.cose_bytes, "COSE bytes deterministic");
    assert_eq!(
        a.content_hash,
        mnemonic_core::codec::hash::hash_bytes(&a.canonical_cbor),
        "content_hash == blake3(canonical_cbor)"
    );
    // Known-answer: the genesis step's hash is frozen across versions.
    assert_eq!(
        a.content_hash, STEP0_HASH,
        "golden genesis step hash drifted"
    );
}

#[test]
fn golden_batch_root_equals_manifest_root_and_proofs_verify() {
    let kp = fixed_keypair();
    let s0 = record(0, "plan the task", None, &kp);
    let s1 = record(1, "call search tool", Some(&s0.content_hash), &kp);
    let s2 = record(2, "synthesize answer", Some(&s1.content_hash), &kp);
    let hashes = vec![
        s0.content_hash.clone(),
        s1.content_hash.clone(),
        s2.content_hash.clone(),
    ];

    let mut store = InMemoryTrajectoryStore::new();
    for s in [s0, s1, s2] {
        store.insert_step(s);
    }

    let report = build_report(&store, TID).unwrap();
    // The report's batch_root is exactly the order-preserving Merkle root.
    let manifest_root = merkle::to_hex32(&merkle::trajectory_root(&hashes));
    assert_eq!(report.batch_root, manifest_root);
    assert_eq!(
        report.batch_root, GOLDEN_BATCH_ROOT,
        "golden batch root drifted"
    );
    assert!(report.chain_valid);

    // Every step proves inclusion against the published batch root.
    let root = merkle::trajectory_root(&hashes);
    for p in trajectory_proofs(&store, TID).unwrap() {
        assert!(
            merkle::verify(&p.content_hash, &p.proof, &root),
            "seq {} inclusion proof must verify against batch root",
            p.seq
        );
    }
}

/// Emit the golden fixtures as JSON for SDK parity tests. Ignored by default;
/// run with `-- --ignored --nocapture` to print.
#[test]
#[ignore]
fn emit_golden_fixtures() {
    let kp = fixed_keypair();
    let s0 = record(0, "plan the task", None, &kp);
    let s1 = record(1, "call search tool", Some(&s0.content_hash), &kp);
    let s2 = record(2, "synthesize answer", Some(&s1.content_hash), &kp);
    let hashes = vec![
        s0.content_hash.clone(),
        s1.content_hash.clone(),
        s2.content_hash.clone(),
    ];
    let root = merkle::to_hex32(&merkle::trajectory_root(&hashes));
    for s in [&s0, &s1, &s2] {
        println!(
            "{{\"seq\":{},\"content_hash\":\"{}\",\"canonical_cbor_hex\":\"{}\",\"cose_hex\":\"{}\"}}",
            s.seq,
            s.content_hash,
            hex::encode(&s.canonical_cbor),
            hex::encode(&s.cose_bytes)
        );
    }
    println!(
        "{{\"batch_root\":\"{root}\",\"producer\":\"{}\"}}",
        kp.pubkey()
    );
    println!("STEP0_HASH={}", s0.content_hash);
}
