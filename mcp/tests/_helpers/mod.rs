// Some helpers are only used by a subset of the integration tests in this
// crate (e.g. `balance_for` is consumed by T3's delivery_guarantee.rs
// landing later). The dead-code lints would fire because Cargo compiles
// `_helpers/mod.rs` once per test binary; silence them at the module
// level so we don't have to sprinkle `#[allow(dead_code)]` on every
// reusable accessor.
#![allow(dead_code)]

//! Integration-test harness for `mcp/tests/modes_per_request.rs`.
//!
//! Factors the existing `build_test_state` + `build_test_router` pattern
//! from `mcp/src/mcp.rs` `#[cfg(test)] mod transport_tests` into a
//! reusable shape so the new mode-aware integration tests don't copy 80
//! lines of `McpState` literal each. Behaviour matches the in-module
//! pattern byte-for-byte — same SQLite tempfile, same JWT secret, same
//! axum router shape with `oauth::bearer_auth_middleware` mounted.
//!
//! Builder shape:
//!
//! ```ignore
//! let server = TestServer::builder()
//!     .storage_mode("full")
//!     .payment_mode("x402")
//!     .sign_memory_cost_micro_usdc(10_000) // $0.01 / write
//!     .build();
//! let resp = server.call_tool("mnemonic_sign_memory", &args).await;
//! let count = server.attestation_count();
//! ```
//!
//! The harness mounts `/mcp` (MCP JSON-RPC), `/api/pending/{id}`,
//! `/api/sign-callback` so future tests for the deferred-signing
//! interaction with `mode` have everything wired without rebuilding the
//! router from scratch. JWT minting reuses `oauth::issue_jwt`.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::identity::pubkey_base58;
use mnemonic_core::storage::AttestationStore;
use mnemonic_mcp::{
    api::{get_pending_handler, sign_callback_handler},
    mcp::{mcp_handler, McpState},
    oauth::{self, refresh, OAuthState},
    test_support::{mint_jwt, mock_state_with},
};
use serde_json::Value;
use tower::ServiceExt;

/// 32-byte JWT secret shared across all `TestServer` instances. The
/// production middleware requires `>= 32 bytes`; this matches the existing
/// `TEST_JWT_SECRET` constants spread across `mcp/tests/*.rs`.
pub const TEST_JWT_SECRET: &[u8; 32] = b"modes-per-request-secret-32-byts";

/// 32-byte refresh-token salt used by the OAuth refresh-token rotation
/// subsystem (Task 1 — `refresh::hash_refresh_token(salt, plaintext)`).
/// Matches the test-only fixed salt used by the in-crate `refresh::tests`
/// module and by `OAuthState::with_defaults` so a row hashed inside a unit
/// test would have the same `token_hash` if recomputed against an instance
/// produced by this builder. NOT for production wiring — production
/// generates a fresh salt via `openssl rand -base64 32` at deploy time.
pub const TEST_REFRESH_SALT: [u8; 32] = [0xABu8; 32];

/// Builder for `TestServer`. All fields are optional with sensible defaults
/// (`storage_mode=local`, `payment_mode=none`, no per-write cost,
/// `oauth_token=false`). Mirror the names in `Config::from_env` so tests
/// read like deployment declarations.
///
/// The OAuth `/token` and `/authorize` routes are NOT mounted unless
/// `with_oauth_token(true)` is called — existing callers that never
/// touched the OAuth surface keep getting 404 on those paths.
pub struct TestServerBuilder {
    storage_mode: String,
    payment_mode: String,
    sign_memory_cost_micro_usdc: i64,
    oauth_token: bool,
    reuse_interval: Option<Duration>,
    evictor_tick: Option<Duration>,
}

impl Default for TestServerBuilder {
    fn default() -> Self {
        Self {
            storage_mode: "local".into(),
            payment_mode: "none".into(),
            sign_memory_cost_micro_usdc: 0,
            oauth_token: false,
            reuse_interval: None,
            evictor_tick: None,
        }
    }
}

impl TestServerBuilder {
    /// `"local"` (default) or `"full"`. Mirrors the env-var of the same name.
    pub fn storage_mode(mut self, s: &str) -> Self {
        self.storage_mode = s.to_string();
        self
    }

