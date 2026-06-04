// Some helpers in this module are consumed only by a subset of the
// delivery_guarantee.rs test cases. We accept dead_code warnings at module
// scope so the shape stays reusable across future T3-adjacent test files.
#![allow(dead_code)]

//! Test harness for T3 (`modes-user-choice` delivery guarantee).
//!
//! Two httpmock servers per test — one for Arweave (POST `/tx`, GET
//! `/{txid}`, GET `/mine`) and one for Solana JSON-RPC (POST `/` answering
//! `getLatestBlockhash`, `sendTransaction`, `getSignatureStatuses`, etc).
//!
//! The Arweave server can be steered by the test through the [`MockArweave`]
//! façade: `happy()`, `corrupted_get()`, `different_signature_bytes()`,
//! `read_fails()`. Each maps to a specific delivery-stage failure mode:
//! - `corrupted_get` → stage `"verify"` (CBOR decode fails OR hash mismatch).
//! - `different_signature_bytes` → stage `"verify"` (CBOR/COSE OK but
//!   signature is from a different key OR hash differs from anchor).
//! - `read_fails` → stage `"refetch"` (GET 404 / 500 — refetch budget
//!   exhausts).
//! - `happy` → all stages pass (anchor + GET return identical bytes;
//!   recall depends on the embedder which is deterministic).
//!
//! The Solana mock always returns a canned `getLatestBlockhash` +
//! `sendTransaction` + `getSignatureStatuses(confirmed)` so `write_memo`
//! succeeds; we treat the chain anchor as orthogonal to T3's delivery
//! check.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use httpmock::prelude::*;
use mnemonic_core::codec::{schema, sign::sign_artifact};
use mnemonic_core::identity::pubkey_base58;
use mnemonic_mcp::{
    mcp::{mcp_handler, McpState},
    test_support::mock_state_for_delivery,
};
use serde_json::Value;
use solana_sdk::signature::Keypair;
use tower::ServiceExt;

/// 32-byte secret kept for compat with the harness signature; the T3 tests
/// intentionally do NOT mount the OAuth middleware (see `build_state_and_router`).
pub const TEST_JWT_SECRET: &[u8; 32] = b"delivery-guarantee-secret-32-bts";

/// Fixed base58 blockhash + tx-signature returned by the Solana mock. The
/// signature is 88 base58 chars per Solana convention.
pub const MOCK_BLOCKHASH: &str = "11111111111111111111111111111111";
pub const MOCK_SOL_SIG: &str =
    "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

/// Steerable Arweave mock. Wraps an `httpmock::MockServer` and tracks the
/// behaviour the test asked for so the next request matches it.
pub struct MockArweave {
    pub server: MockServer,
    /// Returned by GET `/{tx_id}`; set via the `happy*` / `corrupted_*`
    /// helpers below.
    behaviour: ArweaveBehaviour,
    /// The tx_id POST `/tx` will assign to the upload.
    pub fixed_tx_id: String,
}

enum ArweaveBehaviour {
    /// GET returns the bytes the client just uploaded (happy path).
    Happy,
    /// GET returns random bytes that fail CBOR/COSE decode (verify stage).
    CorruptedGet,
    /// GET returns valid COSE bytes signed by a DIFFERENT key (verify stage —
    /// hash will differ from the anchor). The bytes are pre-built at
    /// `happy_with_substituted_bytes` time and stored here.
    SubstitutedBytes(Vec<u8>),
    /// GET returns 404 (refetch budget exhausts).
    ReadFails,
}

impl MockArweave {
    pub fn happy(fixed_tx_id: impl Into<String>) -> Self {
        let server = MockServer::start();
        Self {
            server,
            behaviour: ArweaveBehaviour::Happy,
            fixed_tx_id: fixed_tx_id.into(),
        }
    }

    pub fn corrupted_get(fixed_tx_id: impl Into<String>) -> Self {
        let server = MockServer::start();
        Self {
            server,
            behaviour: ArweaveBehaviour::CorruptedGet,
            fixed_tx_id: fixed_tx_id.into(),
        }
    }

