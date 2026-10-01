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
}
async fn upload(State(s): State<Remote>, bytes: Bytes) -> Json<Value> {
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
        .route("/graphql", post(query))
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
