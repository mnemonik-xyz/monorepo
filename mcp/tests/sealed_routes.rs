//! Integration tests for Task 10 — sealed-memory routes and tools.
//!
//! TDD anchors:
//! - `anchor-sealed` rejects COSE whose producer is another identity.
//! - `store-sealed` never writes an embedding row.
//! - `/api/embed` handler does not log the body (log-capture test).
//! - `mnemonic_share` returns `awaiting_signature` with `approve_url`.
//! - `mnemonic_verify` returns `{sealed:true, readable:false}` for sealed rows.

mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{delete, get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::{
    codec::{sign::sign_cose, canonical::to_canonical_cbor},
    storage::WriteMode,
};
use mnemonic_mcp::{
    mcp::{mcp_handler, McpState},
    oauth::{self, OAuthState},
    sealed_routes::{
        anchor_sealed_handler, delete_grant_handler, embed_handler, list_grants_handler,
        list_sealed_handler, store_sealed_handler, create_grant_handler,
    },
    test_support::{mint_jwt, mock_state},
};
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_JWT_SECRET: &[u8; 32] = b"sealed-routes-test-secret-32-byt";

// ── helpers ────────────────────────────────────────────────────────────────

fn build_router(state: Arc<McpState>) -> Router {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_JWT_SECRET));
    Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/api/anchor-sealed", post(anchor_sealed_handler))
        .route("/api/store-sealed", post(store_sealed_handler))
        .route("/api/sealed", get(list_sealed_handler))
        .route(
            "/api/grants",
            post(create_grant_handler).get(list_grants_handler),
        )
        .route("/api/grants/{id}", delete(delete_grant_handler))
        .route("/api/embed", post(embed_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

/// Issue a raw bytes POST to a sealed route with a Bearer JWT.
async fn post_bytes(
    app: Router,
    uri: &str,
    jwt: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value =
        serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

/// Issue a JSON POST to /mcp (tools/call) with a Bearer JWT.
async fn call_tool(app: Router, jwt: &str, name: &str, args: Value) -> (StatusCode, Value) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": args},
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value =
        serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

/// Build a minimal canonical-CBOR payload and sign it with `keypair`.
/// Returns the raw COSE_Sign1 bytes.
fn make_signed_cose(keypair: &Keypair, producer_did: &str) -> Vec<u8> {
    let payload = serde_json::json!({
        "type": "sealed_memory",
        "schema_version": 1,
        "producer": producer_did,
        "created_at": "2026-09-28T00:00:00Z",
    });
    let cbor = to_canonical_cbor(
        &payload,
        &mnemonic_core::codec::schema::SEALED_V1,
    )
    .unwrap_or_else(|_| serde_json::to_vec(&payload).unwrap());
    sign_cose(&cbor, keypair).expect("sign_cose failed")
}

// ────────────────────────────────────────────────────────────────────────────
// Test 1: anchor-sealed rejects COSE signed by another identity
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn anchor_sealed_rejects_wrong_producer() {
    let state = mock_state();
    let app = build_router(state.clone());

    // Alice is authenticated.
    let alice_kp = Keypair::new();
    let alice_pubkey = alice_kp.pubkey().to_string();
    let alice_jwt = mint_jwt(&alice_pubkey, TEST_JWT_SECRET);

    // Bob signs the COSE (different keypair, different DID).
    let bob_kp = Keypair::new();
    let bob_pubkey = bob_kp.pubkey().to_string();
    let bob_did = format!("did:sol:{bob_pubkey}");
    let cose_bytes = make_signed_cose(&bob_kp, &bob_did);

    // Alice submits Bob's signed COSE → server must reject (403).
    let (status, body) = post_bytes(
        app,
        "/api/anchor-sealed",
        &alice_jwt,
        "application/cbor",
        cose_bytes,
    )
    .await;

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "expected 403/401 for cross-owner COSE, got {status}: {body}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 2: store-sealed never writes an embedding row
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn retired_store_sealed_never_writes_embedding_row() {
    let state = mock_state();
    let app = build_router(state.clone());

    let owner_kp = Keypair::new();
    let owner_pubkey = owner_kp.pubkey().to_string();
    let jwt = mint_jwt(&owner_pubkey, TEST_JWT_SECRET);

    // POST some opaque bytes as a sealed CBOR blob.
    let fake_cbor = b"fake sealed cbor content for testing".to_vec();
    let (status, body) = post_bytes(
        app,
        "/api/store-sealed",
        &jwt,
        "application/cbor",
        fake_cbor,
    )
    .await;

    assert_eq!(status, StatusCode::GONE, "hosted local store must fail: {body}");

    // Verify no embedding row was written.
    let store = state.store.lock().unwrap();
    let embedding_count: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM attestation_embeddings",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        embedding_count, 0,
        "store-sealed must not write an embedding row"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 3: /api/embed does not log the body
//
// We verify this structurally: the handler is a thin function that embeds
// and returns. We assert the text is NOT present in any tracing event by
// using a tracing subscriber that captures all log events.
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn embed_handler_does_not_log_body() {
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    use tracing_subscriber::layer::SubscriberExt;

    // Capture log lines.
    let log_lines: StdArc<StdMutex<Vec<String>>> = StdArc::new(StdMutex::new(Vec::new()));
    let log_lines_clone = log_lines.clone();

    // Build a tracing subscriber that stores formatted events in the vec.
    struct CaptureLayer {
        lines: StdArc<StdMutex<Vec<String>>>,
    }
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = StringVisitor(String::new());
            event.record(&mut visitor);
            self.lines.lock().unwrap().push(visitor.0);
        }
    }
    struct StringVisitor(String);
    impl tracing::field::Visit for StringVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push_str(&format!("[{}={}]", field.name(), value));
        }
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("[{}={:?}]", field.name(), value));
        }
    }

    let capture_layer = CaptureLayer {
        lines: log_lines_clone,
    };
    let subscriber = tracing_subscriber::registry().with(capture_layer);

    // Run the embed call under the capturing subscriber.
    let state = mock_state();
    let app = build_router(state);
    let owner_kp = Keypair::new();
    let jwt = mint_jwt(&owner_kp.pubkey().to_string(), TEST_JWT_SECRET);

    let secret_text = "this-is-the-secret-embedding-input-CANARY";

    let (status, body) = tracing::subscriber::with_default(subscriber, || async {
        post_bytes(
            app,
            "/api/embed",
            &jwt,
            "text/plain",
            secret_text.as_bytes().to_vec(),
        )
        .await
    })
    .await;

    assert_eq!(status, StatusCode::OK, "embed should return 200: {body}");
    assert!(
        body.get("embedding").is_some(),
        "response must include embedding: {body}"
    );

    // The secret canary must NOT appear in any captured log line.
    let all_logs = log_lines.lock().unwrap().join("\n");
    assert!(
        !all_logs.contains(secret_text),
        "embed handler must not log the input body; found canary in logs:\n{all_logs}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 4: mnemonic_share returns awaiting_signature with approve_url
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn mnemonic_share_returns_awaiting_signature() {
    let state = mock_state();
    let app = build_router(state);

    let owner_kp = Keypair::new();
    let owner_pubkey = owner_kp.pubkey().to_string();
    let jwt = mint_jwt(&owner_pubkey, TEST_JWT_SECRET);

    let args = serde_json::json!({
        "memory_hash": "a".repeat(64),
        "reader": "did:sol:ReaderPubkey123",
    });
    let (status, envelope) = call_tool(app, &jwt, "mnemonic_share", args).await;

    assert_eq!(status, StatusCode::OK, "mnemonic_share should return 200: {envelope}");

    // The result is wrapped in MCP content envelope.
    let result_text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let result: Value = serde_json::from_str(result_text).unwrap_or_default();

    assert_eq!(
        result["status"].as_str().unwrap_or(""),
        "awaiting_signature",
        "mnemonic_share must return status=awaiting_signature: {result}"
    );
    assert!(
        result["approve_url"].as_str().is_some(),
        "mnemonic_share must include approve_url: {result}"
    );
    assert!(
        result["correlation_id"].as_str().is_some(),
        "mnemonic_share must include correlation_id: {result}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 5: mnemonic_verify returns sealed:true for sealed row
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn mnemonic_verify_returns_sealed_for_sealed_row() {
    let state = mock_state();
    let app = build_router(state.clone());

    let owner_kp = Keypair::new();
    let owner_pubkey = owner_kp.pubkey().to_string();
    let jwt = mint_jwt(&owner_pubkey, TEST_JWT_SECRET);

    // Insert a sealed row directly so we have known tx ids.
    let attestation_id = "test-sealed-verify-id";
    let content_hash = "b".repeat(64);
    let solana_tx = "local:solana-sealed";
    let arweave_tx = "local:arweave-sealed";
    let now = "2026-09-28T00:00:00Z";
    {
        let store = state.store.lock().unwrap();
        store
            .save_sealed_attestation(
                attestation_id,
                &content_hash,
                &[],
                solana_tx,
                arweave_tx,
                &owner_pubkey,
                &owner_pubkey,
                now,
                WriteMode::Local,
                b"fake-sealed-blob",
            )
            .expect("save_sealed_attestation");
    }

    // Call mnemonic_verify with the solana_tx.
    let args = serde_json::json!({
        "solana_tx": solana_tx,
    });
    let (status, envelope) = call_tool(app, &jwt, "mnemonic_verify", args).await;

    assert_eq!(status, StatusCode::OK, "verify should return 200: {envelope}");

    let result_text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let result: Value = serde_json::from_str(result_text).unwrap_or_default();

    assert_eq!(
        result["sealed"].as_bool().unwrap_or(false),
        true,
        "mnemonic_verify must return sealed=true for sealed rows: {result}"
    );
    assert_eq!(
        result["readable"].as_bool().unwrap_or(true),
        false,
        "mnemonic_verify must return readable=false for sealed rows: {result}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 6: recall includes sealed_hidden count and hint when sealed rows exist
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn recall_includes_sealed_hidden_count() {
    let state = mock_state();
    let app = build_router(state.clone());

    let owner_kp = Keypair::new();
    let owner_pubkey = owner_kp.pubkey().to_string();
    let jwt = mint_jwt(&owner_pubkey, TEST_JWT_SECRET);

    // Insert a sealed row for this owner.
    {
        let store = state.store.lock().unwrap();
        store
            .save_sealed_attestation(
                "sealed-recall-test",
                &"c".repeat(64),
                &[],
                "local:sol-sealed-recall",
                "local:ar-sealed-recall",
                &owner_pubkey,
                &owner_pubkey,
                "2026-09-28T00:00:00Z",
                WriteMode::Local,
                b"opaque",
            )
            .expect("save sealed");
    }

    let args = serde_json::json!({"query": "test query"});
    let (status, envelope) = call_tool(app, &jwt, "mnemonic_recall", args).await;
    assert_eq!(status, StatusCode::OK);

    let result_text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let result: Value = serde_json::from_str(result_text).unwrap_or_default();

    let sealed_hidden = result["sealed_hidden"].as_i64().unwrap_or(0);
    assert!(
        sealed_hidden >= 1,
        "recall must report sealed_hidden >= 1 when sealed rows exist: {result}"
    );
    assert!(
        result["sealed_hint"].as_str().is_some(),
        "recall must include sealed_hint: {result}"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test 7: /api/grants round-trip (POST + GET + DELETE)
// ────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn retired_grant_write_preserves_existing_read_and_withdrawal() {
    let state = mock_state();
    let app = build_router(state.clone());

    let author_kp = Keypair::new();
    let author_pubkey = author_kp.pubkey().to_string();
    let jwt = mint_jwt(&author_pubkey, TEST_JWT_SECRET);

    // Build a minimal GRANT_V1 COSE payload signed by author.
    let grant_payload = serde_json::json!({
        "type": "grant",
        "schema_version": 1,
        "memory_hash": "d".repeat(64),
        "reader_kid": "did:sol:ReaderABC",
        "author": format!("did:sol:{author_pubkey}"),
    });
    let cbor = to_canonical_cbor(
        &grant_payload,
        &mnemonic_core::codec::schema::GRANT_V1,
    )
    .unwrap_or_else(|_| serde_json::to_vec(&grant_payload).unwrap());
    let cose_bytes = sign_cose(&cbor, &author_kp).expect("sign grant");
    let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cose_bytes);

    // POST /api/grants
    let body = serde_json::json!({
        "grant_cose_b64": cose_b64,
        "memory_hash": "d".repeat(64),
        "reader_kid": "did:sol:ReaderABC",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/grants")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let created: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::GONE, "POST /api/grants must be retired: {created}");
    let grant_id = "legacy-grant".to_string();
    // Existing receipts remain readable/withdrawable; migration never erases them.
    state.store.lock().unwrap().save_grant(&grant_id, &"d".repeat(64), Some("did:sol:ReaderABC"), &cose_bytes, &author_pubkey, "2026-10-02T00:00:00Z").unwrap();

    // GET /api/grants?reader=did:sol:ReaderABC
    let req = Request::builder()
        .method("GET")
        .uri("/api/grants?reader=did:sol:ReaderABC")
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    let grants = list["grants"].as_array().expect("grants array");
    assert!(!grants.is_empty(), "should have at least 1 grant");

    // DELETE /api/grants/{id}
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/grants/{grant_id}"))
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET again — should be empty (withdrawn grants excluded).
    let req = Request::builder()
        .method("GET")
        .uri("/api/grants?reader=did:sol:ReaderABC")
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let list2: Value = serde_json::from_slice(&bytes).unwrap();
    let grants2 = list2["grants"].as_array().expect("grants array");
    assert!(grants2.is_empty(), "withdrawn grant must not appear in GET");
}