    /// `"none"` (default), `"balance"`, `"x402"`, or `"both"`. Drives the
    /// paywall gate AND the envelope's `payment_methods` list.
    pub fn payment_mode(mut self, s: &str) -> Self {
        self.payment_mode = s.to_string();
        self
    }

    /// Per-write cost in micro-USDC (1 USDC = 1_000_000). Default is 0.
    /// Surfaces in the whoami envelope as `participate_cost.amount_cents`
    /// (divided by 10_000) — set this to 10_000 to get `1` cent and verify
    /// the integer math.
    pub fn sign_memory_cost_micro_usdc(mut self, v: i64) -> Self {
        self.sign_memory_cost_micro_usdc = v;
        self
    }

    /// Enable mounting of the OAuth `/oauth/token` (POST) and `/oauth/authorize`
    /// (GET + POST) routes on the same tower-stack as `/mcp`. Default is
    /// `false` so existing tests that never exercise these routes get a 404
    /// for any stray probe (anonymous-recall regression guard). When `true`
    /// the harness opens its own tempfile-backed `rusqlite::Connection` for
    /// the OAuth state per Decision 6 and migrates `refresh_tokens` before
    /// returning.
    pub fn with_oauth_token(mut self, on: bool) -> Self {
        self.oauth_token = on;
        self
    }

    /// Override `OAuthState.reuse_interval` (Decision 13 — test-overridable
    /// to keep integration tests sub-second on wall-clock without relying on
    /// the production 5 s default). When `None` (default), the
    /// `OAuthState::with_defaults` fast 100 ms reuse-interval is used on the
    /// `oauth_token == false` path; the same 100 ms is the explicit fallback
    /// when `oauth_token == true` but no override is set, so behaviour is
    /// identical across both paths for tests that don't care.
    pub fn with_reuse_interval(mut self, d: Duration) -> Self {
        self.reuse_interval = Some(d);
        self
    }

    /// Override `OAuthState.evictor_tick` (Decision 13). Semantics mirror
    /// [`Self::with_reuse_interval`]: `None` falls back to the
    /// `with_defaults` 50 ms tick (or the 50 ms explicit fallback on the
    /// `oauth_token == true` path).
    pub fn with_evictor_tick(mut self, d: Duration) -> Self {
        self.evictor_tick = Some(d);
        self
    }

    /// Materialise into a `TestServer`.
    pub fn build(self) -> TestServer {
        let state = mock_state_with(
            &self.storage_mode,
            &self.payment_mode,
            self.sign_memory_cost_micro_usdc,
        );

        // Construct OAuthState. When the OAuth routes are mounted, we open
        // our own tempfile-backed connection so `refresh::hash_refresh_token`
        // + direct SQL helpers (`insert_expired_refresh_for_test`,
        // `family_has_unrevoked_rows`) operate on the same physical DB the
        // production /oauth/token handler reads through OAuthState.
        // Decision 6: a second physical Connection on a tempfile mirrors the
        // production pattern; refresh-token rows live in their own schema
        // independent of `McpState.store` (no cross-table consistency
        // required for Task 5 — verified by tech-spec §I.8).
        let oauth_state = if self.oauth_token {
            let tmp = tempfile::NamedTempFile::new().expect("oauth tempfile");
            let path = tmp.into_temp_path();
            // `keep()` prevents the `TempPath` Drop from deleting the file —
            // the SQLite handle inside `OAuthState` must outlive this builder
            // method. Same leak pattern `mock_state_with` uses for its store.
            let oauth_db = path.keep().expect("keep oauth tempfile");
            let salt = TEST_REFRESH_SALT.to_vec();
            // Default fast timers — matches `OAuthState::with_defaults`
            // semantics so the `oauth_token == true` path is not slower
            // than the unmounted path for tests that don't override.
            let reuse_interval = self.reuse_interval.unwrap_or(Duration::from_millis(100));
            let evictor_tick = self.evictor_tick.unwrap_or(Duration::from_millis(50));
            let cap = refresh::DEFAULT_REUSE_CACHE_CAP;
            Arc::new(
                OAuthState::new(
                    TEST_JWT_SECRET,
                    &oauth_db,
                    salt,
                    reuse_interval,
                    evictor_tick,
                    cap,
                )
                .expect("OAuthState::new for TestServerBuilder"),
            )
        } else {
            Arc::new(OAuthState::with_defaults(TEST_JWT_SECRET))
        };

        let base = Router::new()
            .route("/mcp", post(mcp_handler))
            .route("/api/pending/{correlation_id}", get(get_pending_handler))
            .route("/api/sign-callback", post(sign_callback_handler))
            .layer(middleware::from_fn_with_state(
                oauth_state.clone(),
                oauth::bearer_auth_middleware,
            ))
            .with_state(state.clone());

        let app = if self.oauth_token {
            // Mount BOTH /oauth/token AND /oauth/authorize (GET + POST) on
            // the same tower-stack so `bootstrap_oauth` can drive the real
            // production challenge-sign-redeem path via `app.clone().oneshot`.
            // `with_state(oauth_state.clone())` is applied to the merged
            // sub-router because these handlers need `Arc<OAuthState>`, not
            // `Arc<McpState>`.
            let oauth_routes = Router::new()
                .route(
                    "/oauth/authorize",
                    get(oauth::authorize_init_handler).post(oauth::authorize_handler),
                )
                .route("/oauth/token", post(oauth::token_handler))
                .with_state(oauth_state.clone());
            base.merge(oauth_routes)
        } else {
            base
        };

        TestServer {
            state,
            oauth_state,
            app,
        }
    }
}

