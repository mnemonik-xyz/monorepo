//! Remote artifacts survive complete removal of MCP routing receipts.
mod _helpers;
use _helpers::TestServer;
use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mnemonic_core::codec::a2a::signed::{prepare_signed_a2a, verify_signed_a2a};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_sdk::signature::{Keypair, Signer};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
struct Remote {
    blobs: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    unavailable: Arc<std::sync::atomic::AtomicBool>,
    uploads: Arc<std::sync::atomic::AtomicUsize>,
    payment_calls: Arc<std::sync::atomic::AtomicUsize>,
}
async fn upload(State(s): State<Remote>, bytes: Bytes) -> Json<Value> {
    s.uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(&bytes[..2], &2u16.to_le_bytes());
    let id = URL_SAFE_NO_PAD.encode(Sha256::digest(&bytes[2..66]));
    let tags = u64::from_le_bytes(bytes[108..116].try_into().unwrap()) as usize;
    let original = bytes[116 + tags..].to_vec();
    verify_signed_a2a(&original, None).unwrap();
    s.blobs.lock().unwrap().insert(id.clone(), original);
    Json(json!({"id":id}))
}
async fn read(
    State(s): State<Remote>,
    Path(id): Path<String>,
) -> Result<Vec<u8>, axum::http::StatusCode> {
    if s.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
    s.blobs
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}
async fn unexpected_payment(State(s): State<Remote>) -> axum::http::StatusCode {
    s.payment_calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    axum::http::StatusCode::INTERNAL_SERVER_ERROR
}

async fn put_object(
    State(s): State<Remote>,
    Path(id): Path<String>,
    bytes: Bytes,
) -> axum::http::StatusCode {
    if id != hex::encode(Sha256::digest(&bytes)) {
        return axum::http::StatusCode::BAD_REQUEST;
    }
    let mut blobs = s.blobs.lock().unwrap();
    if blobs.contains_key(&id) {
        return axum::http::StatusCode::PRECONDITION_FAILED;
    }
    blobs.insert(id, bytes.to_vec());
    axum::http::StatusCode::CREATED
}

