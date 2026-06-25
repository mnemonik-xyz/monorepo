//! Integration test for W5(b): `tools::recall` returns a per-owner Merkle
//! commitment (root + one inclusion proof per result), and each proof verifies
//! against the root via the public `merkle::verify` a client would run. This
//! ties the verifiable-recall library primitive (Wave 5) to the actual recall
//! tool output — the operator-omission/tamper detectability surfaced end to end
//! (the only remaining piece is anchoring the root on Solana, which needs a
//! validator and is out of scope here).
//!
//! Requires `--features test-support` for `StubEmbedder`.

use mnemonic_core::merkle::{self, ProofStep};
use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
use mnemonic_mcp::test_support::StubEmbedder;
use mnemonic_mcp::tools;
use solana_sdk::signature::Keypair;

const OWNER: &str = "owner-recall-merkle";

#[allow(clippy::too_many_arguments)]
fn seed(store: &SqliteStore, id: &str, content_hash: &str) {
    store
        .save_attestation(
            id,
            "content",
            content_hash,
            &["t".to_string()],
            &format!("local:{id}-sol"),
            &format!("local:{id}-ar"),
            OWNER,
            OWNER,
            "2026-01-01T00:00:00Z",
            WriteMode::Participate,
            Visibility::Public,
            &[0.1; 8], // matches StubEmbedder's query vector → all rows match
        )
        .expect("save row");
}

fn ch(label: &str) -> String {
    blake3::hash(label.as_bytes()).to_hex().to_string()
}

/// Rebuild `Vec<ProofStep>` from the JSON proof a client receives.
fn parse_proof(steps: &serde_json::Value) -> Vec<ProofStep> {
    steps
        .as_array()
        .expect("proof is an array")
        .iter()
        .map(|s| ProofStep {
            sibling: merkle::parse_hex32(s["sibling"].as_str().expect("sibling hex"))
                .expect("valid sibling hex"),
            sibling_is_right: s["right"].as_bool().expect("right bool"),
        })
        .collect()
}

#[test]
fn recall_returns_verifiable_merkle_proofs_per_result() {
    let store = SqliteStore::in_memory().expect("store");
    let kp = Keypair::new();
    let embedder = StubEmbedder::default();

    let hashes: Vec<String> = (0..4).map(|i| ch(&format!("m{i}"))).collect();
    for (i, h) in hashes.iter().enumerate() {
        seed(&store, &format!("att-{i}"), h);
    }

    let out = tools::recall(&kp, &store, &embedder, "anything", 10, Some(OWNER), None);

    let commitment = &out["merkle_commitment"];
    assert!(
        commitment.is_object(),
        "authenticated recall must carry a commitment"
    );
    let root_hex = commitment["root"].as_str().expect("root hex");
    let root = merkle::parse_hex32(root_hex).expect("root parses");

    // The root equals an independent recomputation over the owner's set.
    assert_eq!(root, merkle::commitment_root(&hashes));

    // Every returned result has a proof that verifies against the root.
    let results = out["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "expected matching rows");
    for r in results {
        let h = r["content_hash"].as_str().expect("result content_hash");
        let proof_json = &commitment["proofs"][h];
        assert!(
            !proof_json.is_null(),
            "every result must carry a proof: {h}"
        );
        let proof = parse_proof(proof_json);
        assert!(
            merkle::verify(h, &proof, &root),
            "inclusion proof for {h} must verify against the recall root"
        );
    }
}

#[test]
fn tampered_proof_from_recall_does_not_verify() {
    let store = SqliteStore::in_memory().expect("store");
    let kp = Keypair::new();
    let embedder = StubEmbedder::default();
    for i in 0..4 {
        seed(&store, &format!("att-{i}"), &ch(&format!("m{i}")));
    }

    let out = tools::recall(&kp, &store, &embedder, "anything", 10, Some(OWNER), None);
    let commitment = &out["merkle_commitment"];
    let root = merkle::parse_hex32(commitment["root"].as_str().unwrap()).unwrap();

    let results = out["results"].as_array().unwrap();
    let r = &results[0];
    let h = r["content_hash"].as_str().unwrap();
    let mut proof = parse_proof(&commitment["proofs"][h]);
    assert!(merkle::verify(h, &proof, &root));

    // A client that received a tampered proof (or wrong root) rejects it.
    if let Some(step) = proof.first_mut() {
        step.sibling[0] ^= 0xff;
        assert!(
            !merkle::verify(h, &proof, &root),
            "tampered proof must fail"
        );
    } else {
        // Single-leaf tree (empty proof): a wrong root must still fail.
        let wrong = merkle::commitment_root(&[ch("other")]);
        assert!(!merkle::verify(h, &proof, &wrong));
    }
}

#[test]
fn anonymous_pool_recall_has_no_single_owner_commitment() {
    let store = SqliteStore::in_memory().expect("store");
    let kp = Keypair::new();
    let embedder = StubEmbedder::default();
    seed(&store, "att-0", &ch("m0"));

    // Anonymous cross-owner public pool: owner_pubkey = None.
    let out = tools::recall(
        &kp,
        &store,
        &embedder,
        "anything",
        10,
        None,
        Some(Visibility::Public),
    );
    assert!(
        out["merkle_commitment"].is_null(),
        "no single-owner commitment exists for the anonymous pool"
    );
}