/// In-process MCP server bundle: shared `McpState`, OAuth state for JWT
/// minting, and an axum `Router` ready for `oneshot`-style calls.
pub struct TestServer {
    pub state: Arc<McpState>,
    pub oauth_state: Arc<OAuthState>,
    pub app: Router,
}

impl TestServer {
    /// Start a builder with defaults (`local` / `none` / `0` cost).
    pub fn builder() -> TestServerBuilder {
        TestServerBuilder::default()
    }

    /// Mint a HS256 JWT for `sub` using `TEST_JWT_SECRET`. Matches the
    /// claim shape `oauth::Claims` expects (`iss="mcp.mnemonik.xyz"`,
    /// `aud="mcp"`, etc.).
    pub fn mint_jwt(&self, sub: &str) -> String {
        mint_jwt(sub, TEST_JWT_SECRET)
    }

    /// Mint a HS256 JWT bound to an explicit pubkey identity. T4 needs
    /// this primitive so a single `TestServer` (one SQLite DB) can be
    /// driven by two distinct authenticated callers for the tenant-
    /// isolation scenario. Semantically equivalent to `mint_jwt`, but
    /// named after the caller's intent (the `sub` value is treated as
    /// the tenant's `owner_pubkey`).
    pub fn mint_test_jwt(&self, pubkey: &str) -> String {
        mint_jwt(pubkey, TEST_JWT_SECRET)
    }

    /// Return a small client wrapper whose `call_tool` injects
    /// `Authorization: Bearer <jwt>`. Lets a test pin a single identity
    /// across a sequence of calls without repeating `mint_jwt` and
    /// remembering to thread `Some(&sub)` through `call_tool`. The
    /// returned wrapper borrows the parent `TestServer` (zero-copy of
    /// the underlying `Router`).
    pub fn with_token<'a>(&'a self, jwt: &'a str) -> AuthedClient<'a> {
        AuthedClient { server: self, jwt }
    }

    /// The server keypair's base58 pubkey — convenient default "owner" for
    /// tests that don't care about multi-tenancy. Equal to `claims.sub`
    /// when the test mints a JWT via `self.mint_jwt(&self.server_pubkey())`.
    pub fn server_pubkey(&self) -> String {
        pubkey_base58(&self.state.keypair)
    }