async fn query(State(s): State<Remote>, Json(req): Json<Value>) -> Json<Value> {
    let filter = req["variables"]["tags"].as_array().unwrap();
    let mut edges = Vec::new();
    for (id, bytes) in s.blobs.lock().unwrap().iter() {
        let v = verify_signed_a2a(bytes, None).unwrap();
        let tags = json!([{"name":"App-Name","value":"mnemonic-protocol"},{"name":"Mnemonic-Type","value":"a2a"},{"name":"Producer","value":v.signer},{"name":"Context-Id","value":v.binding.context_id},{"name":"Content-Hash","value":v.content_hash}]);
        if filter.iter().all(|f| {
            tags.as_array().unwrap().iter().any(|t| {
                t["name"] == f["name"] && f["values"].as_array().unwrap().contains(&t["value"])
            })
        }) {
            edges.push(json!({"cursor":id,"node":{"id":id,"tags":tags}}));
        }
    }
    Json(json!({"data":{"transactions":{"edges":edges,"pageInfo":{"hasNextPage":false}}}}))
}
async fn remote() -> (String, Remote, tokio::task::JoinHandle<()>) {
    let s = Remote::default();
    let app = Router::new()
        .route("/upload", post(upload))
        .route("/paywall/{*path}", post(unexpected_payment))
        .route("/graphql", post(query))
        .route("/objects/{id}", get(read).put(put_object))
        .route("/{id}", get(read))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, s, task)
}
fn result(r: &_helpers::CallResult) -> Value {
    assert!(r.envelope["error"].is_null(), "{:?}", r.envelope);
    serde_json::from_str(r.envelope["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap()
}
fn no_memories(server: &TestServer) {
    let g = server.state.store.lock().unwrap();
    for table in ["attestations", "attestation_embeddings", "grants"] {
        let n: i64 = g
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}
#[tokio::test]
async fn original_bytes_remote_metadata_only_and_retry() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder().arweave_gateway(url).build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let reader = v["reader"]["pubkey_base58"].as_str().unwrap();
    for key in ["plain", "sealed", "stream"] {
        let args = json!({"kind":"message","context_id":"fixture-ctx","sealed":key!="plain","signed":v[key]});
        let a = result(
            &server
                .call_tool(Some(author), "mnemonic_attest_a2a", args.clone())
                .await,
        );
        let b = result(
            &server
                .call_tool(Some(author), "mnemonic_attest_a2a", args)
                .await,
        );
        assert_eq!(a, b);
        assert_eq!(
            hex::encode(&s.blobs.lock().unwrap()[a["arweave_tx"].as_str().unwrap()]),
            v[key]
        );
    }
    no_memories(&server);
    assert_eq!(s.blobs.lock().unwrap().len(), 3);
    let rows = result(
        &server
            .call_tool(
                Some(reader),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx","sealed":true}),
            )
            .await,
    );
    assert_eq!(rows["receipts"].as_array().unwrap().len(), 2);
    task.abort();
}
#[tokio::test]
async fn parent_validation_survives_receipt_loss() {
    let (url, _, task) = remote().await;
    let server = TestServer::builder().arweave_gateway(url).build();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    let payload = json!({"messageId":"root","role":"agent","contextId":"ctx","parts":[{"kind":"text","text":"root"}]});
    let root = prepare_signed_a2a(
        &kp,
        "message",
        payload.clone(),
        "ctx",
        None,
        "2026-10-01T00:00:00Z",
        None,
    )
    .unwrap();
    let receipt = result(
        &server
            .call_tool(
                Some(&owner),
                "mnemonic_attest_a2a",
                json!({"kind":"message","context_id":"ctx","signed":hex::encode(root)}),
            )
            .await,
    );
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("DELETE FROM a2a_anchor_readers; DELETE FROM a2a_anchor_receipts;")
        .unwrap();
    let id = receipt["attestation_id"].as_str().unwrap();
    let child = prepare_signed_a2a(
        &kp,
        "message",
        payload.clone(),
        "ctx",
        Some(id.to_string()),
        "2026-10-01T00:00:01Z",
        None,
    )
    .unwrap();
    let mut args =
        json!({"kind":"message","context_id":"ctx","prev_id":id,"signed":hex::encode(child)});
    let missing = server
        .call_tool(Some(&owner), "mnemonic_attest_a2a", args.clone())
        .await;
    assert!(missing.envelope["error"].is_object());
    args["prev_locator"] = receipt["locator"].clone();
    result(
        &server
            .call_tool(Some(&owner), "mnemonic_attest_a2a", args.clone())
            .await,
    );
    args["prev_locator"] = json!("ar://AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    assert!(server
        .call_tool(Some(&owner), "mnemonic_attest_a2a", args)
        .await
        .envelope["error"]
        .is_object());
    no_memories(&server);
    task.abort();
}
#[tokio::test]
async fn invalid_client_and_local_mode_never_upload() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder().arweave_gateway(url).build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    for args in [
        json!({"kind":"message","context_id":"other","signed":v["sealed"],"sealed":true}),
        json!({"kind":"message","context_id":"fixture-ctx","signed":v["plain"],"mode":"local"}),
    ] {
        assert!(server
            .call_tool(Some(author), "mnemonic_attest_a2a", args)
            .await
            .envelope["error"]
            .is_object());
    }
    assert!(s.blobs.lock().unwrap().is_empty());
    task.abort();
}
#[tokio::test]
#[ignore = "requires built SDK and real WASM"]
async fn sdk_http_sealed_end_to_end() {
    let (url, _, remote_task) = remote().await;
    let server = TestServer::builder().arweave_gateway(url.clone()).build();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = server.app.clone();
    let mcp = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let v = vectors();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/sdk/scripts/test-sealed-a2a-e2e.mjs");
    let mut command = tokio::process::Command::new("node");
    command
        .arg(&script)
        .env("A2A_TEST_URL", &base)
        .env("A2A_GATEWAY_URL", &url)
        .env(
            "A2A_AUTHOR_JWT",
            server.mint_jwt(v["author"]["pubkey_base58"].as_str().unwrap()),
        );
    let write = command.output().await.unwrap();
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let expected = String::from_utf8(write.stdout).unwrap();
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("DELETE FROM a2a_anchor_readers; DELETE FROM a2a_anchor_receipts;")
        .unwrap();
    mcp.abort();
    let restored = tokio::process::Command::new("node")
        .arg(&script)
        .arg("restore")
        .env("A2A_TEST_URL", &base)
        .env("A2A_GATEWAY_URL", &url)
        .env("A2A_EXPECTED", expected.trim())
        .output()
        .await
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let restarted_url = format!("http://{}", listener.local_addr().unwrap());
    let router = server.app.clone();
    let restarted = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let appended = tokio::process::Command::new("node")
        .arg(&script)
        .arg("append")
        .env("A2A_TEST_URL", restarted_url)
        .env("A2A_GATEWAY_URL", &url)
        .env("A2A_EXPECTED", expected.trim())
        .env(
            "A2A_AUTHOR_JWT",
            server.mint_jwt(v["author"]["pubkey_base58"].as_str().unwrap()),
        )
        .output()
        .await
        .unwrap();
    assert!(
        appended.status.success(),
        "{}",
        String::from_utf8_lossy(&appended.stderr)
    );
    no_memories(&server);
    restarted.abort();
    remote_task.abort();
}

#[tokio::test]
async fn paid_invalid_local_never_reaches_paywall() {
    let server = TestServer::builder().payment_mode("x402").build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let r = server
        .call_tool(
            Some(author),
            "mnemonic_attest_a2a",
            json!({"kind":"message","context_id":"fixture-ctx","signed":v["plain"],"mode":"local"}),
        )
        .await;
    assert_eq!(r.status, axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn failed_delivery_is_pending_and_retry_uses_existing_remote_bytes() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder().arweave_gateway(url).build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let args =
        json!({"kind":"message","context_id":"fixture-ctx","sealed":true,"signed":v["stream"]});
    s.unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(server
        .call_tool(Some(author), "mnemonic_attest_a2a", args.clone())
        .await
        .envelope["error"]
        .is_object());
    let rows = result(
        &server
            .call_tool(
                Some(author),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx"}),
            )
            .await,
    );
    assert!(rows["receipts"].as_array().unwrap().is_empty());
    assert_eq!(s.blobs.lock().unwrap().len(), 1);
    s.unavailable
        .store(false, std::sync::atomic::Ordering::SeqCst);
    result(
        &server
            .call_tool(Some(author), "mnemonic_attest_a2a", args)
            .await,
    );
    assert_eq!(s.blobs.lock().unwrap().len(), 1);
    no_memories(&server);
    task.abort();
}

/// A genuinely fresh operator reads the migrated parent from another backend.
/// The original storage listener is stopped and its receipt rows are gone.
#[tokio::test]
async fn independent_operator_continues_from_migrated_parent_without_source() {
    let (source_url, source, source_task) = remote().await;
    let original = TestServer::builder().arweave_gateway(source_url).build();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    let payload = json!({"messageId":"parent","role":"agent","contextId":"migration","parts":[{"kind":"text","text":"parent"}]});
    let parent = prepare_signed_a2a(
        &kp,
        "message",
        payload.clone(),
        "migration",
        None,
        "2026-10-02T00:00:00Z",
        None,
    )
    .unwrap();
    let receipt = result(
        &original
            .call_tool(
                Some(&owner),
                "mnemonic_attest_a2a",
                json!({"kind":"message","context_id":"migration","signed":hex::encode(&parent)}),
            )
            .await,
    );
    let (destination_url, destination, destination_task) = remote().await;
    let digest = hex::encode(Sha256::digest(&parent));
    // Exact copy into the other backend; identity/signature remain unchanged.
    destination
        .blobs
        .lock()
        .unwrap()
        .insert(digest.clone(), parent.clone());
    original
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("DELETE FROM a2a_anchor_readers; DELETE FROM a2a_anchor_receipts;")
        .unwrap();
    source.blobs.lock().unwrap().clear();
    source_task.abort();
    let replacement = TestServer::builder()
        .arweave_gateway(destination_url.clone())
        .parent_blob_origin(destination_url.clone())
        .build();
    assert_ne!(
        original.state.keypair.pubkey_base58(),
        replacement.state.keypair.pubkey_base58()
    );
    let id = receipt["attestation_id"].as_str().unwrap();
    let child = prepare_signed_a2a(
        &kp,
        "message",
        payload.clone(),
        "migration",
        Some(id.into()),
        "2026-10-02T00:00:01Z",
        None,
    )
    .unwrap();
    let args = json!({"kind":"message","context_id":"migration","signed":hex::encode(child),"prev_id":id,"prev_locator":format!("blob://{digest}")});
    let continued = result(
        &replacement
            .call_tool(Some(&owner), "mnemonic_attest_a2a", args.clone())
            .await,
    );
    assert_eq!(continued["delivery_status"], "verified");
    no_memories(&replacement);
    // Disabling the opt-in origin rejects that same hint rather than fetching arbitrary URLs.
    let disabled = TestServer::builder()
        .arweave_gateway(destination_url)
        .build();
    assert!(disabled
        .call_tool(Some(&owner), "mnemonic_attest_a2a", args.clone())
        .await
        .envelope["error"]
        .is_object());
    // A blob at the right path with wrong bytes cannot authorize a continuation.
    destination.blobs.lock().unwrap().get_mut(&digest).unwrap()[0] ^= 1;
    assert!(replacement
        .call_tool(Some(&owner), "mnemonic_attest_a2a", args)
        .await
        .envelope["error"]
        .is_object());
    destination_task.abort();
}

#[tokio::test]
async fn unsupported_paid_a2a_rail_fails_closed_without_upload_or_settlement() {
    let (url, remote, task) = remote().await;
    let server = TestServer::builder()
        .arweave_gateway(url)
        .payment_mode("x402")
        .build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let r = server
        .call_tool(
            Some(author),
            "mnemonic_attest_a2a",
            json!({"kind":"message","context_id":"fixture-ctx","signed":v["plain"]}),
        )
        .await;
    assert_eq!(r.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.envelope["error"]["data"]["error"]
        .as_str()
        .unwrap()
        .contains("no payment accepted"));
    assert!(remote.blobs.lock().unwrap().is_empty());
    no_memories(&server);
    task.abort();
}

/// One real SDK/WASM drill crosses both storage and operator boundaries.
/// It does not exercise a live provider or imply provider retention guarantees.
#[tokio::test]
#[ignore = "requires built SDK and real WASM"]
async fn sdk_migrated_sealed_stream_recipient_continues_through_independent_operator() {
    // Invalidate previous success before any network, SDK, or assertion can fail.
    if let Ok(directory) = std::env::var("MNEMONIC_DEMO_EVIDENCE_DIR") {
        let path = std::path::Path::new(&directory).join("report.json");
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!(
                "cannot invalidate previous demo report {}: {error}",
                path.display()
            ),
        }
    }
    let (source_url, source, source_task) = remote().await;
    let (destination_url, destination, destination_task) = remote().await;
    let original = TestServer::builder()
        .arweave_gateway(source_url.clone())
        .build();
    let original_operator_key = original.state.keypair.pubkey_base58();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let original_url = format!("http://{}", listener.local_addr().unwrap());
    let app = original.app.clone();
    let original_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let fixtures = vectors();
    let author = fixtures["author"]["pubkey_base58"].as_str().unwrap();
    let reader = fixtures["reader"]["pubkey_base58"].as_str().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/sdk/scripts/test-migrated-a2a-e2e.mjs");
    let backup_passphrase = Keypair::new().pubkey().to_string();
    let created = tokio::process::Command::new("node")
        .arg(&script)
        .env("A2A_OPERATOR_URL", &original_url)
        .env("A2A_SOURCE_URL", &source_url)
        .env("A2A_DESTINATION_URL", &destination_url)
        .env("A2A_JWT", original.mint_jwt(author))
        .env("A2A_READER_JWT", original.mint_jwt(reader))
        .env("A2A_BACKUP_PASSPHRASE", &backup_passphrase)
        .output()
        .await
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let packet: Value = serde_json::from_slice(&created.stdout).unwrap();
    let source_bytes = source.blobs.lock().unwrap().clone();
    assert_eq!(source_bytes.len(), 3);
    {
        let migrated = destination.blobs.lock().unwrap();
        assert_eq!(migrated.len(), 3);
        for bytes in source_bytes.values() {
            let digest = hex::encode(Sha256::digest(bytes));
            assert_eq!(migrated.get(&digest).unwrap(), bytes);
            let verified = verify_signed_a2a(bytes, None).unwrap();
            assert_eq!(
                packet["authors"][format!("a2a:{}", verified.content_hash)],
                verified.signer
            );
            assert_eq!(
                packet["originals"][format!("a2a:{}", verified.content_hash)],
                hex::encode(bytes)
            );
        }
    }
    no_memories(&original);
    original.state.store.lock().unwrap().conn().execute_batch(
        "DELETE FROM a2a_anchor_readers; DELETE FROM a2a_anchor_receipts; DELETE FROM artifact_receipts;"
    ).unwrap();
    // Destroy the original operator/router/store and remote source, not just routing hints.
    original_task.abort();
    let _ = original_task.await;
    drop(original);
    source.blobs.lock().unwrap().clear();
    source_task.abort();
    let _ = source_task.await;
    assert!(reqwest::get(format!("{source_url}/graphql")).await.is_err());
    assert!(reqwest::get(format!("{original_url}/health"))
        .await
        .is_err());
    let replacement = TestServer::builder()
        .arweave_gateway(destination_url.clone())
        .parent_blob_origin(destination_url.clone())
        .build();
    assert_ne!(
        original_operator_key,
        replacement.state.keypair.pubkey_base58()
    );
    no_memories(&replacement);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let replacement_url = format!("http://{}", listener.local_addr().unwrap());
    let app = replacement.app.clone();
    let replacement_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let recovered = tokio::process::Command::new("node")
        .arg(&script)
        .arg("restore")
        .env("A2A_OPERATOR_URL", replacement_url)
        .env("A2A_ORIGINAL_OPERATOR", original_url)
        .env("A2A_SOURCE_URL", source_url)
        .env("A2A_DESTINATION_URL", destination_url)
        .env("A2A_JWT", replacement.mint_jwt(reader))
        .env("A2A_RECOVERY_PACKET", packet.to_string())
        .env("A2A_BACKUP_PASSPHRASE", &backup_passphrase)
        .output()
        .await
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let continued: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(continued["restored"], 3);
    assert_eq!(continued["completeToHeads"], true);
    let child_bytes = hex::decode(continued["original"].as_str().unwrap()).unwrap();
    let child = verify_signed_a2a(&child_bytes, Some(reader)).unwrap();
    assert_eq!(
        child.binding.prev_id.as_deref(),
        continued["parent"].as_str()
    );
    let child_locator = continued["locator"]
        .as_str()
        .unwrap()
        .strip_prefix("ar://")
        .unwrap();
    assert_eq!(
        destination.blobs.lock().unwrap()[child_locator],
        child_bytes
    );
    no_memories(&replacement);
    let receipt_count: i64 = replacement
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT COUNT(*) FROM a2a_anchor_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        receipt_count, 1,
        "O2 has only the new child receipt, no restored SQL history"
    );
    // Export only observed checks and public routing/identity metadata, never the recovery packet.
    let mut evidence = continued["evidence"].clone();
    evidence["operators"] =
        json!({"O1":original_operator_key,"O2":replacement.state.keypair.pubkey_base58()});
    evidence["checks"]["originalOperatorOffline"] = json!(true);
    evidence["checks"]["originalSourceOffline"] = json!(true);
    evidence["checks"]["operatorPayloadTablesEmpty"] = json!(true);
    evidence["checks"]["replacementReceiptCount"] = json!(receipt_count);
    evidence["failures"].as_array_mut().unwrap().extend([
        json!({"name":"O1_shutdown_and_receipt_loss","status":"recovered","detail":"Original router stopped and database dropped; fresh B restored R/S/V and delivered W through O2."}),
        json!({"name":"original_storage_shutdown","status":"recovered","detail":"Original storage erased and listener stopped; exact migrated bytes restored from configured destination."}),
    ]);
    assert_eq!(evidence["artifacts"].as_array().unwrap().len(), 4);
    if let Ok(directory) = std::env::var("MNEMONIC_DEMO_EVIDENCE_DIR") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("report.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        eprintln!("Research handoff evidence: {}", path.display());
    }
    replacement_task.abort();
    destination_task.abort();
}

