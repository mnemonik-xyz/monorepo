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
    oauth::{self, OAuthState},
    test_support::{mint_jwt, mock_state_with},
};
use serde_json::Value;
use tower::ServiceExt;

/// 32-byte JWT secret shared across all `TestServer` instances. The
/// production middleware requires `>= 32 bytes`; this matches the existing
/// `TEST_JWT_SECRET` constants spread across `mcp/tests/*.rs`.
pub const TEST_JWT_SECRET: &[u8; 32] = b"modes-per-request-secret-32-byts";

/// Builder for `TestServer`. All fields are optional with sensible defaults
/// (`storage_mode=local`, `payment_mode=none`, no per-write cost). Mirror
/// the names in `Config::from_env` so tests read like deployment
/// declarations.
pub struct TestServerBuilder {
    storage_mode: String,
    payment_mode: String,
    sign_memory_cost_micro_usdc: i64,
}

impl Default for TestServerBuilder {
    fn default() -> Self {
        Self {
            storage_mode: "local".into(),
            payment_mode: "none".into(),
            sign_memory_cost_micro_usdc: 0,
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

    /// Materialise into a `TestServer`.
    pub fn build(self) -> TestServer {
        let state = mock_state_with(
            &self.storage_mode,
            &self.payment_mode,
            self.sign_memory_cost_micro_usdc,
        );
        let oauth_state = Arc::new(OAuthState::new(TEST_JWT_SECRET));
        let app = Router::new()
            .route("/mcp", post(mcp_handler))
            .route("/api/pending/{correlation_id}", get(get_pending_handler))
            .route("/api/sign-callback", post(sign_callback_handler))
            .layer(middleware::from_fn_with_state(
                oauth_state.clone(),
                oauth::bearer_auth_middleware,
            ))
            .with_state(state.clone());
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
    /// signature). Wraps `find_by_tx`. Returns `None` for a miss; panics
    /// on a SQLite error (test scaffolding doesn't need a result type).
    pub fn fetch_attestation_by_tx(
        &self,
        tx_id: &str,
    ) -> Option<mnemonic_core::storage::AttestationRow> {
        let store = self.state.store.lock().expect("store mutex");
        store.find_by_tx(tx_id).expect("find_by_tx must not error")
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