    /// Issue an authenticated `tools/call` to `/mcp`. Returns the parsed
    /// JSON envelope (status code is asserted to be 2xx OR 4xx; callers
    /// inspect `envelope["error"]` for typed-error tests).
    ///
    /// `sub` is the JWT subject (typically equal to `server_pubkey()` for
    /// single-tenant tests). Pass `None` to issue without an auth header;
    /// the middleware then rejects with 401.
    pub async fn call_tool(&self, sub: Option<&str>, name: &str, arguments: Value) -> CallResult {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        });
        let mut builder = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json");
        if let Some(s) = sub {
            builder = builder.header("authorization", format!("Bearer {}", self.mint_jwt(s)));
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let envelope: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            // Some error paths emit non-JSON-RPC error bodies (e.g.
            // payment-required 402 from the x402 path). Return the raw
            // bytes wrapped as a Value::String so the caller still has
            // something to inspect.
            Value::String(String::from_utf8_lossy(&bytes).into())
        });
        CallResult { status, envelope }
    }

    /// Convenience: `call_tool` with `sub = Some(server_pubkey())`.
    pub async fn call_tool_as_server(&self, name: &str, arguments: Value) -> CallResult {
        let owner = self.server_pubkey();
        self.call_tool(Some(&owner), name, arguments).await
    }

    /// Total attestation rows persisted for `signer`. Empty string acts as
    /// "all signers" because `count` is signer-scoped and we want a single
    /// global pre/post assertion in some tests.
    pub fn attestation_count(&self, signer: &str) -> i64 {
        let store = self.state.store.lock().expect("store mutex");
        store.count(signer).unwrap_or(0)
    }

    /// Convenience: total attestation rows scoped to the server keypair
    /// (the typical "default signer" in single-tenant tests).
    pub fn attestation_count_for_server(&self) -> i64 {
        let owner = self.server_pubkey();
        self.attestation_count(&owner)
    }

    /// Fetch a row by Solana tx id (`local:` synthetic id or real
    /// signature) scoped to `owner_pubkey`. Wraps `find_by_tx`. Returns
    /// `None` for a miss OR a wrong-tenant probe (the storage layer
    /// returns `Ok(None)` for both — tenant isolation is enforced at the
    /// SQL predicate per T4). Panics on a SQLite error (test scaffolding
    /// doesn't need a result type).
    pub fn fetch_attestation_by_tx(
        &self,
        tx_id: &str,
        owner_pubkey: &str,
    ) -> Option<mnemonic_core::storage::AttestationRow> {
        let store = self.state.store.lock().expect("store mutex");
        store
            .find_by_tx(tx_id, owner_pubkey)
            .expect("find_by_tx must not error")
    }

    /// Read the persisted `write_mode` column for an attestation by its
    /// Solana tx id. Returns the lowercase string straight from the
    /// SQLite cell so a test can `assert_eq!(..., "local")`. Tests need
    /// this because `AttestationRow` does not (yet) surface `write_mode`
    /// — surfacing it through the public type is T4's job (it adds
    /// `write_mode` to the recall result envelope).
    pub fn write_mode_for_tx(&self, tx_id: &str) -> Option<String> {
        let store = self.state.store.lock().expect("store mutex");
        let conn = store.conn();
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT write_mode FROM attestations WHERE solana_tx = ?",
            rusqlite::params![tx_id],
            |row| row.get::<_, String>(0),
        );
        result.ok()
    }

    /// Number of rows in `attestation_costs` for a given attestation id.
    /// 0 means the cost-recording branch did NOT fire (free / local path);
    /// 1 means it did (paid participate path).
    pub fn attestation_cost_rows(&self, attestation_id: &str) -> i64 {
        let store = self.state.store.lock().expect("store mutex");
        let conn = store.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM attestation_costs WHERE attestation_id = ?",
            rusqlite::params![attestation_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
    }

    /// Read the `api_keys.balance_micro_usdc` for the given api-key.
    /// Returns `None` if the key is unknown. Used by tests that verify a
    /// refund returns the caller's balance to pre-call value.
    pub fn balance_for(&self, api_key: &str) -> Option<i64> {
        let store = self.state.store.lock().expect("store mutex");
        let conn = store.conn();
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?",
            rusqlite::params![api_key],
            |row| row.get::<_, i64>(0),
        );
        result.ok()
    }
}

/// Result of a `call_tool` invocation.
pub struct CallResult {
    pub status: StatusCode,
    pub envelope: Value,
}

