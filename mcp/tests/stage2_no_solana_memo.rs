//! Stage 2 (chain-agnostic): WriteMode::Anchored no longer writes SPL Memos.
//!
//! Three acceptance tests as specified in tech-spec §Testing:
//!
//! 1. `anchored_write_makes_no_solana_rpc_call` — the Solana RPC URL points at
//!    a mock server that returns 500 on ANY request. An anchored stdio write
//!    still proceeds (Arweave upload goes to a happy mock; Solana is never
//!    contacted). The row is stored with solana_tx = '' and the error — if
//!    any — is a DeliveryNotConfirmed (Arweave refetch), NOT a Solana error.
//!
//! 2. `legacy_row_with_real_solana_tx_still_verifies` — a row seeded directly
//!    with a non-empty `solana_tx` (legacy shape) must still pass
//!    `mnemonic_verify` via the Arweave fetch + COSE verify path. The Solana
//!    comparison was removed from the delivery check, but `verify_anchored`
//!    still calls `read_memo` to derive the expected hash; the result must be
//!    `verified`.
//!
//! 3. `restore_covers_memo_and_no_memo_rows` — `snapshot_chain` (recovery)
//!    with one `MemoAnchor` (legacy: real `solana_tx`) and one GraphQL-only
//!    item (new: no memo) both appear in the final snapshot.

mod _helpers;

use httpmock::prelude::*;
use mnemonic_core::{
    arweave::{
        graphql::GraphQlClient,
        recovery::{snapshot_chain, ChainSnapshot},
        ArweaveClient,
    },
    codec::{schema, sign::sign_artifact},
    solana::MemoAnchor,
    storage::{AttestationStore, Visibility, WriteMode},
};
use mnemonic_mcp::{
    test_support::mock_state_with,
    tools::{resolve_write_mode, sign_memory, Transport},
};
use serde_json::json;
use solana_sdk::signature::{Keypair, Signer};
use std::{sync::Arc, time::Duration};

// ── 1. Anchored write makes no Solana RPC call ────────────────────────────────

