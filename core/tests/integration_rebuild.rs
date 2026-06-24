//! Integration test for rebuild-index-from-stored-artifacts (Wave 5, §16).
//!
//! Signs real artifacts (the same shape `sign_memory` produces — content +
//! `metadata.embedding_compressed`), then reconstructs the recall-index rows
//! from the signed COSE bytes alone and proves:
//!
//!   1. Lossless fields (content, owner, signer, tags, content_hash) round-trip
//!      exactly.
//!   2. A recall over the **rebuilt** store returns the same top-1 as a recall
//!      over the **original** store — the index is functionally reconstructable
//!      from durable artifacts (SQLite is a rebuildable cache, not a trust root).
//!   3. The rebuilt embedding is *lossy* (only the TurboQuant-compressed copy is
//!      in the artifact) — the concrete precision gap the opt-in f32 tier closes.
//!   4. A tampered artifact is rejected (no row injected without a valid sig).

use base64::Engine;
use mnemonic_core::codec::{schema, sign::sign_artifact};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::identity;
use mnemonic_core::rebuild::{rebuild_row, rebuild_rows};
use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
use solana_sdk::signature::Keypair;

const DIM: usize = 8;
const BITS: usize = 4;
const SEED: u64 = 42; // protocol constant (matches mcp main.rs)

fn compressor() -> EmbeddingCompressor {
    EmbeddingCompressor::new(DIM, BITS, SEED)
}

/// Build a signed artifact the way `sign_memory` does. Returns
/// `(cose_bytes, content_hash, original_embedding)`.
fn make_artifact(
    kp: &Keypair,
    c: &EmbeddingCompressor,
    id: &str,
    content: &str,
    embedding: &[f32],
    tags: &[&str],
) -> (Vec<u8>, String, Vec<f32>) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(c.compress(embedding).to_bytes());
    let artifact = serde_json::json!({
        "artifact_id": id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": identity::did_sol(kp),
        "created_at": "2026-01-01T00:00:00Z",
        "tags": tags,
        "metadata": {
            "embed_provider": "stub",
            "embed_dim": DIM,
            "turbo_bits": BITS,
            "embedding_compressed": b64,
        },
    });
    let signed = sign_artifact(&artifact, &schema::MEMORY_V1, kp).expect("sign");
    (signed.cose_bytes, signed.content_hash, embedding.to_vec())
}

#[test]
fn rebuild_recovers_signed_fields_losslessly() {
    let kp = Keypair::new();
    let c = compressor();
    let owner = identity::pubkey_base58(&kp);
    let emb = vec![0.9, 0.1, 0.0, 0.3, 0.7, 0.2, 0.5, 0.4];

    let (cose, content_hash, _orig) =
        make_artifact(&kp, &c, "art-1", "hello rebuild", &emb, &["a", "b"]);

    let row = rebuild_row(&cose, &c).expect("rebuild");
    assert_eq!(row.attestation_id, "art-1");
    assert_eq!(row.content, "hello rebuild");
    assert_eq!(row.owner_pubkey, owner);
    assert_eq!(row.signer_pubkey, owner, "self-sovereign: signer == owner");
    assert_eq!(row.tags, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        row.content_hash, content_hash,
        "content_hash is recomputed, lossless"
    );
    assert_eq!(row.embedding.len(), DIM);
}