    pub fn different_signature_bytes(fixed_tx_id: impl Into<String>) -> Self {
        // Sign a synthetic artifact with a throwaway keypair. The COSE bytes
        // are well-formed (verify_cose decodes successfully) but the
        // content_hash will differ from what we anchored — verify stage
        // returns `tampered`, which the delivery check treats as fail.
        let kp = Keypair::new();
        let artifact = serde_json::json!({
            "artifact_id": "substituted",
            "type": "memory",
            "schema_version": 1,
            "content": "substituted content",
            "producer": "did:sol:substituted",
            "created_at": "2026-06-01T00:00:00Z",
            "tags": [],
        });
        let signed = sign_artifact(&artifact, &schema::MEMORY_V1, &kp).expect("sign");
        let server = MockServer::start();
        Self {
            server,
            behaviour: ArweaveBehaviour::SubstitutedBytes(signed.cose_bytes),
            fixed_tx_id: fixed_tx_id.into(),
        }
    }

    pub fn read_fails(fixed_tx_id: impl Into<String>) -> Self {
        let server = MockServer::start();
        Self {
            server,
            behaviour: ArweaveBehaviour::ReadFails,
            fixed_tx_id: fixed_tx_id.into(),
        }
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }

    /// Install the mock endpoints. Returns the POST `/tx` mock handle so a
    /// test can call `.hits()` on it (e.g. to assert no further chain spend
    /// after a quota short-circuit).
    pub fn install(&self) -> httpmock::Mock<'_> {
        let tx_id = self.fixed_tx_id.clone();

