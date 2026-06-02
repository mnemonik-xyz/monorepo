//! MCP protocol handler — JSON-RPC 2.0 dispatcher for both stdio and HTTP.
//!
//! HTTP transport uses MCP **streamable HTTP** per the 2025 specification
//! (`Content-Type: application/x-ndjson`, `Transfer-Encoding: chunked`,
//! one JSON-RPC envelope per newline-terminated frame). See Decision 1 in
//! `work/mnemonic-integrations/tech-spec.md`.

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use bytes::Bytes;
use futures::stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_sdk::signature::Keypair;

use crate::{
    api::BootstrapTickets, llm::LlmClient, payment, pending::PendingBundles,
    pricing::PricingEngine, tools,
};
use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::embed::Embedder;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::{SqliteStore, WriteMode};
use std::convert::Infallible;
use std::sync::Arc;

/// JSON-RPC 2.0 request or notification.
///
/// Per JSON-RPC 2.0 spec, notifications (e.g. MCP `notifications/initialized`,
/// `notifications/cancelled`, `notifications/progress`) MUST NOT have an `id`
/// field — and the server MUST NOT respond to them. Requiring `id` here would
/// reject every MCP notification with a parse error, breaking connector setup
/// (Cursor / Claude.ai send `notifications/initialized` immediately after the
/// `initialize` response per MCP spec).
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC protocol version — deserialized so callers must supply it, but
    /// we do not branch on the value (we always respond with "2.0").
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// `None` = notification (no response sent); `Some` = request expecting a response.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    /// True if this is a JSON-RPC notification (no `id` field, no response expected).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    /// Optional structured error data (JSON-RPC 2.0 §5.1 "Error object" allows
    /// an arbitrary `data` field). Used by the typed errors introduced in T2:
    /// - `-32010 UnsupportedMode` carries `{kind, requested, supported}`.
    /// - `-32602 InvalidParams` carries `{field, received}`.
    ///
    /// Older `-32603 InternalError` / `-32600 InvalidRequest` envelopes
    /// continue to omit this field — `skip_serializing_if` keeps the
    /// pre-T2 wire shape byte-identical for legacy error paths
    /// (golden-fixture compat for the shipped chrome-extension).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Construct a JSON-RPC error with no `data` field. Used for legacy code
    /// paths that pre-date the typed-error helpers (T2). Keeps the on-the-wire
    /// shape `{code, message}` byte-identical to the pre-T2 envelope.
    pub fn simple(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

// ── Typed JSON-RPC errors (T2 — modes-user-choice) ──────────────────────────
//
// Two new error codes covering the per-request `mode` field on
// `mnemonic_sign_memory`. See work/modes-user-choice/tech-spec.md
// §"Typed errors" for the wire-format contract.

/// `-32010 UnsupportedMode` — the caller requested a mode the server cannot
/// serve (e.g. `participate` on a `STORAGE_MODE=local` deploy). Never used as
/// a silent downgrade; the user explicitly asked for chain-anchoring and the
/// server must say "I can't" so the client picks `local` or another operator.
///
/// `data` shape: `{kind: "UnsupportedMode", requested, supported}`.
pub fn unsupported_mode(requested: &str, supported: &[&str]) -> JsonRpcError {
    JsonRpcError {
        code: -32010,
        message: "Unsupported mode".to_string(),
        data: Some(serde_json::json!({
            "kind": "UnsupportedMode",
            "requested": requested,
            "supported": supported,
        })),
    }
}

/// `-32602 InvalidParams` — a request parameter is malformed. The T2 resolver
/// emits this for `mode` values that are not exactly `"local"` or
/// `"participate"` (case-variant, whitespace, null, non-string, unknown).
/// The verbatim received value is echoed back in `data.received` so the
/// caller can diff against its own outgoing payload.
///
/// `data` shape: `{field, received}`.
pub fn invalid_params(field: &str, received: &Value) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: "Invalid params".to_string(),
        data: Some(serde_json::json!({
            "field": field,
            "received": received,
        })),
    }
}

/// `whoami` discoverability envelope — derived once at process start from
/// `Config` (storage_mode + payment_mode + pricing engine snapshot) and
/// returned through `mnemonic_whoami` so clients learn what the server can
/// serve **before** they try to write. See user-spec §"Discoverability через
/// whoami" and tech-spec Decision 3.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    /// Modes the server is willing to accept for `sign_memory.mode`. A pure
    /// `STORAGE_MODE=local` deploy returns `["local"]`; a `full` deploy
    /// returns `["local", "participate"]`.
    pub supported_modes: Vec<&'static str>,
    /// Mode applied when the caller omits the `mode` field. Always `"local"`
    /// for V1 (user-spec invariant — "default `local`").
    pub default_mode: &'static str,
    /// Price metadata for the `participate` mode. `None` on a local-only
    /// server (the field renders as JSON `null`); `Some` with `amount_cents`
    /// and `payment_methods` on any `full`-mode server.
    pub participate_cost: Option<ParticipateCost>,
}

impl Envelope {
    /// True if `supported_modes` contains `"participate"`. Used by the
    /// `sign_memory` entrypoint to reject `participate` requests with a typed
    /// `UnsupportedMode` instead of a silent downgrade.
    pub fn supports_participate(&self) -> bool {
        self.supported_modes.contains(&"participate")
    }