/// Wires a mock Arweave (POST /tx → 200, GET /tx → 404) and a mock Solana
/// (any request → 500) and asserts that an anchored stdio write:
///   (a) makes the Arweave upload (POST /tx is called),
///   (b) makes NO Solana RPC calls (Solana 500 server gets 0 hits),
///   (c) stores the row with solana_tx = '' (empty string),
///   (d) the error — if any — is DeliveryNotConfirmed (Arweave refetch
///       timed out at 0ms budget), not a Solana error.
#[tokio::test]
async fn anchored_write_makes_no_solana_rpc_call() {
    // Arweave mock — POST /tx returns a tx id; GET returns 404 so the delivery
    // check exhausts its 0-ms budget and returns DeliveryNotConfirmed. That is
    // the ONLY error the write path is allowed to produce; any Solana error
    // would be a stage-2 regression.
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

    // Solana mock — returns HTTP 500 for every request. Stage 2: no Solana call
    // is expected. If the code accidentally calls Solana, the returned error
    // would be an "Err" from `submit_memo`, NOT a DeliveryNotConfirmed — and
    // the assertion below would catch it.
    let solana_server = MockServer::start();
    let solana_500_mock = solana_server.mock(|when, then| {
        when.method(POST).path("/");
        then.status(500)
            .header("content-type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"solana must not be called"}}"#);
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
    let solana = mnemonic_core::solana::SolanaClient::new(&solana_server.base_url());
    let keypair_raw = Keypair::new();
    let keypair = mnemonic_core::identity::LazyKeypair::ready(keypair_raw);
    let owner = keypair.pubkey_base58();
    let pending = Arc::new(mnemonic_mcp::pending::PendingBundles::with_defaults());
    let store_mutex = std::sync::Mutex::new(store);
    let cost_hint = mnemonic_mcp::pricing::CostHint {
        irys_lamports: 0,
        sol_tx_fee_lamports: 0,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    };
    let envelope = mnemonic_mcp::mcp::Envelope::from_config("full", "none", 0);

    let result = sign_memory(
        &keypair,
        &solana,
        &arweave,
        &store_mutex,
        embedder.as_ref(),
        &compressor,
        &pending,
        "stage2 anchored no memo content",
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

    // ── Assertion 2: Solana was NOT called.
    assert_eq!(
        solana_500_mock.calls(),
        0,
        "stage 2: anchored write must make ZERO Solana RPC calls; solana_500_mock had hits"
    );

    // ── Assertion 3: the persisted row has solana_tx = ''.
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
        "stage 2: new anchored rows must store solana_tx = '' (empty string); got {solana_tx:?}"
    );

    // ── Assertion 4: the error is DeliveryNotConfirmed (stage=refetch), NOT Solana.
    match &result {
        Ok(_) => {
            // Unlikely with a 0-ms budget and a 404 GET, but not impossible
            // on a very fast machine. Either way, solana_tx='' is already
            // asserted above.
        }
        Err(mnemonic_mcp::tools::ToolError::TypedRpc(rpc)) => {
            assert_eq!(
                rpc.code, -32011,
                "expected DeliveryNotConfirmed (-32011), got code {} message {:?}",
                rpc.code, rpc.message
            );
            assert!(
                !rpc.message.to_lowercase().contains("solana"),
                "error must not reference Solana; got: {}",
                rpc.message
            );
        }
        Err(mnemonic_mcp::tools::ToolError::Other(e)) => {
            panic!("sign_memory failed with non-typed error (expected DeliveryNotConfirmed): {e:#}");
        }
    }
}

// ── 2. Legacy rows with a real solana_tx still verify ────────────────────────

/// Seeds a row with a non-empty `solana_tx` (pre-stage-2 shape) and asserts
/// that `mnemonic_verify` still resolves it to `status: "verified"`. The
/// `verify_anchored` path still calls `read_memo` to derive the expected hash;
/// the Solana mock returns a synthetic memo, Arweave returns the matching COSE
/// bytes.
#[tokio::test]
async fn legacy_row_with_real_solana_tx_still_verifies() {
    // Build the COSE bytes for a synthetic artifact.
    let kp = Keypair::new();
    let owner_pubkey = kp.pubkey().to_string();
    let content = "legacy memory written before stage 2";
    let artifact = serde_json::json!({
        "artifact_id": "legacy-att-id",
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{owner_pubkey}"),
        "created_at": "2026-01-01T00:00:00Z",
        "tags": ["legacy"],
        "metadata": {
            "embed_provider": "stub",
            "embed_dim": 8,
            "turbo_bits": 4,
            "embedding_compressed": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &[0u8; 4],
            ),
        },
    });
    let signed = sign_artifact(&artifact, &schema::MEMORY_V1, &kp).expect("sign artifact");
    let content_hash = signed.content_hash.clone();
    let cose_bytes = signed.cose_bytes.clone();

    let legacy_sol_tx =
        "LEGACY_SOL_SIG_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let legacy_ar_tx = "LEGACY_AR_TX_AAAAAAAAAAAAAAAAAAAAAA";

    // Arweave mock: GET /<ar_tx> → COSE bytes.
    let arweave_server = MockServer::start();
    arweave_server.mock(|when, then| {
        when.method(GET).path(format!("/{legacy_ar_tx}"));
        then.status(200).body(cose_bytes.clone());
    });

    // Solana mock: getTransaction returns a log message containing the legacy
    // memo JSON {"h": content_hash, "a": ar_tx, "v": 2}.
    let memo_json = serde_json::json!({
        "h": content_hash,
        "a": legacy_ar_tx,
        "v": 2,
    })
    .to_string();
    let memo_len = memo_json.len();
    // The log line format is exactly what `read_memo` parses.
    let log_line = format!("Program log: Memo (len {memo_len}): \"{memo_json}\"");
    let solana_rpc_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "meta": {
                "err": null,
                "logMessages": [log_line]
            },
            "transaction": {},
            "slot": 1
        }
    })
    .to_string();
    let solana_server = MockServer::start();
    solana_server.mock(|when, then| {
        when.method(POST).path("/");
        then.status(200)
            .header("content-type", "application/json")
            .body(solana_rpc_body);
    });

    // Build a McpState with the mock Solana + Arweave clients.
    let state = mock_state_with("full", "none", 0);

    // Seed the legacy row into the store.
    {
        let store_g = state.store.lock().unwrap();
        store_g
            .save_attestation(
                "legacy-att-id",
                content,
                &content_hash,
                &["legacy".to_string()],
                legacy_sol_tx,
                legacy_ar_tx,
                &owner_pubkey,
                &owner_pubkey,
                "2026-01-01T00:00:00Z",
                WriteMode::Anchored,
                Visibility::Private,
                &vec![0.1f32; 8],
            )
            .expect("seed legacy row");
    }

    // Call verify with mock SolanaClient + ArweaveClient pointing at the mock servers.
    let solana_client =
        mnemonic_core::solana::SolanaClient::new(&solana_server.base_url());
    let arweave_client = ArweaveClient::new(&arweave_server.base_url());
    let embedder = Box::new(mnemonic_mcp::test_support::StubEmbedder::default());
    let compressor = mnemonic_core::compress::EmbeddingCompressor::new(8, 4, 42);

    let result = mnemonic_mcp::tools::verify(
        &solana_client,
        &arweave_client,
        &state.store,
        Some(legacy_sol_tx),
        None,
        &owner_pubkey,
        "full",
        embedder.as_ref(),
        &compressor,
    )
    .await
    .expect("verify must not return anyhow::Error");

    assert_eq!(
        result["status"].as_str(),
        Some("verified"),
        "legacy row with real solana_tx must still verify; got {result:?}"
    );
    assert_eq!(
        result["content_hash"].as_str(),
        Some(content_hash.as_str()),
        "content_hash must match the seeded value"
    );
}

