//! Integration test for verifiable recall (Wave 5, design §16): build the
//! per-owner Merkle commitment from the (rebuildable) SQLite cache, then prove
//! and verify inclusion for each of an owner's memories — and prove that one
//! owner's member does NOT verify against another owner's root (the
//! omission/tamper-detection property an operator cannot forge).
//!
//! This exercises the chain-independent core of the scheme end-to-end against
//! the real store. Anchoring the root on Solana + distributing it to clients
//! is the operator-relay integration layer (requires a live validator) and is
//! out of scope for this pure-logic test.

use mnemonic_core::merkle::{commitment_root, prove, verify};
use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};

fn open_temp_store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("att.db");
    let store = SqliteStore::open(&path).expect("open store");
    (store, dir)
}

/// 64-hex content hash derived deterministically from a label.
fn ch(label: &str) -> String {
    blake3::hash(label.as_bytes()).to_hex().to_string()
}

#[allow(clippy::too_many_arguments)]
fn seed(store: &SqliteStore, owner: &str, id: &str, content_hash: &str) {
    store
        .save_attestation(
            id,
            "content",
            content_hash,
            &["t".to_string()],
            &format!("local:{id}-sol"),
            &format!("local:{id}-ar"),
            owner, // signer == owner (self-sovereign, post Wave 3)
            owner,
            "2026-01-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
            &[0.1, 0.2],
        )
        .expect("save row");
}

#[test]
fn recall_inclusion_proofs_verify_against_owner_commitment() {
    let (store, _dir) = open_temp_store();
    const OWNER: &str = "owner-A-pubkey";

    // Seed five memories for the owner.
    let hashes: Vec<String> = (0..5).map(|i| ch(&format!("A-mem-{i}"))).collect();
    for (i, h) in hashes.iter().enumerate() {
        seed(&store, OWNER, &format!("att-A-{i}"), h);
    }

    // Rebuild the commitment from the cache (order-independent).
    let from_store = store.owner_content_hashes(OWNER).expect("query hashes");
    assert_eq!(from_store.len(), 5);
    let root = commitment_root(&from_store);

    // Every member proves + verifies against the root.
    for h in &hashes {
        let (proof_root, proof) = prove(&from_store, h).expect("member must prove");
        assert_eq!(
            proof_root, root,
            "proof root must equal the commitment root"
        );
        assert!(verify(h, &proof, &root), "inclusion proof must verify");
    }

    // A hash the owner never wrote has no proof.
    assert!(prove(&from_store, &ch("A-never")).is_none());
}

#[test]
fn one_owners_member_does_not_verify_against_anothers_root() {
    let (store, _dir) = open_temp_store();
    const OWNER_A: &str = "owner-A-pubkey";
    const OWNER_B: &str = "owner-B-pubkey";

    let a_hash = ch("A-secret");
    seed(&store, OWNER_A, "att-A-0", &a_hash);
    seed(&store, OWNER_B, "att-B-0", &ch("B-secret"));
    seed(&store, OWNER_B, "att-B-1", &ch("B-public"));

    let a_hashes = store.owner_content_hashes(OWNER_A).expect("A hashes");
    let b_hashes = store.owner_content_hashes(OWNER_B).expect("B hashes");
    assert_eq!(a_hashes.len(), 1);
    assert_eq!(b_hashes.len(), 2);

    // A's proof verifies against A's root.
    let (a_root, a_proof) = prove(&a_hashes, &a_hash).expect("A proves");
    assert!(verify(&a_hash, &a_proof, &a_root));

    // ...but NOT against B's root — cross-tenant inclusion is unforgeable.
    let b_root = commitment_root(&b_hashes);
    assert_ne!(a_root, b_root);
    assert!(
        !verify(&a_hash, &a_proof, &b_root),
        "owner A's member must not appear included in owner B's commitment"
    );
    // And B's set has no proof for A's hash at all.
    assert!(prove(&b_hashes, &a_hash).is_none());
}

#[test]
fn empty_owner_has_no_members_and_a_stable_empty_root() {
    let (store, _dir) = open_temp_store();
    let hashes = store
        .owner_content_hashes("nobody-pubkey")
        .expect("query empty");
    assert!(hashes.is_empty());
    // Empty commitment is the fixed empty-root sentinel; no member proves.
    assert_eq!(commitment_root(&hashes), commitment_root(&[]));
    assert!(prove(&hashes, &ch("anything")).is_none());
}