impl CallResult {
    /// Return the JSON-RPC `error` object if present. Tests doing typed-
    /// error assertions go through here so the assertion message in a
    /// failure includes the full envelope (via `expect_err_with`).
    pub fn error(&self) -> Option<&Value> {
        self.envelope.get("error")
    }

    /// Expect a JSON-RPC error and return it, panicking with the envelope
    /// pretty-printed if absent.
    pub fn expect_error(&self) -> &Value {
        match self.error() {
            Some(e) => e,
            None => panic!(
                "expected JSON-RPC error but got: {}",
                serde_json::to_string_pretty(&self.envelope).unwrap_or_default()
            ),
        }
    }

    /// Parse the `result.content[0].text` JSON blob into a Value — this is
    /// the MCP 2025-06-18 streamable-HTTP envelope wrapper. Returns the
    /// inner Value so a test can `["write_mode"]` directly.
    pub fn result_text(&self) -> Value {
        let text = self.envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("");
        serde_json::from_str(text).unwrap_or(Value::Null)
    }
}

/// Authenticated client bound to a specific bearer token (and therefore a
/// specific `owner_pubkey` resolved by the OAuth middleware from the JWT
/// `sub` claim). Constructed via [`TestServer::with_token`]; intended for
/// multi-tenant test scenarios such as T4's tenant-isolation regression.
pub struct AuthedClient<'a> {
    server: &'a TestServer,
    jwt: &'a str,
}

impl AuthedClient<'_> {
    /// Issue an authenticated `tools/call` to `/mcp` with the bound JWT
    /// in the `Authorization: Bearer <jwt>` header. Returns the parsed
    /// JSON envelope; the underlying transport behaviour matches
    /// [`TestServer::call_tool`] byte-for-byte.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> CallResult {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.jwt))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = self.server.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let envelope: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        CallResult { status, envelope }
    }
}

// ── refresh-token-rotation fixture functions (Task 4) ────────────────────────
//
// Public helpers consumed by the Task 5 integration suite (`oauth_refresh_e2e.rs`).
// They live alongside the `TestServer` shape because each one operates on a
// fully-built server (HTTP routes, OAuthState, embedded Connection) rather
// than the bare `McpState` that `test_support` provides. Every fixture
// assumes `TestServerBuilder::with_oauth_token(true)` was set — calling them
// against a server without OAuth routes mounted will panic with a 404.

/// Default loopback redirect URI used by `bootstrap_oauth`. Loopback URIs
/// are allowed for any `client_id` per RFC 8252 §7.3 — see
/// `oauth::allowed_redirect`. The constant is exported so Task 5 tests that
/// drive `/oauth/token` directly (e.g. AC1 `test_authcode_grant_returns_refresh`)
/// can echo the same `redirect_uri` when needed.
pub const TEST_REDIRECT_URI: &str = "http://127.0.0.1:9999/cb";

/// PKCE S256 code_challenge from a code_verifier (URL-safe base64, no pad).
fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest)
}

/// Generate 32 random URL-safe-base64 bytes (used as the PKCE
/// `code_verifier` and the `state` parameter — both are opaque
/// client-chosen strings that survive a round-trip through the server).
fn rand_url_safe_32() -> String {
    use rand::RngCore;
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, raw)
}

