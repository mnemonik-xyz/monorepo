//! Restoring a recall index from Arweave (work/arweave-as-source-of-truth Wave 3).
//!
//! The network phases (`enumerate_anchored`, `fetch_restorable`) need a gateway
//! and an RPC node, so they are covered by the mcp integration suite against
//! stubs. What matters here is `apply_restore`: it is the part that decides what
//! lands in a store and under which labels, and it is synchronous, so it can be
//! tested exactly.

use base64::Engine as _;
use mnemonic_core::codec::{schema, sign::sign_artifact};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::identity;
use mnemonic_core::rebuild::rebuild_row_self_describing;
use mnemonic_core::restore::{apply_restore, RestorableItem};
use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
use solana_sdk::signature::Keypair;

const DIM: usize = 8;
const BITS: usize = 4;
const SEED: u64 = 42;

fn compressor() -> EmbeddingCompressor {
    EmbeddingCompressor::new(DIM, BITS, SEED)
}

/// Build an anchored artifact the way `sign_memory` does after Wave 1, then turn
/// it into a `RestorableItem` the way `fetch_restorable` would.
fn restorable(
    kp: &Keypair,
    id: &str,
    content: &str,
    visibility: Option<&str>,
    arweave_tx: &str,
    solana_tx: Option<&str>,
) -> RestorableItem {
    let c = compressor();
    let embedding: Vec<f32> = (0..DIM).map(|i| i as f32 * 0.1).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(c.compress(&embedding).to_bytes());
    let mut artifact = serde_json::json!({
        "artifact_id": id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": identity::did_sol(kp),
        "created_at": "2026-09-27T00:00:00Z",
        "tags": ["restored"],
        "metadata": {
            "embed_provider": "stub",
            "embed_dim": DIM,
            "turbo_bits": BITS,
            "turbo_seed": SEED,
            "embedding_compressed": b64,
        },
    });
    if let Some(v) = visibility {
        artifact["visibility"] = serde_json::json!(v);
        artifact["anchor"] = serde_json::json!("arweave");
    }
    let signed = sign_artifact(&artifact, &schema::MEMORY_V1, kp).expect("sign");
    let row = rebuild_row_self_describing(&signed.cose_bytes).expect("rebuild");
    RestorableItem {
        arweave_tx: arweave_tx.to_string(),
        solana_tx: solana_tx.map(str::to_string),
        row,
    }
}

#[test]
fn restores_only_rows_authored_by_the_requesting_identity() {
    // The enumeration sources are wallet-scoped, not identity-scoped: the
    // operator's fee-payer wallet anchored items for many identities. A restore
    // must not import someone else's memories.
    let me = Keypair::new();
    let someone_else = Keypair::new();
    let mine = identity::pubkey_base58(&me);

    let items = vec![
        restorable(&me, "a1", "mine one", Some("public"), "ar-1", Some("sol-1")),
        restorable(
            &someone_else,
            "b1",
            "not mine",
            Some("public"),
            "ar-2",
            None,
        ),
        restorable(&me, "a2", "mine two", Some("private"), "ar-3", None),
    ];

    let store = SqliteStore::in_memory().unwrap();
    let report = apply_restore(&store, &mine, &items);

    assert_eq!(report.enumerated, 3);
    assert_eq!(report.restored, 2);
    assert_eq!(report.skipped_other_owner, 1);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(store.count(&mine).unwrap(), 2);
}

#[test]
fn restoring_twice_converges_instead_of_duplicating() {
    let kp = Keypair::new();
    let owner = identity::pubkey_base58(&kp);
    let items = vec![restorable(
        &kp,
        "same-id",
        "written once",
        Some("public"),
        "ar-1",
        Some("sol-1"),
    )];

    let store = SqliteStore::in_memory().unwrap();
    let first = apply_restore(&store, &owner, &items);
    let second = apply_restore(&store, &owner, &items);

    assert_eq!(first.restored, 1);
    assert_eq!(second.restored, 1, "a second pass still writes the row");
    assert_eq!(
        store.count(&owner).unwrap(),
        1,
        "but it must not create a second row"
    );
}

