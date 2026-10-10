//! Arweave is the single record of an anchored memory.
//!
//! 1. `anchored_write_stores_no_solana_reference` — an anchored stdio write
//!    uploads to Arweave, stores the legacy `solana_tx` column as an empty
//!    string and returns no Solana field.
//!
//! 2. `anchored_rows_verify_from_arweave` — `mnemonic_verify` fetches the
//!    original bytes from Arweave and checks the COSE signature. A legacy row
//!    whose unused `solana_tx` column is not empty verifies the same way.
//!
//! 3. `snapshot_lists_graphql_items` — `snapshot_chain` (recovery) lists
//!    the items that gateway GraphQL returns and decodes their payloads.

mod _helpers;

use httpmock::prelude::*;
use mnemonic_core::{
    arweave::{
        graphql::GraphQlClient,
        recovery::{snapshot_chain, ChainSnapshot},
        ArweaveClient,
    },
    codec::{schema, sign::sign_artifact},
    storage::{AttestationStore, Visibility, WriteMode},
};
use mnemonic_mcp::{
    test_support::mock_state_with,
    tools::{resolve_write_mode, sign_memory, Transport},
};
use serde_json::json;
use solana_sdk::signature::{Keypair, Signer};
use std::{sync::Arc, time::Duration};

// ── 1. Anchored write stores no Solana reference ─────────────────────────────