    /// Derive the envelope from operator-side env-vars and the current
    /// pricing snapshot. Pure — no I/O, no clock; safe to call at process
    /// start AND inside tests.
    ///
    /// `storage_mode` resolves `supported_modes`. `payment_mode` resolves
    /// `participate_cost.payment_methods`. `price_micro_usdc` is divided by
    /// `10_000` to produce USD cents — the pricing engine quotes in
    /// micro-USDC (1e-6 USD).
    pub fn from_config(storage_mode: &str, payment_mode: &str, price_micro_usdc: i64) -> Self {
        if storage_mode == "local" {
            // Local-only deploy. The server CANNOT anchor and must say so
            // up front — `participate_cost` is null (the field is present
            // in the JSON, not omitted, so clients can distinguish
            // "no participate support" from "old server without envelope").
            return Self {
                supported_modes: vec!["local"],
                default_mode: "local",
                participate_cost: None,
            };
        }
        // Full deploy: micro-USDC → cents (round half-to-zero — the integer
        // truncation matches the existing `record_attestation_cost` math).
        let amount_cents = (price_micro_usdc / 10_000).max(0);
        let payment_methods: Vec<&'static str> = match payment_mode {
            "none" => Vec::new(),
            "balance" => vec!["balance"],
            "x402" => vec!["x402"],
            "both" => vec!["x402", "balance"],
            // Defensive: an unknown mode collapses to empty methods. Operator
            // misconfiguration shouldn't leak as a misleading payment menu.
            _ => Vec::new(),
        };
        Self {
            supported_modes: vec!["local", "participate"],
            default_mode: "local",
            participate_cost: Some(ParticipateCost {
                currency: "USD",
                amount_cents,
                payment_methods,
            }),
        }
    }
}

/// Price + payment-method tuple for `participate` writes. Serialised as part
/// of `Envelope`. `currency` is currently always `"USD"`; `amount_cents` is
/// the per-write cost in USD cents; `payment_methods` enumerates how the
/// caller can pay (`["x402"]`, `["balance"]`, `["x402","balance"]`, or empty
/// for `PAYMENT_MODE=none` self-operator deploys).
#[derive(Debug, Clone, Serialize)]
pub struct ParticipateCost {
    pub currency: &'static str,
    pub amount_cents: i64,
    pub payment_methods: Vec<&'static str>,
}

/// Shared state for the MCP server.
/// AttestationStore uses rusqlite (not Sync), so we wrap in std::sync::Mutex
/// and never hold the lock across await points.
pub struct McpState {
    pub keypair: Keypair,
    pub solana: SolanaClient,
    pub arweave: ArweaveClient,
    pub store: std::sync::Mutex<SqliteStore>,
    pub embedder: Box<dyn Embedder>,
    pub compressor: EmbeddingCompressor,

    // Payment config
    /// "none" | "balance" | "x402" | "both"
    pub payment_mode: String,
    pub treasury_pubkey: String,
    pub usdc_mint: String,
    pub sign_memory_cost_micro_usdc: i64,

    // Dynamic pricing
    pub pricing: Arc<PricingEngine>,
    /// Solana memo tx fee in lamports (passed to CostHint).
    pub sol_tx_fee_lamports: u64,

    // Storage mode
    /// "local" (default, free, SQLite only) or "full" (Arweave + Solana + SQLite)
    pub storage_mode: String,

    // Ollama / RAG (used by chat.rs and seed.rs in Tasks 2-3)
    /// Validated Ollama API base URL (e.g. "http://localhost:11434").
    /// Kept for backward compatibility with OLLAMA_URL validation and seed.rs.
    #[allow(dead_code)]
    pub ollama_url: String,
    /// Ollama model name for chat inference (e.g. "qwen2.5:3b").
    /// Kept for backward compatibility; chat now uses llm_client.model.
    #[allow(dead_code)]
    pub ollama_model: String,
    /// Directory where RAG artifacts (chunked knowledge .zip) are written.
    #[allow(dead_code)]
    pub rag_chunk_dir: std::path::PathBuf,
    /// Absolute canonical path to the pre-built knowledge artifact .zip file.
    /// Set by seed::run() at startup; used by the /download-knowledge handler.
    pub artifact_zip_path: std::sync::Mutex<Option<std::path::PathBuf>>,
    /// Universal LLM client for chat inference (replaces direct Ollama calls).
    pub llm_client: LlmClient,
    /// Shared reqwest client for Ollama HTTP calls (connection pooling,
    /// redirect Policy::none() for SSRF prevention -- Decision 8).
    pub ollama_client: reqwest::Client,
    /// Per-IP rate limiter for the /chat endpoint (10 req/min).
    pub chat_limiter: governor::RateLimiter<
        String,
        governor::state::keyed::DashMapStateStore<String>,
        governor::clock::DefaultClock,
        governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    >,

    /// Browser-mediated signing — unsigned bundles parked between
    /// `mnemonic_sign_memory` (HTTP path) and `POST /api/sign-callback`.
    /// LRU-bounded (10k), TTL-bounded (300s), per-`jwt.sub` capped (50).
    /// See `pending.rs` for the Decision-12 design.
    pub pending: Arc<PendingBundles>,

