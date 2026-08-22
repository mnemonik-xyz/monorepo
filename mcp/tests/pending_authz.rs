//! Integration tests for `GET /api/pending/{correlation_id}` authz —
//! Decision 12. The endpoint is capability-based: the `correlation_id` IS
//! the capability, so any caller in possession of the UUIDv4 may read the
//! unsigned canonical-CBOR bytes (the actual signing chain is gated by
//! COSE + keypair ownership in `/api/sign-callback`, not by JWT here).
//!
//! Covers:
//!
//!   - `test_pending_get_404_for_unknown_correlation_id` — random id
//!     never seen → 404 Not Found.
//!   - `test_pending_get_returns_canonical_cbor_for_owner` — alice parks
//!     and reads back via her cid → 200 OK with `Content-Type: application/cbor`
//!     and the byte-for-byte canonical CBOR.
//!   - `test_pending_get_capability_any_jwt_succeeds` — capability access:
//!     bob's JWT on alice's cid still succeeds, because the cid IS the auth.
//!   - `test_pending_get_capability_no_jwt_succeeds` — capability access:
//!     no JWT at all still succeeds for the same reason; the path is
//!     allowlisted in `bearer_auth_middleware`.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::codec::{canonical::to_canonical_cbor, hash::hash_bytes, schema};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::embed::Embedder;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;
use mnemonic_mcp::{
    api::{get_pending_handler, sign_callback_handler},
    mcp::McpState,
    oauth::{self, OAuthState},
    pending::PendingBundles,
};
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"pending-authz-secret-32-bytes-!!";

struct StubEmbedder;
impl Embedder for StubEmbedder {
    fn embed(&self, _t: &str) -> Vec<f32> {
        vec![0.1; 8]
    }
    fn dim(&self) -> usize {
        8
    }
    fn provider_name(&self) -> &str {
        "stub"
    }
    fn model_id(&self) -> &str {
        "stub"
    }
}

fn build_state() -> Arc<McpState> {
    use governor::Quota;
    use std::num::NonZeroU32;
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let store = SqliteStore::open(tmp.path()).unwrap();
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let quota = Quota::per_minute(NonZeroU32::new(10).unwrap());
    let chat_limiter = governor::RateLimiter::keyed(quota);
    let publish_limiter = governor::RateLimiter::keyed(quota);
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let llm_client =
        mnemonic_mcp::llm::LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512)
            .unwrap();
    let bootstrap_x25519_sk = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
    let bootstrap_x25519_pk = bootstrap_x25519_sk.public_key();
    Arc::new(McpState {
        universal_paywall: None,
        universal_paywall_eip712_name: "USD Coin".to_string(),
        universal_paywall_eip712_version: "2".to_string(),
        universal_paywall_quotes: Default::default(),
        approval_ui_dist: None,
        approval_mock_signer: None,
        approval_chain_rpc_url: String::new(),
        approval_chain_name: String::new(),
        approval_chain_currency_symbol: "ETH".to_string(),
        approval_chain_currency_decimals: 18,
        keypair: Keypair::new(),
        solana: SolanaClient::new("http://localhost:0"),
        arweave: ArweaveClient::new("http://localhost:0"),
        store: std::sync::Mutex::new(store),
        embedder: Box::new(StubEmbedder),
        compressor,
        payment_mode: "none".into(),
        treasury_pubkey: String::new(),
        usdc_mint: String::new(),
        admin_token: String::new(),
        evm_payment: None,
        pricing: mnemonic_mcp::pricing::PricingEngine::new(0),
        sol_tx_fee_lamports: 0,
        storage_mode: "local".into(),
        ollama_url: "http://localhost:0".into(),
        ollama_model: "test-model".into(),
        rag_chunk_dir: std::path::PathBuf::from("/tmp"),
        llm_client,
        artifact_zip_path: std::sync::Mutex::new(None),
        ollama_client,
        chat_limiter,
        publish_limiter,
        pending: Arc::new(PendingBundles::with_defaults()),
        bootstrap_tickets: Arc::new(mnemonic_mcp::api::BootstrapTickets::with_defaults()),
        bootstrap_server_x25519_secret: bootstrap_x25519_sk,
        bootstrap_server_x25519_public: bootstrap_x25519_pk,
        // T2: discoverability envelope (local-only deploy in this test).
        envelope: mnemonic_mcp::mcp::Envelope::from_config("local", "none", 0),
        delivery_refetch_timeout: std::time::Duration::from_secs(15),
        refunds_by_subject: Arc::new(mnemonic_mcp::payment::RefundsBySubject::new(
            std::time::Duration::from_secs(60),
            5,
        )),
        delivery_metrics: Arc::new(mnemonic_mcp::payment::DeliveryMetrics::default()),
        confirmation_ledger: Arc::new(mnemonic_mcp::confirmation_token::ConfirmationLedger::new()),
        hosted_endpoint: String::new(),
        hosted_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("reqwest hosted client"),
        blog_rebuild_hook: None,
        chain_stats: None,
    })
}

fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/api/pending/{correlation_id}", get(get_pending_handler))
        .route("/api/sign-callback", post(sign_callback_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

async fn park_for(state: &Arc<McpState>, owner: &str, content: &str) -> (String, Vec<u8>) {
    let cid_hint = uuid::Uuid::new_v4().to_string();
    let metadata = serde_json::json!({"k": "v"});
    let artifact = serde_json::json!({
        "artifact_id": cid_hint,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{owner}"),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "tags": ["t"],
        "metadata": metadata.clone(),
    });
    let cbor = to_canonical_cbor(&artifact, &schema::MEMORY_V1).unwrap();
    let hash = hash_bytes(&cbor);
    let id = state
        .pending
        .insert(
            owner.to_string(),
            content.to_string(),
            vec![0.1; 8],
            hash,
            cbor.clone(),
            vec!["t".into()],
            metadata,
            mnemonic_core::storage::WriteMode::Participate,
        )
        .await
        .unwrap();
    (id, cbor)
}

#[tokio::test]
async fn test_pending_get_capability_any_jwt_succeeds() {
    // Decision 12 — `/api/pending/*` is capability-based. Any caller in
    // possession of a valid `correlation_id` can fetch the unsigned
    // canonical-CBOR bytes regardless of which JWT (if any) they hold;
    // forging the actual sign-callback still requires the user's
    // localStorage Ed25519 keypair, which the cid does not grant.
    let state = build_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state.clone(), oauth_state.clone());

    let alice = Keypair::new().pubkey().to_string();
    let bob = Keypair::new().pubkey().to_string();

    let (cid, _cbor) = park_for(&state, &alice, "alice's secret").await;

    // Bob's JWT on alice's cid still succeeds — capability is the cid itself.
    let bob_token = oauth::issue_jwt(&oauth_state, &bob).unwrap();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {bob_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Sanity: alice's own JWT also still works (peek does not consume).
    let alice_token = oauth::issue_jwt(&oauth_state, &alice).unwrap();
    let req2 = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {alice_token}"))
        .body(Body::empty())
        .unwrap();
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_pending_get_404_for_unknown_correlation_id() {
    let state = build_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state.clone(), oauth_state.clone());

    let user = Keypair::new().pubkey().to_string();
    let token = oauth::issue_jwt(&oauth_state, &user).unwrap();
    let bogus_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{bogus_id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_pending_get_returns_canonical_cbor_for_owner() {
    let state = build_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state.clone(), oauth_state.clone());

    let alice = Keypair::new().pubkey().to_string();
    let (cid, expected_cbor) = park_for(&state, &alice, "alice memo").await;

    let token = oauth::issue_jwt(&oauth_state, &alice).unwrap();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(ct, "application/cbor");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), expected_cbor.as_slice());
}

#[tokio::test]
async fn test_pending_get_capability_no_jwt_succeeds() {
    // Decision 12 — `/api/pending/*` is allowlisted in
    // `bearer_auth_middleware` and the handler does not require a JWT;
    // possession of the cid is the capability. A request without any
    // Authorization header against a known cid must succeed.
    let state = build_state();
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(state.clone(), oauth_state.clone());
    let alice = Keypair::new().pubkey().to_string();
    let (cid, _) = park_for(&state, &alice, "x").await;

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        // No Authorization header
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