/// Wires a mock Arweave (POST /tx → 200, GET /tx → 404) and asserts that an
/// anchored stdio write:
///   (a) makes the Arweave upload (POST /tx is called),
///   (b) stores the row with solana_tx = '' (empty string),
///   (c) the error — if any — is DeliveryNotConfirmed (Arweave refetch
///       timed out at 0ms budget), and it names no Solana transaction.
#[tokio::test]
async fn anchored_write_stores_no_solana_reference() {
    // Arweave mock — POST /tx returns a tx id; GET returns 404 so the delivery
    // check exhausts its 0-ms budget and returns DeliveryNotConfirmed. That is
    // the ONLY error the write path is allowed to produce.
    let arweave_server = MockServer::start();
    let fixed_ar_tx = "STAGE2_AR_TX_NO_MEMO";

    // POST /tx — accept any upload.
    let post_tx_mock = arweave_server.mock(|when, then| {
        when.method(POST).path("/tx");
        then.status(200)
            .header("content-type", "application/json")
            .body(format!(r#"{{"id":"{fixed_ar_tx}"}}"#));
    });

    // GET /mine — arlocal noop.
    arweave_server.mock(|when, then| {
        when.method(GET).path("/mine");
        then.status(200).body("mined");
    });

    // GET /<any tx id> → 404. With a 0-ms delivery timeout the refetch budget
    // exhausts immediately and the delivery check returns stage="refetch".
    let any_path_re = regex::Regex::new(r"^/[A-Za-z0-9_\-]+$").expect("valid regex");
    arweave_server.mock(|when, then| {
        when.method(GET).path_matches(any_path_re);
        then.status(404).body("not found");
    });

    // Build a SqliteStore + sign_memory inputs.
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.into_temp_path();
    let path_buf = path.keep().expect("keep tempfile");
    let store = mnemonic_core::storage::SqliteStore::open(&path_buf).expect("sqlite");
    mnemonic_mcp::paid_operation::migrate_paid_operations(store.conn())
        .expect("migrate paid_operations");
    mnemonic_mcp::paid_artifact::migrate_paid_artifact_staging(store.conn())
        .expect("migrate paid artifact staging");
    mnemonic_mcp::payment::migrate_free_anchor_usage(store.conn())
        .expect("migrate free anchor usage");

    let compressor = mnemonic_core::compress::EmbeddingCompressor::new(8, 4, 42);
    let embedder = Box::new(mnemonic_mcp::test_support::StubEmbedder::default());
    let arweave = ArweaveClient::new(&arweave_server.base_url());
    let keypair_raw = Keypair::new();
    let keypair = mnemonic_core::identity::LazyKeypair::ready(keypair_raw);
    let owner = keypair.pubkey_base58();
    let pending = Arc::new(mnemonic_mcp::pending::PendingBundles::with_defaults());
    let store_mutex = std::sync::Mutex::new(store);
    let cost_hint = mnemonic_mcp::pricing::CostHint {
        storage_cost_micro_usdc: 0,
        sol_tx_fee_lamports: 0,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    };
    let envelope = mnemonic_mcp::mcp::Envelope::from_config("full", "none", 0);

    let result = sign_memory(
        &keypair,
        &arweave,
        &store_mutex,
        embedder.as_ref(),
        &compressor,
        &pending,
        "arweave-only anchored content",
        &[],
        &cost_hint,
        "full",
        &owner,
        None, // no JWT → stdio path
        Transport::Stdio,
        resolve_write_mode(Some(&json!("anchored")), "full").expect("resolve anchored"),
        Visibility::Public,
        &envelope,
        Duration::ZERO, // zero timeout → delivery refetch exhausts instantly → row demoted
        false,
        "",
        &reqwest::Client::new(),
        &json!({}),
    )
    .await;

    // ── Assertion 1: Arweave upload DID happen.
    assert!(
        post_tx_mock.calls() >= 1,
        "expected at least one Arweave POST /tx; sign_memory must upload the COSE bytes"
    );

    // ── Assertion 2: the persisted row has an empty legacy solana_tx column.
    let store_g = store_mutex.lock().unwrap();
    let rows: Vec<(String, String)> = {
        let conn = store_g.conn();
        let mut stmt = conn
            .prepare("SELECT solana_tx, write_mode FROM attestations")
            .expect("prepare");
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
    };
    drop(store_g);

    assert_eq!(
        rows.len(),
        1,
        "expected exactly one attestation row; got {rows:?}"
    );
    let (solana_tx, _write_mode) = &rows[0];
    assert_eq!(
        solana_tx, "",
        "new anchored rows must store solana_tx = '' (empty string); got {solana_tx:?}"
    );

    // ── Assertion 3: the error is DeliveryNotConfirmed (stage=refetch), and
    // no response names a Solana transaction.
    match &result {
        Ok(envelope) => {
            // Unlikely with a 0-ms budget and a 404 GET, but not impossible
            // on a very fast machine.
            assert!(envelope.get("solana_tx").is_none(), "{envelope}");
            assert!(envelope.get("solana_explorer_url").is_none(), "{envelope}");
        }
        Err(mnemonic_mcp::tools::ToolError::TypedRpc(rpc)) => {
            assert_eq!(
                rpc.code, -32011,
                "expected DeliveryNotConfirmed (-32011), got code {} message {:?}",
                rpc.code, rpc.message
            );
            let data = rpc.data.as_ref().expect("typed error data");
            assert!(data.get("solana_tx").is_none(), "{data}");
            assert!(
                !rpc.message.to_lowercase().contains("solana"),
                "error must not reference Solana; got: {}",
                rpc.message
            );
        }
        Err(mnemonic_mcp::tools::ToolError::Other(e)) => {
            panic!(
                "sign_memory failed with non-typed error (expected DeliveryNotConfirmed): {e:#}"
            );
        }
    }
}

// ── 2. Anchored rows verify from Arweave ─────────────────────────────────────

/// Seeds anchored rows and asserts that `mnemonic_verify` resolves them to
/// `status: "verified"` from the Arweave bytes alone. One row has the new
/// shape (empty `solana_tx`). One row has a legacy non-empty `solana_tx`,
/// which verify ignores.
#[tokio::test]
async fn anchored_rows_verify_from_arweave() {
    let kp = Keypair::new();
    let owner_pubkey = kp.pubkey().to_string();
    let state = mock_state_with("full", "none", 0);
    let arweave_server = MockServer::start();
    let embedder = Box::new(mnemonic_mcp::test_support::StubEmbedder::default());
    let compressor = mnemonic_core::compress::EmbeddingCompressor::new(8, 4, 42);

    for (id, legacy_solana_column, ar_tx) in [
        ("new-att-id", "", "NEW_AR_TX_AAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        (
            "legacy-att-id",
            "LEGACY_SOL_SIG_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "LEGACY_AR_TX_AAAAAAAAAAAAAAAAAAAAAA",
        ),
    ] {
        let content = format!("anchored memory {id}");
        let artifact = serde_json::json!({
            "artifact_id": id,
            "type": "memory",
            "schema_version": 1,
            "content": content,
            "producer": format!("did:sol:{owner_pubkey}"),
            "created_at": "2026-01-01T00:00:00Z",
            "tags": ["anchored"],
        });
        let signed = sign_artifact(&artifact, &schema::MEMORY_V1, &kp).expect("sign artifact");
        let content_hash = signed.content_hash.clone();
        let cose_bytes = signed.cose_bytes.clone();
        arweave_server.mock(|when, then| {
            when.method(GET).path(format!("/{ar_tx}"));
            then.status(200).body(cose_bytes);
        });
        {
            let store_g = state.store.lock().unwrap();
            store_g
                .save_attestation(
                    id,
                    &content,
                    &content_hash,
                    &["anchored".to_string()],
                    legacy_solana_column,
                    ar_tx,
                    &owner_pubkey,
                    &owner_pubkey,
                    "2026-01-01T00:00:00Z",
                    WriteMode::Anchored,
                    Visibility::Private,
                    &[0.1f32; 8],
                )
                .expect("seed row");
        }

        let result = mnemonic_mcp::tools::verify(
            &ArweaveClient::new(&arweave_server.base_url()),
            &state.store,
            Some(ar_tx),
            &owner_pubkey,
            "full",
            embedder.as_ref(),
            &compressor,
        )
        .await
        .expect("verify must not return anyhow::Error");

        assert_eq!(result["status"].as_str(), Some("verified"), "{result}");
        assert_eq!(result["checks"]["cose_signature"], true, "{result}");
        assert_eq!(
            result["content_hash"].as_str(),
            Some(content_hash.as_str()),
            "content_hash must match the seeded value"
        );
        assert!(result.get("solana_tx").is_none(), "{result}");

        // The legacy Solana id is not a lookup key.
        if !legacy_solana_column.is_empty() {
            let miss = mnemonic_mcp::tools::verify(
                &ArweaveClient::new(&arweave_server.base_url()),
                &state.store,
                Some(legacy_solana_column),
                &owner_pubkey,
                "full",
                embedder.as_ref(),
                &compressor,
            )
            .await
            .expect("verify");
            assert_eq!(miss["status"].as_str(), Some("not_found"), "{miss}");
        }
    }
}

// ── 3. Snapshot lists GraphQL items ──────────────────────────────────────────

/// `snapshot_chain` lists each item that gateway GraphQL returns, takes the
/// producer from the tag when present, and otherwise decodes it from the
/// payload. The content hash comes from the payload.
#[tokio::test]
async fn snapshot_lists_graphql_items() {
    let kp = Keypair::new();
    let owner_did = format!("did:sol:{}", kp.pubkey());

    let artifact_tagged = serde_json::json!({
        "artifact_id": "restore-tagged",
        "type": "memory",
        "schema_version": 1,
        "content": "tagged memory",
        "producer": &owner_did,
        "created_at": "2026-09-28T00:00:00Z",
        "tags": ["restore-tagged"],
    });
    let artifact_untagged = serde_json::json!({
        "artifact_id": "restore-untagged",
        "type": "memory",
        "schema_version": 1,
        "content": "untagged memory",
        "producer": &owner_did,
        "created_at": "2025-12-01T00:00:00Z",
        "tags": ["restore-untagged"],
    });
    let signed_tagged = sign_artifact(&artifact_tagged, &schema::MEMORY_V1, &kp).expect("sign");
    let signed_untagged = sign_artifact(&artifact_untagged, &schema::MEMORY_V1, &kp).expect("sign");

    let tagged_ar_tx = "RESTORE_TAGGED_AR_TX";
    let untagged_ar_tx = "RESTORE_UNTAGGED_AR_TX";

    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/graphql");
        then.status(200).json_body(serde_json::json!({
            "data": { "transactions": {
                "pageInfo": { "hasNextPage": false },
                "edges": [
                    {
                        "cursor": "c1",
                        "node": {
                            "id": tagged_ar_tx,
                            "block": { "timestamp": 1_727_481_600 },
                            "tags": [
                                { "name": "Producer", "value": &owner_did },
                                { "name": "App-Name", "value": "mnemonic-protocol" },
                            ]
                        }
                    },
                    {
                        "cursor": "c2",
                        "node": { "id": untagged_ar_tx, "block": null, "tags": [] }
                    }
                ],
            }}
        }));
    });
    server.mock(|when, then| {
        when.method(GET).path(format!("/{tagged_ar_tx}"));
        then.status(200).body(signed_tagged.cose_bytes.clone());
    });
    server.mock(|when, then| {
        when.method(GET).path(format!("/{untagged_ar_tx}"));
        then.status(200).body(signed_untagged.cose_bytes.clone());
    });

    let gql = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
    let gateway = ArweaveClient::new(&server.base_url());

    let ChainSnapshot { items } = snapshot_chain(&gql, &gateway, std::slice::from_ref(&owner_did))
        .await
        .expect("snapshot_chain must succeed");

    assert_eq!(items.len(), 2, "got: {items:?}");
    let find = |ar_tx: &str| {
        items
            .iter()
            .find(|i| i.arweave_tx == ar_tx)
            .unwrap_or_else(|| panic!("item {ar_tx} not found; all: {items:?}"))
    };
    let tagged = find(tagged_ar_tx);
    let untagged = find(untagged_ar_tx);
    assert_eq!(tagged.producer.as_deref(), Some(owner_did.as_str()));
    assert_eq!(untagged.producer.as_deref(), Some(owner_did.as_str()));
    assert_eq!(
        tagged.content_hash.as_deref(),
        Some(signed_tagged.content_hash.as_str())
    );
    assert_eq!(
        untagged.content_hash.as_deref(),
        Some(signed_untagged.content_hash.as_str())
    );
    assert_eq!(untagged.day, None);
}