        // POST /tx — arlocal write path. Always returns the fixed tx id.
        let post_mock = self.server.mock(|when, then| {
            when.method(POST).path("/tx");
            then.status(200)
                .header("content-type", "application/json")
                .body(format!(r#"{{"id":"{tx_id}"}}"#));
        });

        // GET /mine — arlocal noop.
        self.server.mock(|when, then| {
            when.method(GET).path("/mine");
            then.status(200).body("mined");
        });

        // GET /{tx_id} — behaviour-dependent. The arlocal write path
        // discards the POST response body and self-mints the tx_id from a
        // PRNG seed, so we cannot pre-register a specific path. Instead,
        // we install a permissive GET matcher that matches anything
        // *except* `/mine` (the mine endpoint also responds to GET).
        //
        // `path_matches(r"^/[^m].*$|^/$")` would work but is brittle; the
        // simpler pattern is to register the `/mine` GET FIRST (which
        // claims that path) and then a broad `GET /` matcher for
        // everything else. httpmock matches the most-specific path first.
        let any_path_re = regex::Regex::new(r"^/[A-Za-z0-9_\-]+$").expect("valid regex");
        match &self.behaviour {
            ArweaveBehaviour::Happy => {
                self.server.mock(|when, then| {
                    when.method(GET).path_matches(any_path_re.clone());
                    then.status(404).body("not seeded yet");
                });
            }
            ArweaveBehaviour::CorruptedGet => {
                self.server.mock(|when, then| {
                    when.method(GET).path_matches(any_path_re.clone());
                    then.status(200).body("not-the-cose-bytes-you-anchored");
                });
            }
            ArweaveBehaviour::SubstitutedBytes(bytes) => {
                let bytes = bytes.clone();
                self.server.mock(|when, then| {
                    when.method(GET).path_matches(any_path_re.clone());
                    then.status(200).body(bytes);
                });
            }
            ArweaveBehaviour::ReadFails => {
                self.server.mock(|when, then| {
                    when.method(GET).path_matches(any_path_re.clone());
                    then.status(404).body("not found");
                });
            }
        }
        post_mock
    }

    /// For the happy path: after `sign_memory` writes the COSE bytes, the
    /// test calls this with the captured upload body so subsequent GETs
    /// return them. Until then the placeholder 404 from `install` stands.
    pub fn seed_get_bytes(&self, bytes: Vec<u8>) {
        let tx_id = self.fixed_tx_id.clone();
        self.server.mock(|when, then| {
            when.method(GET).path(format!("/{tx_id}"));
            then.status(200).body(bytes);
        });
    }
}

/// Steerable Solana JSON-RPC mock. Always returns canned responses so
/// `write_memo` succeeds. The chain anchor is orthogonal to T3's delivery
/// check — we focus failure modes on the Arweave side.
pub struct MockSolana {
    pub server: MockServer,
}

impl MockSolana {
    pub fn happy() -> Self {
        let server = MockServer::start();

        // The Solana JSON-RPC client posts everything to `/` and switches
        // on the `method` field. httpmock matches on URL + body content;
        // we install three mocks keyed on the `method` substring.
        // `getLatestBlockhash` returns a fixed blockhash.
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(r#""method":"getLatestBlockhash""#);
            then.status(200).header("content-type", "application/json").body(
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"result":{{"context":{{"slot":1}},"value":{{"blockhash":"{MOCK_BLOCKHASH}","lastValidBlockHeight":100}}}}}}"#
                ),
            );
        });

        // `sendTransaction` returns a fixed signature.
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(r#""method":"sendTransaction""#);
            then.status(200)
                .header("content-type", "application/json")
                .body(format!(
                    r#"{{"jsonrpc":"2.0","id":1,"result":"{MOCK_SOL_SIG}"}}"#
                ));
        });

        // `getSignatureStatuses` returns `confirmed` so `confirm_tx` exits
        // on the first poll.
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(r#""method":"getSignatureStatuses""#);
            then.status(200).header("content-type", "application/json").body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"context":{"slot":1},"value":[{"slot":1,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]}}"#,
            );
        });

        Self { server }
    }

    /// Like [`happy`], plus a `getTransaction` mock that returns a
    /// synthetic USDC transfer for the supplied `tx_sig`. The transfer
    /// shows a `cost`-micro-USDC delta on `treasury`'s SPL account for
    /// `usdc_mint`, which is exactly what `verify_usdc_transfer` walks.
    ///
    /// Used by the T3.5 `demotion_on_x402_*` tests so `check_payment`
    /// passes its USDC-transfer verification without us having to mint a
    /// real on-chain transaction.
    pub fn happy_with_x402_payment(
        tx_sig: &str,
        treasury: &str,
        usdc_mint: &str,
        cost: u64,
    ) -> Self {
        let me = Self::happy();
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"meta":{{"err":null,"preTokenBalances":[{{"accountIndex":0,"owner":"{treasury}","mint":"{usdc_mint}","uiTokenAmount":{{"amount":"0","decimals":6,"uiAmount":0.0,"uiAmountString":"0"}}}}],"postTokenBalances":[{{"accountIndex":0,"owner":"{treasury}","mint":"{usdc_mint}","uiTokenAmount":{{"amount":"{cost}","decimals":6,"uiAmount":0.0,"uiAmountString":"0"}}}}]}}}}}}"#
        );
        // The Solana client posts every RPC to `/`; httpmock dispatches by
        // body content. We match on the tx_sig substring so this mock only
        // answers `getTransaction` calls for THIS specific signature; any
        // other signature falls through to httpmock's 404 default, which
        // makes a misconfigured test fail loudly instead of silently
        // mocking a different proof.
        me.server.mock(|when, then| {
            when.method(POST)
                .body_includes(r#""method":"getTransaction""#)
                .body_includes(tx_sig);
            then.status(200)
                .header("content-type", "application/json")
                .body(body);
        });
        me
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }
}

/// Build a `McpState` + axum `Router` ready for `oneshot`-style calls,
/// pointed at the supplied Arweave + Solana mocks.
///
/// **No OAuth middleware** — the T3 integration tests target the *inline*
/// participate flow (no `jwt_sub`), which today is the only path that runs
/// `sign_memory_inline` (the deferred-signing branch fires when a JWT is
/// present and the mode isn't explicit-local). For production HTTP clients
/// the OAuth layer attaches JWT claims; T3's delivery-guarantee code path
/// is orthogonal to that. The payment-gate's `Bearer` header is still
/// honoured for `payment_mode=balance` — `extract_api_key` reads the same
/// Authorization header without needing a verified JWT to back it.
pub fn build_state_and_router(
    arweave_url: &str,
    solana_url: &str,
    payment_mode: &str,
    cost_micro_usdc: i64,
    quota_threshold: u32,
    quota_window: Duration,
    refetch_timeout: Duration,
) -> (Arc<McpState>, Router) {
    let state = mock_state_for_delivery(
        "full",
        payment_mode,
        cost_micro_usdc,
        arweave_url,
        solana_url,
        quota_threshold,
        quota_window,
        refetch_timeout,
        "",
        "",
    );
    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .with_state(state.clone());
    (state, app)
}