/// Drive the production `GET /oauth/authorize` + `POST /oauth/authorize` flow
/// against the in-process router and return the resulting
/// `(authorization_code, code_verifier, redirect_uri)` triple.
///
/// Task 5 uses the triple to POST `/oauth/token` with
/// `grant_type=authorization_code` itself — that lets each AC exercise its
/// own content-type and field shape (AC1, AC10 form/JSON parity) without
/// the helper pre-committing to a single one. The helper deliberately does
/// NOT call `/oauth/token`: returning raw inputs to the token redemption
/// keeps the fixture surface honest about which value is which.
///
/// Production parity (Finding 4 / Task 4 spec §1): the helper drives the
/// real `/oauth/authorize` GET (server mints CBOR challenge, inserts pending
/// row) → Ed25519-signs the bytes with a fresh keypair → POSTs the signature
/// back. No direct SQL inserts into `OAuthState.codes`; no shortcut around
/// `authorize_handler`.
pub async fn bootstrap_oauth(server: &TestServer) -> (String, String, String) {
    use solana_sdk::signature::{Keypair, Signer};

    let kp = Keypair::new();
    let signer_pubkey = kp.pubkey().to_string();
    let verifier = rand_url_safe_32();
    let code_challenge = pkce_challenge(&verifier);
    let csrf_state = rand_url_safe_32();
    let redirect_uri = TEST_REDIRECT_URI;

    // GET /oauth/authorize — JSON mode (Accept: application/json plus
    // `pubkey` query param triggers the JSON branch per authorize_init_handler).
    // Pre-binding `pubkey` lets the server's `expected_pubkey` check pass on
    // the subsequent POST — RFC 6749 §4.1.3 binding is observable in the
    // production path too.
    let get_uri = format!(
        "/oauth/authorize?client_id=test-client&redirect_uri={redirect}\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}&pubkey={pk}",
        redirect = urlencode(redirect_uri),
        challenge = urlencode(&code_challenge),
        state = urlencode(&csrf_state),
        pk = urlencode(&signer_pubkey),
    );
    let get_req = Request::builder()
        .method("GET")
        .uri(&get_uri)
        .header("accept", "application/json")
        .body(Body::empty())
        .expect("build GET /oauth/authorize");
    let get_resp = server
        .app
        .clone()
        .oneshot(get_req)
        .await
        .expect("oneshot GET /oauth/authorize");
    let get_status = get_resp.status();
    let get_bytes = get_resp
        .into_body()
        .collect()
        .await
        .expect("collect GET body")
        .to_bytes();
    assert_eq!(
        get_status,
        200,
        "GET /oauth/authorize must return 200 (JSON mode); body={}",
        String::from_utf8_lossy(&get_bytes)
    );
    let init: Value = serde_json::from_slice(&get_bytes).expect("parse AuthorizeInitResponse JSON");
    let challenge_b64 = init["challenge_cbor"]
        .as_str()
        .expect("challenge_cbor field present")
        .to_string();
    let challenge_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &challenge_b64)
            .expect("decode challenge_cbor base64");

    // Sign the canonical-CBOR challenge bytes with raw Ed25519 — matches the
    // post-refactor wire contract enforced at `authorize_handler` (signature
    // MUST be 64 bytes).
    let sig = mnemonic_core::identity::sign_bytes(&kp, &challenge_bytes);
    let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig);

    // POST /oauth/authorize — server verifies sig, emits authorization_code.
    let post_body = serde_json::json!({
        "state": csrf_state,
        "signature": sig_b64,
        "signer_pubkey": signer_pubkey,
    });
    let post_req = Request::builder()
        .method("POST")
        .uri("/oauth/authorize")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&post_body).unwrap()))
        .expect("build POST /oauth/authorize");
    let post_resp = server
        .app
        .clone()
        .oneshot(post_req)
        .await
        .expect("oneshot POST /oauth/authorize");
    let post_status = post_resp.status();
    let post_bytes = post_resp
        .into_body()
        .collect()
        .await
        .expect("collect POST body")
        .to_bytes();
    assert_eq!(
        post_status,
        200,
        "POST /oauth/authorize must return 200; body={}",
        String::from_utf8_lossy(&post_bytes)
    );
    let authz: Value = serde_json::from_slice(&post_bytes).expect("parse AuthorizeResponse JSON");
    let code = authz["code"]
        .as_str()
        .expect("authorize response carries `code`")
        .to_string();
    (code, verifier, redirect_uri.to_string())
}