    /// CLI bootstrap-ticket store (mnemonic-cli tech-spec Decision 7).
    /// Webapp issues a ticket via `POST /api/cli-bootstrap/issue` (Bearer
    /// JWT'd); CLI redeems with `GET /api/cli-bootstrap/redeem/:ticket`
    /// (UUID is the capability — no auth header required). Tickets are
    /// in-memory only; server restart drops every pending ticket.
    /// LRU 100, TTL 600s, per-user cap 3. See `api.rs` for the design.
    pub bootstrap_tickets: Arc<BootstrapTickets>,

    /// Static x25519 keypair for the CLI bootstrap symmetric flow (Task 12).
    /// Generated once at process boot via `SecretKey::generate(&mut OsRng)`.
    /// Process-lifetime only — restarting the server invalidates all in-flight
    /// CLI-origin tickets (acceptable given the 5-min TTL). Exposed via
    /// `GET /api/cli-bootstrap/server-pub` so CLIs can wrap their secrets.
    pub bootstrap_server_x25519_secret: crypto_box::SecretKey,
    pub bootstrap_server_x25519_public: crypto_box::PublicKey,

    /// `whoami` discoverability envelope — populated once at process start
    /// from `Config` (storage_mode + payment_mode + initial pricing
    /// snapshot). See `Envelope::from_config`. Threaded into
    /// `tools::whoami` for the new envelope-output contract AND into
    /// `tools::sign_memory` so the `participate`-on-local-only rejection
    /// path can return `unsupported_mode("participate", &supported)`
    /// without re-deriving the list. Decision 3 in
    /// work/modes-user-choice/tech-spec.md.
    pub envelope: Envelope,
}

// Safety: We only access store through std::sync::Mutex (short critical sections, no await)
// Keypair is just bytes, SolanaClient/ArweaveClient are reqwest::Client (Send+Sync)
unsafe impl Send for McpState {}
unsafe impl Sync for McpState {}

fn tool_definitions() -> Value {
    serde_json::json!([
        {
            "name": "mnemonic_whoami",
            "description": "Returns this agent's cryptographic identity: Solana public key, did:sol, did:key, attestation count",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "mnemonic_sign_memory",
            "description": "Creates a verifiable memory attestation: canonical CBOR + blake3 hash, signed with COSE_Sign1 (Ed25519), stored on Arweave, hash anchored as SPL Memo on Solana",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "Content to attest"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Optional tags"},
                },
                "required": ["content"],
            },
        },
        {
            "name": "mnemonic_verify",
            "description": "Verifies a memory attestation by recomputing hash and comparing against on-chain record",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "solana_tx": {"type": "string", "description": "Solana TX signature"},
                    "arweave_tx": {"type": "string", "description": "Arweave TX ID"},
                },
            },
        },
        {
            "name": "mnemonic_prove_identity",
            "description": "Signs a challenge with Ed25519 key, proving identity without on-chain transaction",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "challenge": {"type": "string", "description": "Challenge to sign"},
                },
                "required": ["challenge"],
            },
        },
        {
            "name": "mnemonic_recall",
            "description": "Searches attested memory history using semantic similarity",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "limit": {"type": "integer", "description": "Max results", "default": 5},
                },
                "required": ["query"],
            },
        },
        {
            "name": "mnemonic_check_pending",
            "description": "Resolves a deferred-sign correlation_id to its on-chain state. Use this AFTER mnemonic_sign_memory returns awaiting_signature and the user has approved in the browser. Returns {status: 'signed', solana_tx, arweave_tx, solana_explorer_url, arweave_url, attestation_id, ...} on success, {status: 'awaiting_signature'} if user has not approved yet, or {status: 'not_found'} if expired.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "correlation_id": {"type": "string", "description": "The correlation_id returned by mnemonic_sign_memory's awaiting_signature response"},
                },
                "required": ["correlation_id"],
            },
        },
    ])
}

pub async fn handle_request(
    req: &JsonRpcRequest,
    state: &McpState,
    owner_pubkey: &str,
    jwt_sub: Option<&str>,
) -> JsonRpcResponse {
    let result: Result<Value, JsonRpcError> = match req.method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mnemonic", "version": "0.1.0"},
        })),
        "tools/list" => Ok(serde_json::json!({"tools": tool_definitions()})),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or_default();
            handle_tool_call(name, &args, state, owner_pubkey, jwt_sub).await
        }
        "notifications/initialized" | "ping" => Ok(serde_json::json!({})),
        _ => Err(JsonRpcError::simple(
            -32603,
            format!("unknown method: {}", req.method),
        )),
    };

    // For notifications (no `id`), JSON-RPC 2.0 forbids a response. Callers
    // must check `req.is_notification()` before using this function's return
    // value — `mcp_handler` returns 204 No Content for notifications and never
    // serializes this response. We still construct one so the call shape is
    // uniform (avoids dual return types). Use `Value::Null` as a placeholder
    // when `id` is absent.
    let response_id = req.id.clone().unwrap_or(Value::Null);

    match result {
        Ok(val) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: response_id,
            result: Some(val),
            error: None,
        },
        Err(err) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: response_id,
            result: None,
            error: Some(err),
        },
    }
}

// ── Streamable HTTP transport (MCP spec 2025) ────────────────────────────────
//
// Per Decision 1 the `/mcp` endpoint serves chunked NDJSON: `Content-Type:
// application/x-ndjson`, no `Content-Length` (axum auto-sets
// `Transfer-Encoding: chunked` when the response body is a stream), one JSON
// frame per newline. Today we emit exactly one frame per inbound request —
// multi-frame support (progress notifications, async tool results from Task 4b
// PendingBundles) is wired but unused.

