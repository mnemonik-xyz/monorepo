//! Shared test helpers for `mcp/tests/*.rs` integration tests.
//!
//! Gated by `#[cfg(any(test, feature = "test-support"))]` so it ships with
//! `cargo test` AND the `mint-test-jwt` binary (which builds with
//! `--features test-support`) but never with the production binary.
//!
//! Two public helpers:
//!
//! - [`mock_state`] — wires a fully-formed [`McpState`] with an in-memory
//!   `tempfile::NamedTempFile` SQLite, a deterministic stub embedder, and
//!   `STORAGE_MODE=local` defaults. Each call gets its own DB so parallel
//!   `cargo test` invocations don't cross-contaminate.
//! - [`mint_jwt`] — issues a valid HS256 JWT against a caller-supplied secret
//!   for any `sub`. The claim shape (`iss`/`aud`/`sub`/`iat`/`exp`/`jti`)
//!   matches `oauth::Claims` exactly so a token minted here verifies via
//!   `oauth::verify_jwt(state, token)` without further setup.
//!
//! Why a separate module rather than inline copies in each test file?
//! Tests #1–#9 of Task 8 all build the same `McpState` shape; centralising
//! avoids drift if `McpState` gains a field (`pending`, future `lineage_dao`,
//! etc).

use std::sync::Arc;

use chrono::Utc;
use governor::Quota;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;
use std::num::NonZeroU32;
use uuid::Uuid;

use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::embed::Embedder;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;

use crate::llm::LlmClient;
use crate::mcp::{Envelope, McpState};
use crate::pending::PendingBundles;
use crate::pricing::PricingEngine;

/// Test-only deterministic embedder. 8 dims, all 0.1 — the same shape used by
/// the existing `StubEmbedder` copies in `tests/pending_authz.rs` and
/// `tests/sign_callback.rs`. Using a constant vector keeps `recall` stable
/// across tests (search returns rows in insertion order).
pub struct StubEmbedder {
    dim: usize,
}

impl Default for StubEmbedder {
    fn default() -> Self {
        Self { dim: 8 }
    }
}

impl StubEmbedder {
    /// Construct with a custom dimension. Production code paths assume `dim
    /// == compressor.dim()` so callers tweaking this must rebuild the
    /// compressor too.
    pub fn with_dim(dim: usize) -> Self {
        Self { dim }
    }
}

impl Embedder for StubEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        vec![0.1; self.dim]
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn provider_name(&self) -> &str {
        "stub"
    }
    fn model_id(&self) -> &str {
        "stub-zero"
    }
}

/// Build a fully-formed `McpState` for tests. Each call mints a fresh
/// `tempfile::NamedTempFile` SQLite database — parallel `cargo test`
/// invocations cannot collide.
///
/// Defaults:
/// - `keypair`: fresh `Keypair::new()` (server-side identity for stdio fallbacks)
/// - `embedder`: `StubEmbedder` 8-dim
/// - `payment_mode`: `"none"`
/// - `storage_mode`: `"local"`
/// - `pending`: production-default `PendingBundles` (10k/300s/50)
///
/// The Solana + Arweave clients point at unreachable `localhost:0` — any
/// network call panics; tests using `STORAGE_MODE=local` never make one.
pub fn mock_state() -> Arc<McpState> {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    // NOTE: leaking the path keeps the file alive for the duration of the
    // process. The OS reclaims it when the test binary exits. Holding the
    // `NamedTempFile` value would require returning it alongside `McpState`
    // and threading through every test — not worth it.
    let path = tmp.into_temp_path();
    let path_buf = path.keep().expect("keep tempfile");

    let store = SqliteStore::open(&path_buf).expect("sqlite open");
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let quota = Quota::per_minute(NonZeroU32::new(10).expect("nz"));
    let chat_limiter = governor::RateLimiter::keyed(quota);
    let publish_limiter = governor::RateLimiter::keyed(quota);
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest");
    let llm_client =
        LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512).expect("llm_client");

    let bootstrap_server_x25519_secret =
        crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
    let bootstrap_server_x25519_public = bootstrap_server_x25519_secret.public_key();

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
        pricing: PricingEngine::new(0),
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
        bootstrap_tickets: Arc::new(crate::api::BootstrapTickets::with_defaults()),
        bootstrap_server_x25519_secret,
        bootstrap_server_x25519_public,
        envelope: Envelope::from_config("local", "none", 0),
        delivery_refetch_timeout: std::time::Duration::from_secs(15),
        refunds_by_subject: Arc::new(crate::payment::RefundsBySubject::new(
            std::time::Duration::from_secs(60),
            5,
        )),
        delivery_metrics: Arc::new(crate::payment::DeliveryMetrics::default()),
        confirmation_ledger: Arc::new(crate::confirmation_token::ConfirmationLedger::new()),
        // Empty endpoint sentinel — `tools::sign_memory` treats an empty
        // string as "no soft-fall available" so tests that don't wire an
        // httpmock get the default no-network behaviour.
        hosted_endpoint: String::new(),
        hosted_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("reqwest hosted client"),
    })
}