#[test]
fn recall_over_rebuilt_store_matches_original() {
    let kp = Keypair::new();
    let c = compressor();
    let owner = identity::pubkey_base58(&kp);

    // Three well-separated memories so compression can't reorder top-1.
    let memos: [(&str, &str, Vec<f32>); 3] = [
        (
            "art-x",
            "about cats",
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ),
        (
            "art-y",
            "about ships",
            vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ),
        (
            "art-z",
            "about stars",
            vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ),
    ];

    let original = SqliteStore::in_memory().expect("orig store");
    let rebuilt = SqliteStore::in_memory().expect("rebuilt store");

    for (id, content, emb) in &memos {
        // ORIGINAL store: persisted with the true f32 embedding (what
        // sign_memory writes to SQLite today).
        original
            .save_attestation(
                id,
                content,
                &format!("h-{id}"),
                &["t".to_string()],
                &format!("local:{id}"),
                &format!("local:{id}"),
                &owner,
                &owner,
                "2026-01-01T00:00:00Z",
                WriteMode::Participate,
                Visibility::Public,
                emb,
            )
            .expect("orig save");

        // REBUILT store: persisted with the embedding reconstructed from the
        // signed artifact alone.
        let (cose, content_hash, _) = make_artifact(&kp, &c, id, content, emb, &["t"]);
        let row = rebuild_row(&cose, &c).expect("rebuild");
        rebuilt
            .save_attestation(
                &row.attestation_id,
                &row.content,
                &content_hash,
                &row.tags,
                &format!("local:{id}"),
                &format!("local:{id}"),
                &row.signer_pubkey,
                &row.owner_pubkey,
                &row.created_at,
                WriteMode::Participate,
                Visibility::Public,
                &row.embedding,
            )
            .expect("rebuilt save");
    }

    // Querying each memory's own embedding must return that memory as top-1 in
    // BOTH stores — the rebuilt index is functionally equivalent for recall.
    for (id, content, emb) in &memos {
        let a = original
            .search(emb, Some(&owner), None, 1)
            .expect("orig search");
        let b = rebuilt
            .search(emb, Some(&owner), None, 1)
            .expect("rebuilt search");
        assert_eq!(a[0].content, *content, "original top-1 for {id}");
        assert_eq!(
            b[0].content, *content,
            "rebuilt store must return the same top-1 for {id}"
        );
    }
}

#[test]
fn rebuilt_embedding_is_lossy_but_content_is_exact() {
    // This is the precision gap that motivates the f32 tier: content/integrity
    // survive exactly, but the embedding recovered from the artifact is only
    // the dequantized approximation.
    let kp = Keypair::new();
    let c = compressor();
    let emb = vec![0.83, -0.21, 0.44, 0.12, -0.67, 0.05, 0.91, -0.38];

    let (cose, content_hash, orig) = make_artifact(&kp, &c, "art-p", "precision", &emb, &["t"]);
    let row = rebuild_row(&cose, &c).expect("rebuild");

    // Lossless: the content hash matches exactly.
    assert_eq!(row.content_hash, content_hash);

    // Lossy: the reconstructed embedding drifts from the original.
    let drift: f32 = orig
        .iter()
        .zip(&row.embedding)
        .map(|(a, b)| (a - b).abs())
        .sum();
    assert!(
        drift > 0.0,
        "TurboQuant-compressed embedding must be lossy on rebuild (drift={drift}) \
         — this is exactly what the f32-in-artifact tier would eliminate"
    );
}

#[test]
fn tampered_artifact_is_rejected() {
    let kp = Keypair::new();
    let c = compressor();
    let emb = vec![0.5; DIM];
    let (mut cose, _h, _e) = make_artifact(&kp, &c, "art-t", "tamper", &emb, &["t"]);

    // Flip a byte in the signed envelope.
    let mid = cose.len() / 2;
    cose[mid] ^= 0xff;

    assert!(
        rebuild_row(&cose, &c).is_err(),
        "a tampered artifact must not yield a rebuilt row"
    );

    // Batch rebuild reports the bad one and keeps going.
    let (good_cose, _, _) = make_artifact(&kp, &c, "art-ok", "fine", &emb, &["t"]);
    let (ok, errs) = rebuild_rows(&[cose, good_cose], &c);
    assert_eq!(ok.len(), 1, "one good row survives");
    assert_eq!(errs.len(), 1, "one bad artifact reported");
    assert_eq!(errs[0].0, 0, "the tampered artifact is at index 0");
}