const JSON_CONTENT_TYPE: &str = "application/json";

/// Build an MCP streamable-HTTP response. Per MCP spec 2025-06-18, a single
/// JSON-RPC response uses `Content-Type: application/json` with the response
/// body being one JSON envelope (NOT NDJSON, NOT SSE — those formats are for
/// multi-frame streaming responses, which we do not currently emit).
///
/// Cursor / Claude.ai / VS Code MCP clients reject `application/x-ndjson`
/// (which the spec does not define for the single-response case) with
/// "Unexpected content type" — observed during T15 post-deploy QA.
///
/// We keep the body as a `Body::from_stream` of one `Bytes` chunk so axum
/// emits it as chunked transfer-encoding without a `Content-Length` header.
/// This is compatible with `application/json` (clients parse the body as a
/// single JSON value regardless of transfer-encoding). When we eventually
/// emit progress-notification frames the response shape will switch to
/// `Content-Type: text/event-stream` (SSE) per the same spec.
fn ndjson_response<T: Serialize>(status: StatusCode, frame: &T) -> Response {
    let body_str = match serde_json::to_string(frame) {
        Ok(s) => s,
        Err(e) => {
            // Last-ditch fallback — produce a JSON-RPC parse error envelope.
            // Logged because reaching here means our own response type failed
            // to serialize, which is a programmer error, not a client one.
            tracing::error!(error = %e, "failed to serialize JSON-RPC frame");
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal serialize error\"}}"
                .to_string()
        }
    };

    let body = Body::from_stream(stream::once(async move {
        Ok::<Bytes, Infallible>(Bytes::from(body_str))
    }));

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static(JSON_CONTENT_TYPE),
    );
    resp
}

/// Build a non-streaming JSON error response for cases where the *request*
/// itself was malformed (e.g. JSON parse error before we even know the
/// JSON-RPC id). Emitted as one `application/json` envelope per MCP spec.
fn ndjson_error(status: StatusCode, code: i32, message: &str) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": code, "message": message},
    });
    ndjson_response(status, &body)
}