/// Builder-shaped variant of [`mock_state`] used by `mcp/tests/_helpers/` to
/// vary `storage_mode` + `payment_mode` per test without copying the full
/// `McpState` literal. Defaults match `mock_state()`; the price quoted in the
/// envelope mirrors the production wiring (PricingEngine current_price()).
///
/// Returns the freshly-built `Arc<McpState>` plus an `Arc<oauth::OAuthState>`
/// with the standard test secret — the harness combines both into a single
/// `TestServer` value.
pub fn mock_state_with(
    storage_mode: &str,
    payment_mode: &str,
    sign_memory_cost_micro_usdc: i64,
) -> Arc<McpState> {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.into_temp_path();
    let path_buf = path.keep().expect("keep tempfile");

    let store = SqliteStore::open(&path_buf).expect("sqlite open");
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let quota = Quota::per_minute(NonZeroU32::new(10).expect("nz"));
    let chat_limiter = governor::RateLimiter::keyed(quota);
    let publish_limiter = governor::RateLimiter::keyed(quota);
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest");
    let llm_client =
        LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512).expect("llm_client");

    let bootstrap_server_x25519_secret =
        crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
    let bootstrap_server_x25519_public = bootstrap_server_x25519_secret.public_key();

    // Pricing engine seeded with the cost the caller wants quoted. `Envelope`
    // reads this same value via `current_price()` so the wire-format
    // `participate_cost.amount_cents` matches what the paywall actually
    // charges (single source of truth — drift impossible by construction).
    let pricing = PricingEngine::new(sign_memory_cost_micro_usdc);

    Arc::new(McpState {
        keypair: Keypair::new(),
        solana: SolanaClient::new("http://localhost:0"),
        arweave: ArweaveClient::new("http://localhost:0"),
        store: std::sync::Mutex::new(store),
        embedder: Box::new(StubEmbedder::default()),
        compressor,
        payment_mode: payment_mode.to_string(),
        treasury_pubkey: String::new(),
        usdc_mint: String::new(),
        admin_token: String::new(),
        evm_payment: None,
        pricing,
        sol_tx_fee_lamports: 0,
        storage_mode: storage_mode.to_string(),
        ollama_url: "http://localhost:0".into(),
        ollama_model: "test-model".into(),
        rag_chunk_dir: std::path::PathBuf::from("/tmp"),
        llm_client,
        artifact_zip_path: std::sync::Mutex::new(None),
        ollama_client,
        chat_limiter,
        publish_limiter,
        pending: Arc::new(PendingBundles::with_defaults()),
        bootstrap_tickets: Arc::new(crate::api::BootstrapTickets::with_defaults()),
        bootstrap_server_x25519_secret,
        bootstrap_server_x25519_public,
        envelope: Envelope::from_config(storage_mode, payment_mode, sign_memory_cost_micro_usdc),
        delivery_refetch_timeout: std::time::Duration::from_secs(15),
        refunds_by_subject: Arc::new(crate::payment::RefundsBySubject::new(
            std::time::Duration::from_secs(60),
            5,
        )),
        delivery_metrics: Arc::new(crate::payment::DeliveryMetrics::default()),
        confirmation_ledger: Arc::new(crate::confirmation_token::ConfirmationLedger::new()),
        hosted_endpoint: String::new(),
        hosted_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("reqwest hosted client"),
    })
}

