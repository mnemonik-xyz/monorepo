//! Integration test: pending bundle expiry → 410 Gone (Task 8 #16 /
//! tech-spec line 227 / Decision 12 TTL=300s).
//!
//! `PendingBundles` uses `chrono::Utc::now()` (wall-clock) for expiry, so
//! `tokio::time::pause/advance` is a no-op here. We deterministically test
//! the expiry branch by building the store with TTL=1s and crossing the
//! boundary with a single 1100ms `tokio::time::sleep`. The full test stays
//! under 2s wall-clock.
//!
//! Logged as a Phase-1 deviation in `decisions.md` — Phase 2 will swap
//! `chrono::Utc::now()` for `tokio::time::Instant` so paused-time testing
//! works directly.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use governor::Quota;
use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::codec::{canonical::to_canonical_cbor, hash::hash_bytes, schema};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;
use mnemonic_mcp::{
    api::{get_pending_handler, sign_callback_handler},
    mcp::McpState,
    oauth::{self, OAuthState},
    pending::PendingBundles,
    test_support::StubEmbedder,
};
use solana_sdk::signature::{Keypair, Signer};
use std::num::NonZeroU32;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"pending-expiry-secret-32-bytes-!";

/// Build an `McpState` with a custom-TTL `PendingBundles`. Inlined rather
/// than reusing `test_support::mock_state()` because that helper bakes the
/// production 300s TTL.
fn state_with_short_ttl(ttl_secs: i64) -> Arc<McpState> {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let store = SqliteStore::open(tmp.path()).expect("sqlite");
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let chat_limiter =
        governor::RateLimiter::keyed(Quota::per_minute(NonZeroU32::new(10).unwrap()));
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
        keypair: Keypair::new(),
        solana: SolanaClient::new("http://localhost:0"),
        arweave: ArweaveClient::new("http://localhost:0"),
        store: std::sync::Mutex::new(store),
        embedder: Box::new(StubEmbedder::default()),
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
        pending: Arc::new(PendingBundles::new(10, ttl_secs, 5)),
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
    })
}

fn build_app(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/api/pending/{correlation_id}", get(get_pending_handler))
        .route("/api/sign-callback", post(sign_callback_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state)
}

async fn park_bundle(state: &Arc<McpState>, owner: &str) -> String {
    let metadata = serde_json::json!({"k": "v"});
    let artifact = serde_json::json!({
        "artifact_id": "expiry-test",
        "type": "memory",
        "schema_version": 1,
        "content": "expires-soon",
        "producer": format!("did:sol:{owner}"),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "tags": ["t"],
        "metadata": metadata.clone(),
    });
    let cbor = to_canonical_cbor(&artifact, &schema::MEMORY_V1).expect("cbor");
    let hash = hash_bytes(&cbor);
    state
        .pending
        .insert(
            owner.to_string(),
            "expires-soon".to_string(),
            vec![0.1; 8],
            hash,
            cbor,
            vec!["t".into()],
            metadata,
            mnemonic_core::storage::WriteMode::Participate,
        )
        .await
        .expect("park")
}

#[tokio::test]
async fn test_after_301s_pending_returns_410_and_evicts() {
    let state = state_with_short_ttl(1);
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_app(state.clone(), oauth_state.clone());

    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let token = oauth::issue_jwt(&oauth_state, &pubkey).expect("issue_jwt");

    let cid = park_bundle(&state, &pubkey).await;

    // Sanity: pre-expiry GET returns 200 (proves we're hitting the right
    // path and the TTL-1 hasn't already fired).
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "pre-expiry GET should be 200"
    );

    // Wall-clock sleep to cross the 1s TTL.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;

    // 1. GET /api/pending/<id> → 410 Gone.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/pending/{cid}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::GONE,
        "expired GET must be 410 Gone"
    );

    // 2. POST /api/sign-callback now sees NotFound (entry was lazily
    //    evicted in step 1) → 410 Gone via the handler's Decision 12 override.
    let req = Request::builder()
        .method("POST")
        .uri("/api/sign-callback")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "correlation_id": cid,
                "cose_signed_bytes": "AAAA",
                "signer_pubkey": pubkey,
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::GONE,
        "expired sign-callback must be 410 Gone"
    );
}