#[tokio::test]
#[ignore = "requires built SDK and real WASM"]
async fn two_operator_uploads_have_distinct_locators_and_sdk_verified_dedup() {
    let (url, remote, task) = remote().await;
    let first = TestServer::builder().arweave_gateway(url.clone()).build();
    let second = TestServer::builder().arweave_gateway(url.clone()).build();
    assert_ne!(
        first.state.keypair.pubkey_base58(),
        second.state.keypair.pubkey_base58()
    );
    let v = vectors();
    let owner = v["author"]["pubkey_base58"].as_str().unwrap();
    let args = json!({"kind":"message", "context_id":v["context_id"], "signed":v["plain"]});
    let a = result(
        &first
            .call_tool(Some(owner), "mnemonic_attest_a2a", args.clone())
            .await,
    );
    let b = result(
        &second
            .call_tool(Some(owner), "mnemonic_attest_a2a", args)
            .await,
    );
    assert_eq!(a["attestation_id"], b["attestation_id"]);
    assert_ne!(a["locator"], b["locator"]);
    assert_eq!(remote.uploads.load(std::sync::atomic::Ordering::SeqCst), 2);
    for receipt in [&a, &b] {
        let id = receipt["arweave_tx"].as_str().unwrap();
        assert_eq!(hex::encode(&remote.blobs.lock().unwrap()[id]), v["plain"]);
    }
    no_memories(&first);
    no_memories(&second);
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/sdk/scripts/test-migrated-a2a-e2e.mjs");
    let output = tokio::process::Command::new("node")
        .arg(script)
        .arg("dedup")
        .env("A2A_SOURCE_URL", url)
        .env(
            "A2A_DEDUP_PACKET",
            json!({"id":a["attestation_id"]}).to_string(),
        )
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["verifiedCandidates"], 2);
    assert_eq!(report["artifacts"], 1);
    task.abort();
}