/// T3-specific variant of [`mock_state_with`] for the
/// `mcp/tests/delivery_guarantee.rs` integration suite. Adds two knobs that
/// the other paths don't need to vary:
///
/// - `arweave_url` — points the underlying `ArweaveClient` at a caller-
///   supplied `httpmock` base URL (the production code paths always use
///   `localhost:0` which won't accept POSTs).
/// - `quota_threshold` / `quota_window` — tunes the DoS guard so a 6-call
///   test scenario can induce the short-circuit without waiting 60s of
///   wall-clock time.
/// - `refetch_timeout` — lets tests use a sub-second budget so the
///   refetch-failure path returns within an acceptable test duration
///   instead of the production 15s default.
#[allow(clippy::too_many_arguments)]
pub fn mock_state_for_delivery(
    storage_mode: &str,
    payment_mode: &str,
    sign_memory_cost_micro_usdc: i64,
    arweave_url: &str,
    solana_rpc_url: &str,
    quota_threshold: u32,
    quota_window: std::time::Duration,
    refetch_timeout: std::time::Duration,
    treasury_pubkey: &str,
    usdc_mint: &str,
) -> Arc<McpState> {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.into_temp_path();
    let path_buf = path.keep().expect("keep tempfile");

    let store = SqliteStore::open(&path_buf).expect("sqlite open");
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let quota = Quota::per_minute(NonZeroU32::new(10).expect("nz"));
    let chat_limiter = governor::RateLimiter::keyed(quota);
    let publish_limiter = governor::RateLimiter::keyed(quota);
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest");
    let llm_client =
        LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512).expect("llm_client");

    let bootstrap_server_x25519_secret =
        crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
    let bootstrap_server_x25519_public = bootstrap_server_x25519_secret.public_key();

    let pricing = PricingEngine::new(sign_memory_cost_micro_usdc);

    Arc::new(McpState {
        keypair: Keypair::new(),
        solana: SolanaClient::new(solana_rpc_url),
        arweave: ArweaveClient::new(arweave_url),
        store: std::sync::Mutex::new(store),
        embedder: Box::new(StubEmbedder::default()),
        compressor,
        payment_mode: payment_mode.to_string(),
        treasury_pubkey: treasury_pubkey.to_string(),
        usdc_mint: usdc_mint.to_string(),
        admin_token: String::new(),
        evm_payment: None,
        pricing,
        sol_tx_fee_lamports: 0,
        storage_mode: storage_mode.to_string(),
        ollama_url: "http://localhost:0".into(),
        ollama_model: "test-model".into(),
        rag_chunk_dir: std::path::PathBuf::from("/tmp"),
        llm_client,
        artifact_zip_path: std::sync::Mutex::new(None),
        ollama_client,
        chat_limiter,
        publish_limiter,
        pending: Arc::new(PendingBundles::with_defaults()),
        bootstrap_tickets: Arc::new(crate::api::BootstrapTickets::with_defaults()),
        bootstrap_server_x25519_secret,
        bootstrap_server_x25519_public,
        envelope: Envelope::from_config(storage_mode, payment_mode, sign_memory_cost_micro_usdc),
        delivery_refetch_timeout: refetch_timeout,
        refunds_by_subject: Arc::new(crate::payment::RefundsBySubject::new(
            quota_window,
            quota_threshold,
        )),
        delivery_metrics: Arc::new(crate::payment::DeliveryMetrics::default()),
        confirmation_ledger: Arc::new(crate::confirmation_token::ConfirmationLedger::new()),
        hosted_endpoint: String::new(),
        hosted_client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("reqwest hosted client"),
    })
}