/// Variant of [`build_state_and_router`] that wires `treasury_pubkey` and
/// `usdc_mint` into the `McpState` so the x402 payment-gate (`check_payment`
/// → `verify_usdc_transfer`) has destination addresses to match against.
///
/// Used exclusively by the T3.5 `demotion_on_x402_*` tests; for `balance`
/// or `none` payment modes the regular `build_state_and_router` is enough.
#[allow(clippy::too_many_arguments)]
pub fn build_state_and_router_x402(
    arweave_url: &str,
    solana_url: &str,
    cost_micro_usdc: i64,
    quota_threshold: u32,
    quota_window: Duration,
    refetch_timeout: Duration,
    treasury_pubkey: &str,
    usdc_mint: &str,
) -> (Arc<McpState>, Router) {
    let state = mock_state_for_delivery(
        "full",
        "x402",
        cost_micro_usdc,
        arweave_url,
        solana_url,
        quota_threshold,
        quota_window,
        refetch_timeout,
        treasury_pubkey,
        usdc_mint,
    );
    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .with_state(state.clone());
    (state, app)
}

/// Seed an `api_keys` row with the given balance and return the api_key.
/// Used by the T3 tests to obtain a Bearer the payment middleware accepts
/// for the `participate` path. No JWT is involved (the OAuth layer is not
/// mounted in `build_state_and_router`); the bearer string is just an
/// identifier the payment-gate looks up against `api_keys.api_key`.
pub fn seed_api_key_jwt(state: &McpState, balance_micro_usdc: i64) -> String {
    let owner = pubkey_base58(&state.keypair);
    // Use a fresh UUID per call so concurrent tests don't collide on the
    // PRIMARY KEY constraint. Prefix matches the production `mnm_` shape.
    let api_key = format!("mnm_{}", uuid::Uuid::new_v4().simple());
    let store = state.store.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO api_keys (api_key, owner_pubkey, balance_micro_usdc, created_at) VALUES (?,?,?,?)",
            rusqlite::params![api_key, owner, balance_micro_usdc, now],
        )
        .expect("seed api_keys row");
    api_key
}

/// Read the current balance for an api_key (= JWT for these tests). Returns
/// 0 on a miss, matching test ergonomics.
pub fn balance_for(state: &McpState, api_key: &str) -> i64 {
    let store = state.store.lock().unwrap();
    let conn = store.conn();
    conn.query_row(
        "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?",
        rusqlite::params![api_key],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
}

/// Issue a `sign_memory { mode: "participate", content }` against the
/// supplied router with the supplied bearer (JWT + api_key are the same
/// string). Returns (status, envelope).
pub async fn call_sign_memory_participate(
    app: &Router,
    bearer: &str,
    content: &str,
) -> (StatusCode, Value) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "mnemonic_sign_memory",
            "arguments": {"content": content, "mode": "participate"},
        },
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let envelope: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, envelope)
}

/// Issue a `sign_memory { mode: "participate", content }` with an
/// `X-Payment` header pointing at the supplied `tx_sig`. The payload is
/// the raw JSON shape that `payment::extract_x402_proof` accepts (it
/// tries raw JSON before falling back to base64).
///
/// The on-chain payment proof itself is mocked elsewhere (see
/// [`MockSolana::happy_with_x402_payment`]); this helper only constructs
/// the HTTP request.
pub async fn call_sign_memory_participate_x402(
    app: &Router,
    tx_sig: &str,
    content: &str,
) -> (StatusCode, Value) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "mnemonic_sign_memory",
            "arguments": {"content": content, "mode": "participate"},
        },
    });
    // Raw JSON shape per X402PaymentProof; extract_x402_proof tries this
    // first before the base64 fallback so no encoding round-trip is needed.
    let x_payment = serde_json::json!({
        "tx_sig": tx_sig,
        "network": "solana-mainnet",
    })
    .to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("x-payment", x_payment)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let envelope: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, envelope)
}

/// Parse the `result.content[0].text` JSON blob into a Value — this is the
/// MCP 2025-06-18 streamable-HTTP envelope wrapper. Returns the inner Value
/// so a test can `["delivery_receipt"]` directly.
pub fn result_text(envelope: &Value) -> Value {
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    serde_json::from_str(text).unwrap_or(Value::Null)
}