/// Streamable-HTTP `/mcp` handler. Returns chunked NDJSON; one frame today,
/// extensible to many (progress notifications, deferred sign-callback frames
/// from Task 4b).
///
/// Payment-gating semantics from the previous request-response handler are
/// preserved: when `mnemonic_sign_memory` is invoked under
/// `payment_mode != "none" && storage_mode != "local"`, we run the full
/// `payment::check_payment` -> `deduct_balance` -> dispatch -> refund flow.
/// Each terminal state emits exactly one NDJSON frame.
pub async fn mcp_handler(
    State(state): State<Arc<McpState>>,
    headers: HeaderMap,
    request: axum::http::Request<axum::body::Body>,
) -> Response {
    // Pull the JWT-resolved Claims out of request extensions (set by
    // `oauth::bearer_auth_middleware` on success). Allowlisted methods
    // (`initialize`, `tools/list`) reach this handler without Claims —
    // those paths never touch storage so the fallback below is safe.
    let claims = request.extensions().get::<crate::oauth::Claims>().cloned();

    // Buffer the body — middleware already consumed and re-injected once;
    // a second consumption is fine.
    let body_bytes = match axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return ndjson_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                -32700,
                &format!("body read failed: {e}"),
            );
        }
    };
    let body = body_bytes;

    // Parse the JSON-RPC envelope manually so we control the error shape (we
    // need to emit a single NDJSON frame on parse failure, not the default
    // axum `Json` rejection HTML).
    let req: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return ndjson_error(
                StatusCode::BAD_REQUEST,
                -32700,
                &format!("parse error: {e}"),
            );
        }
    };

    // JSON-RPC 2.0 spec: notifications (no `id`) MUST NOT receive a response.
    // MCP streamable-HTTP spec (2025-06-18 §2.4) further specifies:
    //   "If the input consists solely of (any number of) JSON-RPC responses
    //    or notifications, the server MUST return HTTP status code 202
    //    Accepted with no body."
    // VS Code's MCP client logs `Unexpected 204 response` when we return 204
    // — switch to 202 for spec compliance. Cursor accepts both, no harm.
    if req.is_notification() {
        return axum::http::Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(axum::body::Body::empty())
            .expect("static response builds");
    }

    // Resolve owner_pubkey:
    //   - If JWT-authenticated (Decision 9): use `claims.sub`.
    //   - Otherwise (allowlisted methods like `tools/list`): fall back to
    //     the local server keypair so legacy code paths in tools.rs do not
    //     blow up. `tools/list` and `initialize` never touch storage so the
    //     value is unused on those paths.
    let owner_pubkey: String = match &claims {
        Some(c) => c.sub.clone(),
        None => mnemonic_core::identity::pubkey_base58(&state.keypair),
    };
    // Decision 12: HTTP/JWT presence is the trigger for the deferred-signing
    // branch in `tools::sign_memory`. Stdio path always passes `None` here.
    let jwt_sub: Option<String> = claims.map(|c| c.sub);

    let is_sign_memory = req.method == "tools/call"
        && req.params.get("name").and_then(|n| n.as_str()) == Some("mnemonic_sign_memory");

    // T2: resolve the per-request `mode` field BEFORE the paywall gate so
    // that (a) malformed `mode` values short-circuit with `-32602
    // InvalidParams` instead of charging, (b) `WriteMode::Local` requests
    // ALWAYS skip the paywall regardless of `STORAGE_MODE`, (c) the
    // resolved `WriteMode` is the single value driving both the paywall
    // decision and the column persisted by `sign_memory_inline` (drift
    // impossible by construction — Decision 1).
    //
    // For non-sign_memory requests, this resolution is a no-op (we never
    // touch the value); we still tunnel it through `handle_tool_call`
    // when needed so the same parsing happens exactly once.
    let resolved_mode_for_gate: Option<WriteMode> = if is_sign_memory {
        let args = req.params.get("arguments").cloned().unwrap_or_default();
        match crate::tools::resolve_write_mode(args.get("mode"), &state.storage_mode) {
            Ok(m) => Some(m),
            Err(err) => {
                // Bad `mode` field — short-circuit with the typed error.
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id.clone().unwrap_or(Value::Null),
                    result: None,
                    error: Some(err),
                };
                return ndjson_response(StatusCode::BAD_REQUEST, &resp);
            }
        }
    } else {
        None
    };

    // Paywall fires only on resolved `Participate` + a paid deploy. A
    // `Local` write on a `STORAGE_MODE=full + PAYMENT_MODE=x402` server
    // bypasses the gate entirely — the whitepaper §5.7.1 free-local
    // invariant is now structural, not configurational.
    let participate_gate = matches!(resolved_mode_for_gate, Some(WriteMode::Participate));
    if is_sign_memory && participate_gate && state.payment_mode != "none" {
        // Use live price from pricing engine (refreshed in background).
        let current_cost = state.pricing.current_price();
        let gate = payment::check_payment(
            &headers,
            &state.payment_mode,
            &state.store,
            &state.solana,
            &state.treasury_pubkey,
            &state.usdc_mint,
            current_cost,
        )
        .await;

        match gate {
            payment::PaymentGate::Proceed(api_key) => {
                // Deduct balance BEFORE executing the tool (reserve funds).
                if let Some(ref key) = api_key {
                    let store = state.store.lock().expect("store mutex poisoned");
                    if let Err(e) = payment::deduct_balance(
                        &store,
                        key,
                        state.sign_memory_cost_micro_usdc,
                        "mnemonic_sign_memory",
                    ) {
                        let err_body = serde_json::json!({
                            "jsonrpc": "2.0", "id": req.id,
                            "error": {"code": -32600, "message": format!("payment failed: {e}")}
                        });
                        return ndjson_response(StatusCode::PAYMENT_REQUIRED, &err_body);
                    }
                }

                let resp = handle_request(&req, &state, &owner_pubkey, jwt_sub.as_deref()).await;

                // Refund on tool failure. Uses `refund_balance` (not
                // `credit_deposit`) so the per-tx_sig idempotency guard does
                // not silently swallow repeated refunds for the same
                // underlying failure class.
                if resp.error.is_some() {
                    if let Some(ref key) = api_key {
                        let reason = resp
                            .error
                            .as_ref()
                            .map(|e| e.message.as_str())
                            .unwrap_or("error");
                        let store = state.store.lock().expect("store mutex poisoned");
                        if let Err(e) = payment::refund_balance(&store, key, current_cost, reason) {
                            tracing::warn!(api_key = %key, error = %e, "refund failed");
                        }
                    }
                }

                ndjson_response(StatusCode::OK, &resp)
            }
            payment::PaymentGate::NeedPayment(x402) => {
                ndjson_response(StatusCode::PAYMENT_REQUIRED, &x402)
            }
            payment::PaymentGate::Unauthorized(msg) => {
                let err_body = serde_json::json!({
                    "jsonrpc": "2.0", "id": req.id,
                    "error": {"code": -32600, "message": msg}
                });
                ndjson_response(StatusCode::UNAUTHORIZED, &err_body)
            }
        }
    } else {
        let resp = handle_request(&req, &state, &owner_pubkey, jwt_sub.as_deref()).await;
        ndjson_response(StatusCode::OK, &resp)
    }
}

// Bearer-auth middleware lives in `oauth.rs::bearer_auth_middleware`. The
// `bearer_auth_layer` scaffolding from Task 1 has been removed as part of
// Task 4 — there is no longer a "no-op" path. `main.rs::run_http` wires
// `oauth::bearer_auth_middleware` with the OAuthState directly.