/// Build an `McpState` for the agent-native-distribution Task 5 soft-fall
/// tests. Two extra knobs over [`mock_state`]:
///
/// - `embedder: Box<dyn Embedder>` — lets the test inject `FailingEmbedder`
///   so `sign_memory_inline` returns `-32098 EmbedderInvalid` deterministically
///   without needing a real ONNX model on disk. The production `mock_state`
///   uses [`StubEmbedder`]; this variant is the constructor the task spec
///   describes as "test-only McpState constructor variant taking
///   `Box<dyn Embedder>` directly".
/// - `hosted_endpoint: String` — points the soft-fall proxy at an
///   `httpmock` instance OR at an unreachable URL (to exercise the
///   `-32011 HostedUnavailable` branch).
///
/// `dim` must match `embedder.dim()`; the compressor is rebuilt against it
/// so the canonical CBOR pipeline doesn't reject a length-mismatched
/// vector. All other defaults mirror [`mock_state`] verbatim.
pub fn mock_state_with_embedder_and_endpoint(
    embedder: Box<dyn Embedder>,
    hosted_endpoint: String,
) -> Arc<McpState> {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.into_temp_path();
    let path_buf = path.keep().expect("keep tempfile");

    let dim = embedder.dim();
    let store = SqliteStore::open(&path_buf).expect("sqlite open");
    let compressor = EmbeddingCompressor::new(dim, 4, 42);
    let quota = Quota::per_minute(NonZeroU32::new(10).expect("nz"));
    let chat_limiter = governor::RateLimiter::keyed(quota);
    let publish_limiter = governor::RateLimiter::keyed(quota);
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest");
    let llm_client =
        LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512).expect("llm_client");

    let bootstrap_server_x25519_secret =
        crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
    let bootstrap_server_x25519_public = bootstrap_server_x25519_secret.public_key();

    let hosted_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Short timeout — the no-network test asserts on `HostedUnavailable`
        // within a single test step rather than a 30s default.
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("reqwest hosted client");

    Arc::new(McpState {
        keypair: Keypair::new(),
        solana: SolanaClient::new("http://localhost:0"),
        arweave: ArweaveClient::new("http://localhost:0"),
        store: std::sync::Mutex::new(store),
        embedder,
        compressor,
        payment_mode: "none".into(),
        treasury_pubkey: String::new(),
        usdc_mint: String::new(),
        admin_token: String::new(),
        evm_payment: None,
        pricing: PricingEngine::new(0),
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
        bootstrap_tickets: Arc::new(crate::api::BootstrapTickets::with_defaults()),
        bootstrap_server_x25519_secret,
        bootstrap_server_x25519_public,
        envelope: Envelope::from_config("local", "none", 0),
        delivery_refetch_timeout: std::time::Duration::from_secs(2),
        refunds_by_subject: Arc::new(crate::payment::RefundsBySubject::new(
            std::time::Duration::from_secs(60),
            5,
        )),
        delivery_metrics: Arc::new(crate::payment::DeliveryMetrics::default()),
        confirmation_ledger: Arc::new(crate::confirmation_token::ConfirmationLedger::new()),
        hosted_endpoint,
        hosted_client,
    })
}

/// Local copy of the JWT claim shape — must mirror `oauth::Claims` exactly.
/// Inlined to avoid a circular `pub use` (the helpers feature is also
/// reachable from the `mint-test-jwt` binary which doesn't link the
/// `oauth` module's serde-derived public type back through `lib.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalClaims {
    iss: String,
    aud: String,
    sub: String,
    iat: u64,
    exp: u64,
    jti: String,
}

/// Mint a valid HS256 JWT for `sub` using `secret`. Same claim shape as
/// production (`iss="mcp.mnemonik.xyz"`, `aud="mcp"`, `iat=now`,
/// `exp=now+3600`, `jti=Uuid::new_v4()`).
///
/// The `secret` must be `>= 32` bytes; shorter inputs panic at the
/// `EncodingKey::from_secret` boundary in production via
/// `OAuthState::new` / `OAuthState::with_defaults` asserts. We do not enforce here because tests
/// occasionally use shorter secrets to exercise rejection paths.
pub fn mint_jwt(sub: &str, secret: &[u8]) -> String {
    let now = Utc::now().timestamp() as u64;
    let claims = LocalClaims {
        iss: "mcp.mnemonik.xyz".to_string(),
        aud: "mcp".to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + 3600,
        jti: Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("JWT encode")
}