#[test]
fn the_signed_visibility_label_survives_on_the_artifact() {
    // Wave 1 put `visibility` inside the signed payload so a restore does not
    // have to guess. Prove the label round-trips through signing and rebuild,
    // independently of what the storage layer then decides to store (see the
    // D-8 test below).
    let kp = Keypair::new();
    let pubv = restorable(&kp, "pub", "public memory", Some("public"), "ar-1", None);
    let privv = restorable(&kp, "priv", "private memory", Some("private"), "ar-2", None);
    assert_eq!(pubv.row.visibility.as_deref(), Some("public"));
    assert_eq!(privv.row.visibility.as_deref(), Some("private"));
    assert_eq!(pubv.row.anchor.as_deref(), Some("arweave"));
}

#[test]
fn restore_cannot_resurrect_a_private_label_for_plaintext_on_arweave() {
    // Owner decision D-8: content anchored on Arweave is plain text anyone can
    // read, so the server must not label it private. A restore is bound by the
    // same truth — the bytes are public whatever the author signed. Until sealed
    // mode ships there is no such thing as a private anchored memory, and this
    // test is what stops a future change from quietly pretending otherwise.
    let kp = Keypair::new();
    let owner = identity::pubkey_base58(&kp);
    let items = vec![restorable(
        &kp,
        "priv-anchored",
        "author asked for private",
        Some("private"),
        "ar-real",
        None,
    )];

    let store = SqliteStore::in_memory().unwrap();
    assert_eq!(apply_restore(&store, &owner, &items).restored, 1);

    let listed = store.list_public_artifacts(10).unwrap();
    let row = listed
        .iter()
        .find(|r| r.attestation_id == "priv-anchored")
        .expect("an anchored row with real Arweave bytes is public, per D-8");
    assert!(
        row.plaintext_on_arweave,
        "and it must be flagged as plaintext on Arweave"
    );
}

#[test]
fn an_unlabelled_row_defaults_to_private_when_d8_does_not_apply() {
    // D-8 only overrides rows whose bytes really reached Arweave. For anything
    // else the default must be the non-disclosing side: hiding something meant
    // to be shared is correctable, publishing something private is not.
    let kp = Keypair::new();
    let owner = identity::pubkey_base58(&kp);
    let items = vec![restorable(
        &kp,
        "legacy",
        "no label",
        None,
        "local:synthetic",
        None,
    )];

    let store = SqliteStore::in_memory().unwrap();
    assert_eq!(apply_restore(&store, &owner, &items).restored, 1);
    assert!(
        store.list_public_artifacts(10).unwrap().is_empty(),
        "an unlabelled artifact must never default to public"
    );
}

#[test]
fn an_unknown_visibility_label_is_reported_not_guessed() {
    let kp = Keypair::new();
    let owner = identity::pubkey_base58(&kp);
    let items = vec![restorable(
        &kp,
        "weird",
        "odd label",
        Some("semi"),
        "ar-1",
        None,
    )];

    let store = SqliteStore::in_memory().unwrap();
    let report = apply_restore(&store, &owner, &items);

    assert_eq!(report.restored, 0);
    assert_eq!(report.failed.len(), 1);
    assert!(
        report.failed[0].1.contains("visibility"),
        "{:?}",
        report.failed
    );
    assert_eq!(store.count(&owner).unwrap(), 0);
}

#[test]
fn restored_rows_carry_the_anchor_mode_and_their_chain_ids() {
    let kp = Keypair::new();
    let owner = identity::pubkey_base58(&kp);
    let items = vec![restorable(
        &kp,
        "anchored-row",
        "on chain",
        Some("public"),
        "ar-xyz",
        Some("sol-abc"),
    )];

    let store = SqliteStore::in_memory().unwrap();
    assert_eq!(apply_restore(&store, &owner, &items).restored, 1);

    let row = store
        .find_by_tx("sol-abc", &owner)
        .unwrap()
        .expect("row is findable by its memo tx");
    assert_eq!(row.arweave_tx, "ar-xyz");
    assert_eq!(
        store.find_write_mode_by_tx("sol-abc", &owner).unwrap(),
        Some(WriteMode::Anchored),
        "a restored row is anchored, not local"
    );
    let _ = Visibility::Public; // keeps the import meaningful if asserts change
}