async fn handle_tool_call(
    name: &str,
    args: &Value,
    state: &McpState,
    owner_pubkey: &str,
    jwt_sub: Option<&str>,
) -> Result<Value, JsonRpcError> {
    let result = match name {
        "mnemonic_whoami" => {
            // DB-only: lock, query, release before returning
            let store = state.store.lock().unwrap();
            tools::whoami(&state.keypair, &store, &state.storage_mode, &state.envelope)
        }
        "mnemonic_sign_memory" => {
            let content = args["content"]
                .as_str()
                .ok_or_else(|| JsonRpcError::simple(-32603, "content required"))?
                .to_string();
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|t| t.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            // T2: resolve the per-request `mode` field into a typed
            // `WriteMode`. Errors here (`-32602 InvalidParams`) propagate
            // out as typed JSON-RPC errors with `data.field == "mode"`
            // and `data.received` echoing the raw input. The paywall
            // gate in `mcp_handler` resolves the same value (single
            // source of truth — drift impossible by construction).
            let write_mode = tools::resolve_write_mode(args.get("mode"), &state.storage_mode)?;
            let cost_hint = state.pricing.cost_hint(state.sol_tx_fee_lamports);
            tools::sign_memory(
                &state.keypair,
                &state.solana,
                &state.arweave,
                &state.store,
                state.embedder.as_ref(),
                &state.compressor,
                &state.pending,
                &content,
                &tags,
                &cost_hint,
                &state.storage_mode,
                owner_pubkey,
                jwt_sub,
                write_mode,
                &state.envelope,
            )
            .await
            .map_err(tool_error_to_json_rpc)?
        }
        "mnemonic_verify" => {
            let sol = args.get("solana_tx").and_then(|v| v.as_str());
            let ar = args.get("arweave_tx").and_then(|v| v.as_str());
            tools::verify(
                &state.solana,
                &state.arweave,
                &state.store,
                sol,
                ar,
                &state.storage_mode,
            )
            .await
            .map_err(|e| JsonRpcError::simple(-32603, e.to_string()))?
        }
        "mnemonic_prove_identity" => {
            // Pure crypto, no DB or network
            tools::prove_identity(
                &state.keypair,
                args["challenge"]
                    .as_str()
                    .ok_or_else(|| JsonRpcError::simple(-32603, "challenge required"))?,
            )
        }
        "mnemonic_recall" => {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| JsonRpcError::simple(-32603, "query required"))?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            // DB-only: lock, query, release
            let store = state.store.lock().unwrap();
            tools::recall(
                &state.keypair,
                &store,
                state.embedder.as_ref(),
                query,
                limit,
                owner_pubkey,
            )
        }
        "mnemonic_check_pending" => {
            let cid = args["correlation_id"]
                .as_str()
                .ok_or_else(|| JsonRpcError::simple(-32603, "correlation_id required"))?
                .to_string();
            tools::check_pending(&state.pending, &state.store, &cid).await
        }
        _ => {
            return Err(JsonRpcError::simple(
                -32603,
                format!("unknown tool: {name}"),
            ))
        }
    };

    Ok(serde_json::json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default()}]
    }))
}