/// Minimal URL-encoder for query-string values. Avoids pulling in
/// `percent-encoding` for the helper (already a transitive dep through
/// `reqwest`, but importing it from `_helpers` would require a `pub use`
/// in `lib.rs`). Mirrors the in-handler `urlencoding_encode` private helper
/// in `oauth/mod.rs` so the round-trip is byte-identical.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Drive a single `grant_type=refresh_token` rotation against `/oauth/token`
/// and return the freshly-emitted `(access_token, refresh_token)` pair.
///
/// Task 5 AC12 ("10 parallel rotations") asserts byte-identity on BOTH
/// fields — returning both as owned `String`s here surfaces the contract to
/// the test layer rather than reducing it to a single field.
///
/// Happy-path only: panics on non-200. Negative paths (expired, replay,
/// revoked) belong in Task 5 test bodies where the caller controls the
/// status-code assertions directly.
pub async fn rotate(server: &TestServer, refresh: &str) -> (String, String) {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .expect("build POST /oauth/token");
    let resp = server
        .app
        .clone()
        .oneshot(req)
        .await
        .expect("oneshot POST /oauth/token");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    assert_eq!(
        status,
        200,
        "rotate POST /oauth/token must return 200; body={}",
        String::from_utf8_lossy(&bytes)
    );
    let v: Value = serde_json::from_slice(&bytes).expect("parse TokenResponse JSON");
    let access = v["access_token"]
        .as_str()
        .expect("access_token field present")
        .to_string();
    let refresh = v["refresh_token"]
        .as_str()
        .expect("refresh_token field present on Branch A rotation")
        .to_string();
    (access, refresh)
}

/// Insert a synthetic `refresh_tokens` row whose `expires_at` is one minute
/// in the past, returning the `(plaintext, family_id)` pair. The plaintext
/// is what Task 5's Branch-D test (`test_expired_refresh_rejected_no_family_revoke`,
/// AC5) POSTs to `/oauth/token`; the `family_id` is what the same test
/// passes to [`family_has_unrevoked_rows`] after the expected 400 to verify
/// the family was NOT revoked (Branch D is "expired, not an attack").
///
/// Why 60 s in the past rather than 1 s: a sub-second cushion would race
/// against the `SystemTime::now()` read inside `refresh::rotate_inner`
/// under cgroup throttling. 60 s is far enough to be stable.
///
/// No evictor task runs in the test harness — `TestServerBuilder::build()`
/// does not call `refresh::start_evictor`; only `main.rs::run_http` does.
/// This row is therefore safe from eviction regardless of timing (and
/// even if the evictor were spawned, `expires_at = now - 60` exactly
/// equals the `EVICTOR_GRACE_SECS = 60` cutoff, so the strict
/// `expires_at < cutoff` predicate would still keep the row alive).
///
/// Mutex discipline (CLAUDE.md): the connection guard is acquired, the SQL
/// is run, the guard is dropped — all before the function returns. No
/// `.await` is held under the guard because the body itself is synchronous.
pub async fn insert_expired_refresh_for_test(server: &TestServer) -> (String, String) {
    use rand::RngCore;

    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let plaintext = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, raw);
    let token_hash = refresh::hash_refresh_token(&server.oauth_state.refresh_salt, &plaintext);
    let family_id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("now")
        .as_secs();
    let issued_at = now - 120;
    let expires_at = now - 60;

    {
        let guard = server
            .oauth_state
            .refresh_store
            .lock()
            .expect("refresh_store mutex");
        guard
            .execute(
                "INSERT INTO refresh_tokens
                    (token_hash, sub, google_sub, issued_at, expires_at,
                     revoked, rotated_at, rotated_to, family_id)
                    VALUES (?1, 'expired-refresh-test', NULL, ?2, ?3, 0, NULL, NULL, ?4)",
                rusqlite::params![token_hash, issued_at, expires_at, family_id],
            )
            .expect("INSERT expired refresh row");
        // Guard drops here — synchronous block, no `.await` follows under
        // the lock.
    }

    (plaintext, family_id)
}

/// SELECT helper for Task 5 AC5: returns `true` when at least one row in the
/// family has `revoked = 0`. Used to assert that Branch D (expired refresh)
/// did NOT trigger `family_revoke` — siblings in the same family must
/// remain reachable.
///
/// Empty family → `false`. All-revoked family → `false`. Mixed family with
/// at least one `revoked = 0` row → `true`.
pub fn family_has_unrevoked_rows(server: &TestServer, family_id: &str) -> bool {
    let guard = server
        .oauth_state
        .refresh_store
        .lock()
        .expect("refresh_store mutex");
    let count: i64 = guard
        .query_row(
            "SELECT COUNT(*) FROM refresh_tokens WHERE family_id = ?1 AND revoked = 0",
            rusqlite::params![family_id],
            |row| row.get(0),
        )
        .expect("count unrevoked rows in family");
    count > 0
}