// ── 3. Restore covers both memo and no-memo rows ─────────────────────────────

/// `snapshot_chain` with one `MemoAnchor` (legacy: real `solana_tx`) and one
/// GraphQL-only item (new row written after stage 2, no memo) both appear in
/// the final snapshot. This exercises the post-stage-2 production state where
/// some items predate and some postdate the memo removal.
#[tokio::test]
async fn restore_covers_memo_and_no_memo_rows() {
    let kp = Keypair::new();
    let owner_did = format!("did:sol:{}", kp.pubkey());

    let artifact_legacy = serde_json::json!({
        "artifact_id": "restore-legacy",
        "type": "memory",
        "schema_version": 1,
        "content": "legacy memory (has a solana memo)",
        "producer": &owner_did,
        "created_at": "2025-12-01T00:00:00Z",
        "tags": ["restore-legacy"],
    });
    let artifact_new = serde_json::json!({
        "artifact_id": "restore-new",
        "type": "memory",
        "schema_version": 1,
        "content": "new memory (no solana memo, stage 2)",
        "producer": &owner_did,
        "created_at": "2026-09-28T00:00:00Z",
        "tags": ["restore-new"],
    });

    let signed_legacy =
        sign_artifact(&artifact_legacy, &schema::MEMORY_V1, &kp).expect("sign legacy");
    let signed_new =
        sign_artifact(&artifact_new, &schema::MEMORY_V1, &kp).expect("sign new");

    let legacy_ar_tx = "RESTORE_LEGACY_AR_TX";
    let new_ar_tx = "RESTORE_NEW_AR_TX";
    let legacy_sol_tx = "RESTORE_LEGACY_SOL_TX";

    let server = MockServer::start();

    // GraphQL: returns only the new item (no memo → only discoverable via GraphQL).
    server.mock(|when, then| {
        when.method(POST).path("/graphql");
        then.status(200).json_body(serde_json::json!({
            "data": { "transactions": {
                "pageInfo": { "hasNextPage": false },
                "edges": [
                    {
                        "cursor": "c1",
                        "node": {
                            "id": new_ar_tx,
                            "block": { "timestamp": 1_727_481_600 },
                            "tags": [
                                { "name": "Producer", "value": &owner_did },
                                { "name": "App-Name", "value": "mnemonic-protocol" },
                            ]
                        }
                    }
                ],
            }}
        }));
    });

    // Arweave gateway: payload fetches for both items.
    server.mock(|when, then| {
        when.method(GET).path(format!("/{legacy_ar_tx}"));
        then.status(200).body(signed_legacy.cose_bytes.clone());
    });
    server.mock(|when, then| {
        when.method(GET).path(format!("/{new_ar_tx}"));
        then.status(200).body(signed_new.cose_bytes.clone());
    });

    // Legacy anchor (has a solana_tx).
    let memo_anchors = vec![MemoAnchor {
        solana_tx: legacy_sol_tx.to_string(),
        arweave_tx: legacy_ar_tx.to_string(),
        content_hash: signed_legacy.content_hash.clone(),
        block_time: Some(1_748_736_000), // 2025-06-01
    }];

    let gql = GraphQlClient::new(&format!("{}/graphql", server.base_url()));
    let gateway = ArweaveClient::new(&server.base_url());

    let ChainSnapshot { items } =
        snapshot_chain(&gql, &gateway, &[owner_did.clone()], &memo_anchors)
            .await
            .expect("snapshot_chain must succeed");

    assert_eq!(
        items.len(),
        2,
        "restore must recover both the memo-anchored (legacy) row and the GraphQL-only \
         (stage-2) row; got: {items:?}"
    );

    let find = |ar_tx: &str| {
        items
            .iter()
            .find(|i| i.arweave_tx == ar_tx)
            .unwrap_or_else(|| {
                panic!("item with arweave_tx={ar_tx} not found in snapshot; all: {items:?}")
            })
    };

    let legacy_item = find(legacy_ar_tx);
    let new_item = find(new_ar_tx);

    // Legacy item: carries the solana_tx and content_hash from the memo anchor.
    assert_eq!(
        legacy_item.solana_tx.as_deref(),
        Some(legacy_sol_tx),
        "legacy row must carry the original solana_tx from the memo anchor"
    );
    assert_eq!(
        legacy_item.content_hash.as_deref(),
        Some(signed_legacy.content_hash.as_str()),
        "legacy row must carry the content_hash from the memo"
    );

    // New item (stage 2, no memo): solana_tx is None.
    assert_eq!(
        new_item.solana_tx, None,
        "stage-2 row must not have a solana_tx in the snapshot (GraphQL-only item)"
    );
    assert!(
        new_item.producer.is_some(),
        "stage-2 row must have a producer tag from GraphQL; got: {new_item:?}"
    );
}