/// Translate an `anyhow::Error` coming out of `tools::sign_memory` into a
/// `JsonRpcError`. `sign_memory_inline` smuggles typed JSON-RPC errors (e.g.
/// `-32010 UnsupportedMode`) through `anyhow` as JSON-stringified bodies;
/// every other error stays a generic `-32603 InternalError` with the
/// stringified `Display`.
///
/// The smuggle format is exactly the inner-data shape of `JsonRpcError`:
/// `{"code": i32, "message": String, "data": Option<Value>}`. If parsing
/// succeeds, the original typed error is reconstituted; otherwise we fall
/// back to the generic envelope. This keeps the tool-impl side honest
/// (it doesn't need to know about JSON-RPC) while still letting typed
/// errors reach the wire.
fn tool_error_to_json_rpc(e: anyhow::Error) -> JsonRpcError {
    let msg = e.to_string();
    if let Ok(parsed) = serde_json::from_str::<Value>(&msg) {
        if let (Some(code), Some(message)) = (
            parsed.get("code").and_then(|c| c.as_i64()),
            parsed.get("message").and_then(|m| m.as_str()),
        ) {
            return JsonRpcError {
                code: code as i32,
                message: message.to_string(),
                data: parsed.get("data").cloned().filter(|v| !v.is_null()),
            };
        }
    }
    JsonRpcError::simple(-32603, msg)
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Per Decision 1 + Task 1 acceptance criteria: the streamable-HTTP transport
// must emit chunked NDJSON, survive client disconnect mid-stream without
// panicking, and have the bearer-auth middleware *registered* on `/mcp`
// today (Task 4a flips its body from no-op to JWT validation, hence the
// `#[ignore]` on the auth test which is wired now so Task 4a only has to
// flip the ignore + assertion).

#[cfg(test)]
mod transport_tests {
    use super::*;
    use axum::{http::Request, middleware as axum_middleware, routing::post, Router};
    use http_body_util::BodyExt;
    use mnemonic_core::embed::Embedder;
    use mnemonic_core::storage::AttestationStore;
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// Minimal embedder for transport tests. Returns zero vectors — the
    /// transport tests never hit the embed path (we only call `tools/list`
    /// and similar pure-RPC methods).
    struct StubEmbedder;
    impl Embedder for StubEmbedder {
        fn embed(&self, _text: &str) -> Vec<f32> {
            vec![0.0; 8]
        }
        fn dim(&self) -> usize {
            8
        }
        fn provider_name(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub-zero"
        }
    }

    /// Build a minimal `McpState` for transport tests. Storage is an
    /// in-memory SQLite (tempfile would also work; in-memory is faster and
    /// has no on-disk side effects). No external services are dialed.
    fn build_test_state() -> Arc<McpState> {
        use governor::Quota;
        use std::num::NonZeroU32;

        let tmp = tempfile::NamedTempFile::new().expect("create tmp file");
        let store = SqliteStore::open(tmp.path()).expect("open sqlite store");
        let compressor = EmbeddingCompressor::new(8, 4, 42);
        let quota = Quota::per_minute(NonZeroU32::new(10).expect("nonzero quota"));
        let chat_limiter = governor::RateLimiter::keyed(quota);
        let ollama_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build reqwest client");
        let llm_client =
            crate::llm::LlmClient::new("ollama", "", "test-model", "http://localhost:0", 512)
                .expect("build llm client");

        let bootstrap_server_x25519_secret =
            crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let bootstrap_server_x25519_public = bootstrap_server_x25519_secret.public_key();

        Arc::new(McpState {
            keypair: solana_sdk::signature::Keypair::new(),
            solana: SolanaClient::new("http://localhost:0"),
            arweave: ArweaveClient::new("http://localhost:0"),
            store: std::sync::Mutex::new(store),
            embedder: Box::new(StubEmbedder),
            compressor,
            payment_mode: "none".into(),
            treasury_pubkey: String::new(),
            usdc_mint: String::new(),
            sign_memory_cost_micro_usdc: 0,
            pricing: crate::pricing::PricingEngine::new(0),
            sol_tx_fee_lamports: 0,
            storage_mode: "local".into(),
            ollama_url: "http://localhost:0".into(),
            ollama_model: "test-model".into(),
            rag_chunk_dir: PathBuf::from("/tmp"),
            llm_client,
            artifact_zip_path: std::sync::Mutex::new(None),
            ollama_client,
            chat_limiter,
            pending: Arc::new(crate::pending::PendingBundles::with_defaults()),
            bootstrap_tickets: Arc::new(crate::api::BootstrapTickets::with_defaults()),
            bootstrap_server_x25519_secret,
            bootstrap_server_x25519_public,
            envelope: Envelope::from_config("local", "none", 0),
        })
    }

    /// 32-byte test secret for OAuth JWT verification. Matches the
    /// production length requirement (Decision 11).
    const TEST_JWT_SECRET: &[u8; 32] = b"unit-test-secret-32-bytes-long!!";

    /// Build a `Router` with the `/mcp` route plus the bearer-auth middleware
    /// (Task 4 — `oauth::bearer_auth_middleware`). Mirrors the production
    /// wiring in `main.rs::run_http`. The middleware allows JSON-RPC
    /// `initialize` and `tools/list` without a JWT (per Decision 9) so the
    /// existing `test_chunked_response_encoding` test keeps passing.
    fn build_test_router(state: Arc<McpState>) -> Router {
        let oauth_state = Arc::new(crate::oauth::OAuthState::new(TEST_JWT_SECRET));
        let mcp_route = post(mcp_handler).layer(axum_middleware::from_fn_with_state(
            oauth_state,
            crate::oauth::bearer_auth_middleware,
        ));
        Router::new().route("/mcp", mcp_route).with_state(state)
    }

    /// Drives `Body::collect()` and returns the full bytes. Helper because
    /// `BodyExt::collect().await.unwrap().to_bytes()` is verbose.
    async fn collect_body(resp: Response) -> (StatusCode, axum::http::HeaderMap, Bytes) {
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("body collect failed")
            .to_bytes();
        (status, headers, bytes)
    }

    /// Drives: streamable-HTTP refactor + header setup. Asserts:
    /// (a) `content-type: application/json` — per MCP spec 2025-06-18 a
    ///     single JSON-RPC response uses `application/json`, NOT
    ///     `application/x-ndjson` (the latter was rejected by Cursor /
    ///     Claude.ai / VS Code with "Unexpected content type" during T15
    ///     post-deploy QA; see `ndjson_response` doc comment),
    /// (b) no `content-length` header (axum auto-emits chunked transfer-
    ///     encoding when the body is a stream without a known length),
    /// (c) body is exactly one JSON envelope that round-trips to the
    ///     `tools/list` shape with the 7 expected tools.
    #[tokio::test]
    async fn test_chunked_response_encoding() {
        let state = build_test_state();
        let app = build_test_router(state);

        let req_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 1,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&req_body).expect("serialize req"),
            ))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("oneshot");
        let (status, headers, body) = collect_body(resp).await;

        assert_eq!(status, StatusCode::OK, "tools/list must return 200");
        assert_eq!(
            headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
            "application/json",
            "single JSON-RPC response must use application/json per MCP spec 2025-06-18",
        );
        assert!(
            headers.get("content-length").is_none(),
            "chunked response must not advertise Content-Length (got {:?})",
            headers.get("content-length"),
        );

        let body_str = std::str::from_utf8(&body).expect("body utf-8");
        let envelope: Value = serde_json::from_str(body_str).expect("body is valid JSON");
        assert_eq!(envelope["jsonrpc"], "2.0");
        assert_eq!(envelope["id"], 1);
        let tools = envelope["result"]["tools"]
            .as_array()
            .expect("tools array present");
        assert_eq!(
            tools.len(),
            6,
            "expected 6 MCP tools in tools/list response (whoami, sign_memory, verify, prove_identity, recall, check_pending)",
        );
    }

    /// Drives: cancellation safety of `Body::from_stream`. Sends a request,
    /// collects the response, then drops the response without reading the
    /// body. Then sends a second request to prove the server is still
    /// healthy (no poisoned mutex, no panicked task). Today the body is a
    /// single `Bytes` chunk so cancellation is trivially safe; the test is
    /// the regression guard for when Task 4b adds multi-frame mpsc-backed
    /// streaming.
    #[tokio::test]
    async fn test_partial_response_client_disconnect() {
        let state = build_test_state();
        let app = build_test_router(state);

        let req_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 7,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&req_body).expect("serialize req"),
            ))
            .expect("build request");

        // Get the response, then drop it without consuming the body — this
        // simulates the client closing the TCP socket before reading any
        // chunks. Must not panic, must not poison the store mutex.
        let resp = app.clone().oneshot(req).await.expect("oneshot");
        let _status = resp.status();
        drop(resp);

        // Yield to give any spawned tasks a chance to observe the drop.
        tokio::task::yield_now().await;

        // Second request must still succeed — proves no global state corruption.
        let req2_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 8,
        });
        let req2 = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&req2_body).expect("serialize req"),
            ))
            .expect("build request");

        let resp2 = app.oneshot(req2).await.expect("second oneshot");
        let (status2, _headers2, body2) = collect_body(resp2).await;
        assert_eq!(
            status2,
            StatusCode::OK,
            "second request after dropped response must still succeed",
        );
        let line = std::str::from_utf8(&body2)
            .expect("body utf-8")
            .trim_end_matches('\n');
        let env: Value = serde_json::from_str(line).expect("frame valid JSON");
        assert_eq!(env["id"], 8);
    }

    /// Mint a HS256 JWT for tests using the module's `TEST_JWT_SECRET`.
    /// Wrap around `crate::oauth::issue_jwt` to keep the call shape in
    /// the TDD anchor (and any future test) short and explicit.
    fn mint_jwt_for_tests(sub: &str) -> String {
        let oauth_state = crate::oauth::OAuthState::new(TEST_JWT_SECRET);
        crate::oauth::issue_jwt(&oauth_state, sub).expect("issue_jwt")
    }

    /// TDD anchor for T2 (modes-user-choice). Drives end-to-end:
    /// `sign_memory { mode: "participate" }` against a local-only
    /// server (default `STORAGE_MODE=local` from `build_test_state`)
    /// returns the typed `-32010 UnsupportedMode` envelope with
    /// `data.supported == ["local"]` and writes ZERO rows. Same
    /// expectations as the integration test in
    /// `mcp/tests/modes_per_request.rs`, but inlined here against the
    /// existing in-module test plumbing so we have a fast unit-level
    /// regression guard inside the dispatcher's own test module.
    #[tokio::test]
    async fn participate_against_local_only_server_returns_unsupported_mode() {
        let state = build_test_state(); // STORAGE_MODE defaults to "local"
        let app = build_test_router(state.clone());

        // The owner pubkey must match jwt.sub for the OAuth middleware to
        // bind the request to a real Claims extension.
        let owner = mnemonic_core::identity::pubkey_base58(&state.keypair);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": "hi", "mode": "participate"},
            },
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header(
                "authorization",
                format!("Bearer {}", mint_jwt_for_tests(&owner)),
            )
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (_status, _hdrs, bytes) = collect_body(resp).await;
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let err = envelope["error"]
            .as_object()
            .expect("expected JSON-RPC error envelope");
        assert_eq!(err["code"], -32010, "code must be -32010 UnsupportedMode");
        assert_eq!(err["message"], "Unsupported mode");
        let data = err["data"]
            .as_object()
            .expect("typed error must carry `data`");
        assert_eq!(data["kind"], "UnsupportedMode");
        assert_eq!(data["requested"], "participate");
        assert_eq!(data["supported"], serde_json::json!(["local"]));

        // DB must be unchanged — no row written, no synthetic id minted.
        let store = state.store.lock().unwrap();
        // `count` is signer-scoped; pass empty string for "all signers".
        assert_eq!(store.count("").unwrap_or_default(), 0);
        // Also count under the test owner key directly to be extra-safe.
        assert_eq!(store.count(&owner).unwrap_or_default(), 0);
    }

    /// Companion to the TDD anchor: `invalid mode` value (uppercase
    /// `"Local"`) returns `-32602 InvalidParams` with `data.field == "mode"`
    /// and `data.received` echoing the raw input. Strict — no normalisation.
    #[tokio::test]
    async fn invalid_mode_string_returns_invalid_params() {
        let state = build_test_state();
        let app = build_test_router(state.clone());
        let owner = mnemonic_core::identity::pubkey_base58(&state.keypair);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "mnemonic_sign_memory",
                "arguments": {"content": "hi", "mode": "Local"},
            },
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header(
                "authorization",
                format!("Bearer {}", mint_jwt_for_tests(&owner)),
            )
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (_status, _hdrs, bytes) = collect_body(resp).await;
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let err = envelope["error"]
            .as_object()
            .expect("expected JSON-RPC error");
        assert_eq!(err["code"], -32602);
        let data = err["data"].as_object().expect("data");
        assert_eq!(data["field"], "mode");
        assert_eq!(data["received"], "Local");

        let store = state.store.lock().unwrap();
        assert_eq!(store.count(&owner).unwrap_or_default(), 0);
    }

    /// Active under Task 4 — `oauth::bearer_auth_middleware` rejects
    /// unauthenticated `tools/call` with HTTP 401 and a JSON-RPC error
    /// envelope (`code: -32001`). `initialize` and `tools/list` remain
    /// allowlisted (Decision 9).
    #[tokio::test]
    async fn test_missing_authorization_header_returns_401() {
        let state = build_test_state();
        let app = build_test_router(state);

        let req_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_recall", "arguments": {"query": "x"}},
            "id": 99,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            // NO Authorization header — Task 4a will reject this.
            .body(Body::from(
                serde_json::to_vec(&req_body).expect("serialize req"),
            ))
            .expect("build request");

        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Task 4a must reject /mcp tools/call without Bearer JWT",
        );
    }
}