#[tokio::test]
async fn invalid_external_parents_fail_before_upload_or_payment() {
    let (url, remote, task) = remote().await;
    let config = mnemonic_mcp::universal_paywall::UniversalPaywallConfig {
        url: format!("{url}/paywall"),
        api_key: "test-only".into(),
        network: "eip155:84532".into(),
        asset: "0x0000000000000000000000000000000000000001".into(),
        pay_to: "0x0000000000000000000000000000000000000002".into(),
        payer_wallet: String::new(),
        approval_url_base: String::new(),
    };
    let server = TestServer::builder()
        .arweave_gateway(url)
        .payment_mode("x402")
        .universal_paywall(config)
        .build();
    let author = Keypair::new();
    let outsider = Keypair::new();
    let make = |key: &Keypair, context: &str, label: &str, prev: Option<String>| {
        let payload = json!({"messageId":label,"role":"agent","contextId":context,"parts":[{"kind":"text","text":label}]});
        prepare_signed_a2a(
            key,
            "message",
            payload,
            context,
            prev,
            "2026-10-02T00:00:00Z",
            None,
        )
        .unwrap()
    };
    let parent = make(&author, "parent-ctx", "parent", None);
    let wrong = make(&author, "parent-ctx", "different-parent", None);
    let parent_id = format!(
        "a2a:{}",
        verify_signed_a2a(&parent, None).unwrap().content_hash
    );
    let good_locator = "A".repeat(43);
    let wrong_locator = "B".repeat(43);
    let corrupt_locator = "C".repeat(43);
    let mut corrupt = parent.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    {
        let mut blobs = remote.blobs.lock().unwrap();
        blobs.insert(good_locator.clone(), parent);
        blobs.insert(wrong_locator.clone(), wrong);
        blobs.insert(corrupt_locator.clone(), corrupt);
    }
    let cases = [
        (
            "wrong hash",
            &author,
            "parent-ctx",
            Some(wrong_locator),
            "parent hash mismatch",
        ),
        (
            "wrong context",
            &author,
            "another-ctx",
            Some(good_locator.clone()),
            "parent context mismatch",
        ),
        (
            "ineligible writer",
            &outsider,
            "parent-ctx",
            Some(good_locator),
            "parent link not eligible",
        ),
        (
            "corrupt signature",
            &author,
            "parent-ctx",
            Some(corrupt_locator),
            "signature",
        ),
        (
            "unavailable",
            &author,
            "parent-ctx",
            Some("D".repeat(43)),
            "ParentUnavailable",
        ),
        (
            "missing locator",
            &author,
            "parent-ctx",
            None,
            "ParentLocatorRequired",
        ),
        (
            "malformed locator",
            &author,
            "parent-ctx",
            Some("invalid-id".into()),
            "invalid parent locator",
        ),
    ];
    for (label, signer, context, locator, expected_message) in cases {
        let child = make(signer, context, label, Some(parent_id.clone()));
        let mut args = json!({"kind":"message","context_id":context,"signed":hex::encode(child),"prev_id":parent_id});
        if let Some(locator) = locator {
            args["prev_locator"] = json!(format!("ar://{locator}"));
        }
        let response = server
            .call_tool(
                Some(&signer.pubkey().to_string()),
                "mnemonic_attest_a2a",
                args,
            )
            .await;
        let (status, code) = if label == "unavailable" {
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, -32011)
        } else {
            (axum::http::StatusCode::BAD_REQUEST, -32602)
        };
        assert_eq!(response.status, status, "{label}: {:?}", response.envelope);
        assert_eq!(response.envelope["error"]["code"], code);
        let message = response.envelope["error"]["message"].as_str().unwrap();
        assert!(message.contains(expected_message), "{label}: {message}");
        if label != "unavailable" {
            assert_ne!(message, "ParentUnavailable");
        }
        assert_eq!(
            remote.uploads.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{label}"
        );
        assert_eq!(
            remote
                .payment_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{label}"
        );
        let store = server.state.store.lock().unwrap();
        for table in [
            "paid_operations",
            "delivery_operations",
            "a2a_anchor_receipts",
        ] {
            let count: i64 = store
                .conn()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{label}: {table}");
        }
    }
    no_memories(&server);
    task.abort();
}
