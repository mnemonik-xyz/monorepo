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

/// Build-time-generated skill manifest constants, projected from
/// `mcp/assets/skills/*.md` by `mcp/build.rs`. Three slots per skill —
/// `*_FULL_MARKDOWN`, `*_PURPOSE_PLUS_TRIGGER`, `*_PURPOSE_ONE_LINER` —
/// plus an `ALL_SKILLS: &[SkillManifest]` table. Consumed by the
/// `prompts/*`, `resources/*`, and enriched `tools/list` dispatch arms
/// below. See [`work/agent-native-distribution/tech-spec.md`] Decision 1
/// for the single-source-of-truth rationale.
pub mod skills {
    include!(concat!(env!("OUT_DIR"), "/skills_generated.rs"));
}

/// URI scheme for skill resources. `resources/list` advertises one URI
/// per skill manifest in the shape `mnemonik://skills/<name>.md`;
/// `resources/read` accepts the same shape and returns the verbatim
/// `FULL_MARKDOWN` slot. Namespaced to our protocol so a client that
/// mixes mnemonic resources with other servers' resources can route by
/// scheme.
const RESOURCE_URI_PREFIX: &str = "mnemonik://skills/";

/// Process-level embedder build identity surfaced through the MCP
/// `initialize` response (`result.embedder.model_version`). Format:
/// `<mcp-version>-<embedder-family>-<library-version>`. Today we ship
/// fastembed; a future swap to OpenAI would mint a new value such as
/// `"0.2.0-openai-text-embedding-3-small"`.
///
/// **Not** wired into `Embedder::model_id()` — the trait identifies the
/// MODEL (e.g. `"all-MiniLM-L6-v2"`); this constant identifies the
/// process build that drove the embedding pipeline. Both are useful to
/// callers who want to re-embed and compare.
///
/// **Process risk — manual sync required.** This is a plain string
/// literal (the `concat!` macro accepts only literal tokens, not const
/// expressions, so we cannot derive it from `CARGO_PKG_VERSION` plus
/// `fastembed::VERSION`). The Task 13 release checklist MUST include:
/// "verify `EMBEDDER_MODEL_VERSION` matches `mcp/Cargo.toml` `version`
/// and `cargo tree -p fastembed | head -1` output before tagging" so a
/// fastembed bump or mcp version bump never silently ships a stale
/// identifier. Defined here (not in `lib.rs`) because the binary's
/// `mod mcp;` and the library's `pub mod mcp;` both compile this file,
/// so a single canonical definition under `mcp` satisfies both
/// compilation units; `lib.rs` re-exports it for integration tests.
pub const EMBEDDER_MODEL_VERSION: &str = "0.1.0-fastembed-5.13.2";

/// Build the `prompts/list` response payload. One entry per skill
/// manifest: `name`, `description` (the `PURPOSE_ONE_LINER` slot). The
/// MCP spec leaves prompt arguments optional — we don't take any, so
/// the field is omitted (clients render the prompt as "ready-to-send"
/// markdown).
fn prompts_list_payload() -> Value {
    let prompts: Vec<Value> = skills::ALL_SKILLS
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.purpose_one_liner,
            })
        })
        .collect();
    serde_json::json!({ "prompts": prompts })
}

/// Build the `prompts/get` response for a named skill. Returns
/// `Err(invalid_params("name", received))` if `name` is missing or does
/// not match any built-in skill — MCP clients render `-32602` as a
/// distinct "unknown prompt" UX, so the error code is meaningful.
fn prompts_get_payload(params: &Value) -> Result<Value, JsonRpcError> {
    let raw_name = params.get("name");
    let name = match raw_name.and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return Err(invalid_params(
                "name",
                &raw_name.cloned().unwrap_or(Value::Null),
            ));
        }
    };
    let skill = skills::ALL_SKILLS.iter().find(|s| s.name == name);
    let skill = match skill {
        Some(s) => s,
        None => {
            return Err(invalid_params("name", &Value::String(name.to_string())));
        }
    };
    Ok(serde_json::json!({
        "description": skill.purpose_one_liner,
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": skill.full_markdown,
                },
            }
        ],
    }))
}

/// Build the `resources/list` response payload. One resource per skill
/// manifest: `uri` (`mnemonik://skills/<name>.md`), `name`,
/// `description` (one-liner), `mimeType: "text/markdown"`.
fn resources_list_payload() -> Value {
    let resources: Vec<Value> = skills::ALL_SKILLS
        .iter()
        .map(|s| {
            serde_json::json!({
                "uri": format!("{RESOURCE_URI_PREFIX}{name}.md", name = s.name),
                "name": s.name,
                "description": s.purpose_one_liner,
                "mimeType": "text/markdown",
            })
        })
        .collect();
    serde_json::json!({ "resources": resources })
}

/// Build the `resources/read` response. Validates the `uri` shape —
/// must start with `RESOURCE_URI_PREFIX` and end with `.md`, with a
/// skill stem that matches a registered manifest. Returns the verbatim
/// `FULL_MARKDOWN` slot (byte-identical to the source file under
/// `mcp/assets/skills/<name>.md`).
fn resources_read_payload(params: &Value) -> Result<Value, JsonRpcError> {
    let raw_uri = params.get("uri");
    let uri = match raw_uri.and_then(Value::as_str) {
        Some(u) => u,
        None => {
            return Err(invalid_params(
                "uri",
                &raw_uri.cloned().unwrap_or(Value::Null),
            ));
        }
    };
    let stem = uri
        .strip_prefix(RESOURCE_URI_PREFIX)
        .and_then(|rest| rest.strip_suffix(".md"));
    let skill = match stem {
        Some(s) => skills::ALL_SKILLS.iter().find(|m| m.name == s),
        None => None,
    };
    let skill = match skill {
        Some(s) => s,
        None => {
            return Err(invalid_params("uri", &Value::String(uri.to_string())));
        }
    };
    Ok(serde_json::json!({
        "contents": [
            {
                "uri": uri,
                "mimeType": "text/markdown",
                "text": skill.full_markdown,
            }
        ],
    }))
}

/// Find the skill manifest that matches a given MCP tool name. The six
/// public tools map 1:1 to the six user-facing skills (the seventh
/// skill — `mnemonik-help` — is a meta-skill with no underlying tool);
/// `mnemonic_check_pending` is also mapped to `mnemonik-attest` because
/// it is the deferred-result polling half of the attest flow.
fn skill_for_tool(tool: &str) -> Option<&'static skills::SkillManifest> {
    let target = match tool {
        "mnemonic_whoami" => "mnemonik-status",
        "mnemonic_sign_memory" => "mnemonik-attest",
        "mnemonic_check_pending" => "mnemonik-attest",
        "mnemonic_verify" => "mnemonik-verify",
        "mnemonic_recall" => "mnemonik-recall",
        "mnemonic_prove_identity" => "mnemonik-init",
        _ => return None,
    };
    skills::ALL_SKILLS.iter().find(|s| s.name == target)
}

/// Append the matching skill manifest's `Purpose+Trigger` section to a
/// tool's base description. Keeps `tool_definitions()` declarative;
/// the enrichment is impossible to forget because `enriched_tools()`
/// runs the lookup for every entry.
fn enrich_tool_description(tool: &Value) -> Value {
    let mut out = tool.clone();
    let Some(name) = tool.get("name").and_then(Value::as_str) else {
        return out;
    };
    let Some(skill) = skill_for_tool(name) else {
        return out;
    };
    let base = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let enriched = format!("{base}\n\n{}", skill.purpose_plus_trigger);
    if let Some(obj) = out.as_object_mut() {
        obj.insert("description".to_string(), Value::String(enriched));
    }
    out
}

/// Enriched `tools/list` payload — each base entry from
/// `tool_definitions()` has the matching skill manifest's Purpose +
/// Trigger appended to its `description`. Drift between manifest and
/// tools/list is now physically impossible because the manifest body is
/// the single source of truth for that copy.
///
/// Cached: both `tool_definitions()` and the skill manifests are
/// `'static` / deterministic, so the enriched vector is computed once
/// per process and shared by reference (tech-spec implementation hint;
/// code-reviewer round 1 CR2-03). `tools/list` no longer allocates a
/// fresh `Vec<Value>` per request — the payload-construction site in
/// `handle_request_with_resolved_mode` clones from the cached slice.
fn enriched_tools() -> &'static [Value] {
    static CACHE: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        tool_definitions()
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|t| enrich_tool_description(&t))
            .collect()
    })
}

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

/// `-32011 DeliveryNotConfirmed` — the participate write hit Arweave + Solana
/// but the post-anchor recall+verify round-trip failed at `stage`. The
/// attestation row was persisted as `local` (so the embed/signature aren't
/// wasted), the reserved payment was refunded, no `attestation_costs` row
/// was written. The client may either accept the local-only persistence or
/// retry; on x402 retries the same payment header is still good (the nonce
/// stays unconsumed). T3 — modes-user-choice.
///
/// `data` shape: `{kind, arweave_tx, solana_tx, stage, row_demoted_to,
/// attestation_id}`.
pub fn delivery_not_confirmed(
    stage: &str,
    arweave_tx: &str,
    solana_tx: &str,
    attestation_id: &str,
) -> JsonRpcError {
    JsonRpcError {
        code: -32011,
        message: "Delivery not confirmed".to_string(),
        data: Some(serde_json::json!({
            "kind": "DeliveryNotConfirmed",
            "arweave_tx": arweave_tx,
            "solana_tx": solana_tx,
            "stage": stage,
            "row_demoted_to": "local",
            "attestation_id": attestation_id,
        })),
    }
}

/// Derive the per-request quota subject for the DoS guard. Returns the
/// `blake3` hex digest of either the bearer api_key (balance mode) or the
/// x402 tx_sig (x402 mode). `None` on stdio path (no auth headers) and on
/// `payment_mode == "none"` (no billable subject).
///
/// Centralised so the subject derivation is the same value at the entry
/// quota check, the success-path nonce-consumption call (via the matching
/// `tx_sig`), and the failure-branch counter increment. A divergence
/// between any two of those would let an attacker bypass the quota by
/// rotating the part of the request the failure branch doesn't see.
///
/// **Subject choice rationale** (round-2 security-auditor fix):
/// - Balance: `blake3(api_key).to_hex()` — same as round 1. Operator-issued.
/// - x402: `blake3(tx_sig).to_hex()` — stable across retries with the
///   same `X-Payment` header. A fresh tx_sig means a fresh USDC payment;
///   the caller pays their own way around the quota.
/// - `Both` mode: prefer balance if a Bearer header is present, otherwise
///   fall back to x402 — matches the order `check_payment` uses.
pub(crate) fn derive_quota_subject(headers: &HeaderMap, payment_mode: &str) -> Option<String> {
    if payment_mode == "none" {
        return None;
    }
    // Balance has precedence in `both` mode — matches `check_payment`'s order.
    if matches!(payment_mode, "balance" | "both") {
        if let Some(raw_key) = payment::extract_api_key(headers) {
            return Some(payment::hash_api_key(&raw_key));
        }
    }
    if matches!(payment_mode, "x402" | "both") {
        if let Some(proof) = payment::extract_x402_proof(headers) {
            return Some(payment::hash_api_key(&proof.tx_sig));
        }
    }
    None
}

/// `-32099 TokenExpired` — the cached OAuth JWT at `~/.mnemonic/token.json`
/// has an `expires_at` in the past. The Rust binary surfaces this when it
/// reads the file via [`mnemonic_core::identity::token_store::read_token`]
/// for an outbound authenticated call. The agent client re-initiates the
/// OAuth loopback to refresh the token; the failure is recoverable without
/// user intervention beyond clicking through the consent page again.
///
/// `data` shape: `{kind: "TokenExpired", expires_at, pubkey}`.
///
/// Wired into the soft-fall proxy path by Task 5 round-3 (SAR5-INFO3 —
/// security-audit round 1): `tools::proxy_participate` maps
/// `TokenStoreError::Expired` from `mnemonic_core::identity::read_token`
/// to this typed JSON-RPC error so the agent sees the canonical `-32099
/// TokenExpired` code from the AC16 error catalogue instead of the
/// hosted side's `-32001 unauthorized` rebuke.
pub fn token_expired(expires_at: &str, pubkey: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32099,
        message: "Token expired".to_string(),
        data: Some(serde_json::json!({
            "kind": "TokenExpired",
            "expires_at": expires_at,
            "pubkey": pubkey,
        })),
    }
}

/// `-32011 DeliveryQuotaExceeded` — entry-of-participate-path short-circuit
/// fired by `mcp_handler` BEFORE any Arweave/Solana write because the
/// caller's `api_key_hash` has accumulated `>= threshold` delivery-failure
/// demotions inside the sliding window. Spends zero chain fees. The error
/// shares the `-32011` code with `DeliveryNotConfirmed`; clients
/// discriminate via `data.kind`. T3 — modes-user-choice.
///
/// `data` shape: `{kind, window_secs, threshold}`.
pub fn delivery_quota_exceeded(window_secs: u64, threshold: u32) -> JsonRpcError {
    JsonRpcError {
        code: -32011,
        message: "Delivery quota exceeded".to_string(),
        data: Some(serde_json::json!({
            "kind": "DeliveryQuotaExceeded",
            "window_secs": window_secs,
            "threshold": threshold,
        })),
    }
}

// ── Typed JSON-RPC errors (Task 4 — agent-native-distribution) ──────────────
//
// New entries from the Error Catalogue table in
// `work/agent-native-distribution/tech-spec.md` (Decision 4 + Decision 5b +
// soft-fall routing). Each helper documents the trigger condition, the
// `data.kind` discriminator, and every documented `data` field. A
// parametrized integration test in `mcp/tests/error_catalogue.rs` covers
// every row by triggering the condition via the production code path
// rather than hand-crafting a response.

/// `-32095 PublicWriteRequiresConfirmation` — `sign_memory` arrived with
/// `mode=participate + visibility=public` but the `public_write_confirmation`
/// field was missing, malformed, replayed, expired, cross-owner, or
/// content_hash-mismatched. The caller must rerun
/// `request_public_write_confirmation` to mint a fresh token and retry.
/// Decision 5b.
///
/// `data` shape: `{kind, content_hash, suggested_action}`.
pub fn public_write_requires_confirmation(content_hash: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32095,
        message: "Public-write confirmation required".to_string(),
        data: Some(serde_json::json!({
            "kind": "PublicWriteRequiresConfirmation",
            "content_hash": content_hash,
            "suggested_action":
                "Call request_public_write_confirmation, surface the content_hash to the user \
                 for in-turn approval, then retry sign_memory with the returned \
                 confirmation_token + jti within the 5-minute TTL.",
        })),
    }
}

/// `-32096 OAuthTimeout` — the OAuth-loopback browser flow exceeded the
/// per-call deadline (`MNEMONIC_OAUTH_TIMEOUT_SECS`, default 120s) without
/// the user finishing consent. Decision 4 + AC11. Production trigger lives
/// in Task 5 (`mcp-stdio` participate-mode); helper is defined here so the
/// Error Catalogue table has one canonical home.
///
/// `data` shape: `{kind, sign_url, expires_at, attempted_at}`.
#[allow(dead_code)]
pub fn oauth_timeout(sign_url: &str, expires_at: u64, attempted_at: u64) -> JsonRpcError {
    JsonRpcError {
        code: -32096,
        message: "OAuth loopback timed out".to_string(),
        data: Some(serde_json::json!({
            "kind": "OAuthTimeout",
            "sign_url": sign_url,
            "expires_at": expires_at,
            "attempted_at": attempted_at,
        })),
    }
}

/// `-32098 EmbedderInvalid` — the local embedder produced an unusable vector
/// (model file missing, corrupted, or ONNX runtime crashed). The `Embedder`
/// trait at `core/src/embed/mod.rs` is infallible by design; the production
/// code path in `sign_memory_inline` surfaces this typed error by treating
/// an empty `Vec::new()` return from `embed()` as the failure signal.
/// `fallback_available` advertises whether the request could be retried with
/// `allow_fallback_to_participate=true`.
///
/// `data` shape: `{kind, reason, repair_hint, fallback_available}`.
pub fn embedder_invalid(reason: &str, repair_hint: &str, fallback_available: bool) -> JsonRpcError {
    JsonRpcError {
        code: -32098,
        message: "Embedder invalid".to_string(),
        data: Some(serde_json::json!({
            "kind": "EmbedderInvalid",
            "reason": reason,
            "repair_hint": repair_hint,
            "fallback_available": fallback_available,
        })),
    }
}

/// `-32099 LocalStorageBusy` — SQLite returned `SQLITE_BUSY` after the
/// configured 5000ms internal busy-timeout (Decision 13). The agent should
/// retry after `retry_after_ms`. Shares the `-32099` code with
/// `TokenExpired`; clients discriminate via `data.kind`. Production trigger
/// lives in `core::storage::sqlite` busy-error mapping (T6 wire-up).
///
/// `data` shape: `{kind, retry_after_ms}`.
#[allow(dead_code)]
pub fn local_storage_busy(retry_after_ms: u64) -> JsonRpcError {
    JsonRpcError {
        code: -32099,
        message: "Local storage busy".to_string(),
        data: Some(serde_json::json!({
            "kind": "LocalStorageBusy",
            "retry_after_ms": retry_after_ms,
        })),
    }
}

/// `-32094 IdentityBootstrapFailed` — `core::identity::ensure()` returned an
/// error (no keychain access, no file fallback, all attempted paths failed).
/// Distinct from `-32099 TokenExpired`: the token is a JWT, the identity is
/// the Ed25519 signing key. Identity failure blocks every signed operation.
/// Production trigger lives in Task 5/6 keychain wire-up.
///
/// `data` shape: `{kind, reason, repair_hint}`.
#[allow(dead_code)]
pub fn identity_bootstrap_failed(reason: &str, repair_hint: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32094,
        message: "Identity bootstrap failed".to_string(),
        data: Some(serde_json::json!({
            "kind": "IdentityBootstrapFailed",
            "reason": reason,
            "repair_hint": repair_hint,
        })),
    }
}

/// `-32011 HostedUnavailable` — `mcp-stdio`'s participate-mode proxy could
/// not reach `MNEMONIC_HOSTED_ENDPOINT` (DNS, TCP, TLS, or 5xx-after-retry).
/// Shares `-32011` with `DeliveryNotConfirmed` and `DeliveryQuotaExceeded`;
/// clients discriminate via `data.kind`. Decision 4 (soft-fall) maps
/// post-escalation hosted unreachability to this code so the caller learns
/// the escalation failure, not the original local failure. Production
/// trigger lives in Task 5 (mcp-stdio soft-fall proxy).
///
/// `data` shape: `{kind, last_error, retry_after_ms}`.
#[allow(dead_code)]
pub fn hosted_unavailable(last_error: &str, retry_after_ms: u64) -> JsonRpcError {
    JsonRpcError {
        code: -32011,
        message: "Hosted endpoint unavailable".to_string(),
        data: Some(serde_json::json!({
            "kind": "HostedUnavailable",
            "last_error": last_error,
            "retry_after_ms": retry_after_ms,
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
    /// Admin bearer token gating operator-only endpoints (P&L). Empty = disabled.
    pub admin_token: String,
    /// EVM x402 settlement config (Wave 1). `None` = EVM rail disabled.
    pub evm_payment: Option<crate::payment::EvmPaymentConfig>,
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

    /// Wall-clock budget for the post-anchor Arweave re-fetch in the
    /// participate delivery-guarantee flow (T3). Used by
    /// `tools::sign_memory_inline` to bound the exponential-backoff retry
    /// loop. Operator-tunable via `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS`.
    pub delivery_refetch_timeout: std::time::Duration,

    /// Outcome-based per-`api_key_hash` quota counter (T3 — DoS guard).
    /// Consulted at the *entry* of the participate path in `mcp_handler`
    /// BEFORE any Arweave/Solana write; incremented in the failure branch
    /// of `sign_memory_inline` after a delivery demotion. Bounded by the
    /// background eviction task spawned in `main.rs::run_http`. Keyed on
    /// `api_key_hash` (blake3(api_key).to_hex()), NEVER `owner_pubkey`.
    pub refunds_by_subject: Arc<payment::RefundsBySubject>,

    /// Process-lifetime counters incremented by the delivery-guarantee
    /// flow. Stub for the eventual Prometheus surface — see
    /// `payment::DeliveryMetrics` for the four counters and the
    /// no-per-tenant-label rationale.
    pub delivery_metrics: Arc<payment::DeliveryMetrics>,

    /// In-process ledger for the public-write confirmation ceremony
    /// (Decision 5b — agent-native-distribution). `request_public_write_confirmation`
    /// mints an HMAC-bound token; `sign_memory` with
    /// `mode=participate + visibility=public` consumes it. The HMAC secret
    /// is regenerated at construction time and never persisted; a process
    /// restart invalidates every in-flight token (intentional graceful-
    /// degradation — the agent reruns the 3s ceremony).
    pub confirmation_ledger: Arc<crate::confirmation_token::ConfirmationLedger>,

    /// Resolved hosted MCP endpoint for the participate-mode soft-fall
    /// proxy on `mcp-stdio` (Decision 4 + Decision 12 —
    /// agent-native-distribution). Default
    /// [`crate::DEFAULT_HOSTED_ENDPOINT`] unless the operator passed
    /// `--allow-custom-endpoint` AND set `MNEMONIC_HOSTED_ENDPOINT`. Empty
    /// string is a sentinel for "no soft-fall available" (test fixtures
    /// that exercise the local code path without wiring an HTTPS client).
    pub hosted_endpoint: String,

    /// Shared HTTP client used by the soft-fall proxy to POST JSON-RPC to
    /// [`Self::hosted_endpoint`]. Pinned `Policy::none()` on redirects so
    /// a compromised hosted operator cannot 302 us to an unrelated host
    /// (the env-var redirection vector Decision 12 closes for the
    /// pre-connect side; this closes the mid-connection side).
    pub hosted_client: reqwest::Client,
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
                    "mode": {
                        "type": "string",
                        "enum": ["local", "participate"],
                        "description": "Per-request write intent (T2 — modes-user-choice). 'local' keeps the artifact on the server's own SQLite (free, no chain writes). 'participate' anchors on Arweave + Solana (paid on hosted operators; cost surfaced via mnemonic_whoami). Optional — omit to use the server's default; call mnemonic_whoami to see supported_modes / default_mode / participate_cost first.",
                    },
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
        {
            "name": "request_public_write_confirmation",
            "description": "Public-write ceremony gate: presents the content_hash about to be anchored on Arweave + Solana so the user can confirm or refuse in-turn before any chain write fires. Consumed by Task 4's handler; not user-facing — agent skills invoke it inline whenever they intend to issue a `mode='participate'` write with `visibility='public'`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content_hash": {"type": "string", "description": "blake3 hex of the canonical-CBOR bundle the caller is about to anchor"},
                },
                "required": ["content_hash"],
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
    // T2 round-2: callers without a pre-resolved mode (stdio dispatch via
    // `run_stdio` → `handle_request`) get `None` here. The dispatcher
    // resolves on demand inside `handle_tool_call`. `mcp_handler` (HTTP)
    // resolves up front for the paywall gate and passes the result in via
    // `handle_request_with_resolved_mode` below.
    handle_request_with_resolved_mode(req, state, owner_pubkey, jwt_sub, None).await
}

/// Variant of [`handle_request`] that accepts a pre-resolved `mode`. The
/// HTTP `mcp_handler` resolves `mode` once before the paywall gate (so
/// the gate decision and the storage column come from the same value);
/// it threads the resolved value here so `handle_tool_call` doesn't
/// re-parse the same input. The single call site eliminates the latent
/// drift risk the round-1 implementation carried.
pub async fn handle_request_with_resolved_mode(
    req: &JsonRpcRequest,
    state: &McpState,
    owner_pubkey: &str,
    jwt_sub: Option<&str>,
    pre_resolved_mode: Option<crate::tools::ResolvedMode>,
) -> JsonRpcResponse {
    let result: Result<Value, JsonRpcError> = match req.method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "tools": {},
                "prompts": {},
                "resources": {},
            },
            "serverInfo": {"name": "mnemonic", "version": "0.1.0"},
            "embedder": {
                "model_id": state.embedder.model_id(),
                "model_version": EMBEDDER_MODEL_VERSION,
                "dim": state.embedder.dim(),
            },
        })),
        "tools/list" => Ok(serde_json::json!({"tools": enriched_tools()})),
        "prompts/list" => Ok(prompts_list_payload()),
        "prompts/get" => prompts_get_payload(&req.params),
        "resources/list" => Ok(resources_list_payload()),
        "resources/read" => resources_read_payload(&req.params),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or_default();
            handle_tool_call(name, &args, state, owner_pubkey, jwt_sub, pre_resolved_mode).await
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
/// Payment-gating semantics (T2 round-2 — modes-user-choice): the gate
/// fires only when `mnemonic_sign_memory` is invoked AND the resolved
/// per-request `mode` is `Participate` AND `payment_mode != "none"`.
/// A `Local` request (explicit or env-fallback) bypasses the gate
/// entirely regardless of `STORAGE_MODE` — the whitepaper §5.7.1
/// free-local invariant is now structural, not configurational. The
/// resolved `WriteMode` is computed once here and threaded into
/// `handle_request_with_resolved_mode` so the dispatch column and the
/// gate decision come from the same value (drift impossible by
/// construction). On a gate-pass we run the full
/// `payment::check_payment` -> `deduct_balance` -> dispatch -> refund
/// flow. Each terminal state emits exactly one NDJSON frame.
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

    // T2 round-2: resolve the per-request `mode` field ONCE here, before
    // the paywall gate. The resolved value drives THREE things:
    //
    //   1. The paywall gate predicate below.
    //   2. The persisted `write_mode` column (threaded into
    //      `handle_request_with_resolved_mode` →  `handle_tool_call` →
    //      `tools::sign_memory` → `save_attestation`).
    //   3. The deferred-vs-inline routing in `sign_memory` (uses
    //      `ResolvedMode::is_explicit_local` to honour the user-spec
    //      invariant uniformly across deploys).
    //
    // Single source of truth → drift impossible by construction. On a
    // malformed value (case-variant, whitespace, null, etc.) we
    // short-circuit with `-32602 InvalidParams` BEFORE charging, BEFORE
    // touching storage, and after emitting a structured warn log for
    // operator visibility.
    let resolved_mode_for_gate: Option<crate::tools::ResolvedMode> = if is_sign_memory {
        let args = req.params.get("arguments").cloned().unwrap_or_default();
        match crate::tools::resolve_write_mode(args.get("mode"), &state.storage_mode) {
            Ok(r) => Some(r),
            Err(err) => {
                let received = args.get("mode").cloned().unwrap_or(Value::Null);
                tracing::warn!(
                    field = "mode",
                    received = %received,
                    "rejected non-canonical mode value"
                );
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
    let participate_gate = matches!(
        resolved_mode_for_gate.map(|r| r.write_mode),
        Some(WriteMode::Participate)
    );

    // T3 — outcome-based DoS guard. Consulted at the *entry* of the
    // participate path, BEFORE `check_payment`/`deduct_balance`, BEFORE
    // any Arweave/Solana write. The subject is the stable billable
    // identifier for the request:
    //
    //   - **balance mode**: `blake3(bearer_api_key)` — the operator-issued
    //     api_key the caller can't rotate without re-paying.
    //   - **x402 mode**: `blake3(tx_sig)` — the on-chain payment proof.
    //     After the round-2 nonce deferral, the same tx_sig is reusable on
    //     delivery failure (no charge), so it serves as a stable
    //     per-payment identifier. A fresh tx_sig means a fresh USDC
    //     payment — the caller is paying their own way around the quota,
    //     which is the right blast-radius.
    //
    // Keying on `owner_pubkey` (Ed25519) would let an attacker mint a new
    // identity per request → quota bypass. The chosen subject derivation
    // closes that gap for both auth methods.
    //
    // No-op on the stdio path (no Bearer JWT, no x402 header) since stdio
    // is trusted-local. No-op on `payment_mode == "none"` since there is
    // no billable subject to key on.
    if is_sign_memory && participate_gate && state.payment_mode != "none" {
        if let Some(subject) = derive_quota_subject(&headers, &state.payment_mode) {
            if state.refunds_by_subject.is_over(&subject) {
                state.delivery_metrics.record_quota_short_circuit();
                let err = delivery_quota_exceeded(
                    state.refunds_by_subject.window().as_secs(),
                    state.refunds_by_subject.threshold(),
                );
                tracing::warn!(
                    subject_hash = %subject,
                    threshold = state.refunds_by_subject.threshold(),
                    window_secs = state.refunds_by_subject.window().as_secs(),
                    "delivery quota exceeded — short-circuiting participate request"
                );
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id.clone().unwrap_or(Value::Null),
                    result: None,
                    error: Some(err),
                };
                // 429 Too Many Requests — semantically correct for an
                // outcome-based quota; matches existing
                // tower_governor 429 returns elsewhere in the stack.
                return ndjson_response(StatusCode::TOO_MANY_REQUESTS, &resp);
            }
        }
    }

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
            state.evm_payment.as_ref(),
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

                let resp = handle_request_with_resolved_mode(
                    &req,
                    &state,
                    &owner_pubkey,
                    jwt_sub.as_deref(),
                    resolved_mode_for_gate,
                )
                .await;

                // Refund + bookkeeping on tool failure. Uses
                // `refund_balance` (not `credit_deposit`) so the per-tx_sig
                // idempotency guard does not silently swallow repeated
                // refunds for the same underlying failure class.
                //
                // T3 (modes-user-choice) — the typed
                // `-32011 DeliveryNotConfirmed` carries the demoted
                // `attestation_id` in `data.attestation_id` so the refund
                // reason can correlate 1:1 with the downgrade. We also:
                //   1. Increment the per-stage `delivery_not_confirmed_total`
                //      counter (no per-tenant label — high-cardinality
                //      anti-pattern; per-tenant detail goes to the
                //      `tracing::warn!` line emitted from
                //      `sign_memory_inline`).
                //   2. On the participate path the SAME error increments
                //      the `RefundsBySubject` counter so the entry-of-
                //      participate quota guard fires after `threshold`
                //      consecutive demotions. The subject is derived from
                //      `derive_quota_subject(headers, payment_mode)` so it
                //      matches the value the entry quota-check already
                //      computed (drift-impossible).
                //   3. On refund-itself failure write a structured
                //      `payment_events` audit row via
                //      `payment::record_refund_failed` so an operator can
                //      forensically trace stuck ledger state.
                //
                // T3 round-2 — on SUCCESS, consume the x402 nonce here
                // (deferred from `check_payment`). A delivery failure
                // leaves the nonce reusable so the caller's USDC payment
                // isn't forfeit when the operator's anchor isn't proved
                // retrievable. The race window between the entry
                // `x402_nonce_already_consumed` check and this INSERT is
                // resolved by the `x402_nonces.tx_sig` UNIQUE constraint:
                // the loser sees ConstraintViolation, which is the right
                // behaviour for two concurrent requests with the same
                // payment.
                let quota_subject = derive_quota_subject(&headers, &state.payment_mode);
                let x402_proof = payment::extract_x402_proof(&headers);

                if let Some(ref err) = resp.error {
                    // T3 — DeliveryNotConfirmed-specific bookkeeping.
                    // Extract `stage` + `attestation_id` from the typed
                    // error's `data` payload (set by
                    // `delivery_not_confirmed`).
                    let dnc_data = err
                        .data
                        .as_ref()
                        .filter(|d| d["kind"] == "DeliveryNotConfirmed");
                    let dnc_stage = dnc_data.and_then(|d| d["stage"].as_str()).unwrap_or("");
                    let dnc_attestation_id = dnc_data
                        .and_then(|d| d["attestation_id"].as_str())
                        .unwrap_or("");

                    if dnc_data.is_some() {
                        state.delivery_metrics.record_not_confirmed(dnc_stage);
                    }

                    // Refund — balance path only. x402 refund is implicit
                    // (the nonce was never consumed, so the same
                    // `X-Payment` header can be retried).
                    if let Some(ref key) = api_key {
                        // Refund reason format includes the demoted
                        // attestation_id so the `payment_events.description`
                        // column lets an operator grep
                        // `description LIKE '<id>%'` for the audit trail
                        // (tech-spec Decision 7).
                        let reason = if dnc_data.is_some() && !dnc_attestation_id.is_empty() {
                            format!("delivery_not_confirmed: {dnc_attestation_id}")
                        } else {
                            err.message.clone()
                        };

                        let refund_result = {
                            let store = state.store.lock().expect("store mutex poisoned");
                            payment::refund_balance(&store, key, current_cost, &reason)
                        }; // mutex dropped here; no `.await` while held

                        if let Err(refund_err) = refund_result {
                            let log_subject = quota_subject.clone().unwrap_or_default();
                            tracing::warn!(
                                subject_hash = %log_subject,
                                error = %refund_err,
                                attestation_id = %dnc_attestation_id,
                                "refund failed; writing audit row"
                            );
                            // Best-effort audit row. Lives in `mcp/` per
                            // the project's hard architectural rule.
                            // Body sticks to the PII allow-list pinned
                            // in the spec: subject_hash (NOT raw key),
                            // attestation_id, reason, occurred_at. No
                            // content_preview, no cose_bytes, no
                            // embedding.
                            let now = chrono::Utc::now().to_rfc3339();
                            let store = state.store.lock().expect("store mutex poisoned");
                            let _ = payment::record_refund_failed(
                                &store,
                                &log_subject,
                                dnc_attestation_id,
                                "refund-itself-failed",
                                &now,
                            );
                        }
                    }

                    // T3 — increment the per-subject quota counter on
                    // delivery demotions only. Other failure classes
                    // (e.g. embed/Arweave failure before the delivery
                    // check) do NOT count against the quota; only the
                    // induced-refund pattern matters for the DoS
                    // mitigation. Counter increment happens OUTSIDE
                    // the SQLite mutex (Decision 8). Fires for BOTH
                    // balance- and x402-authed callers — the subject
                    // derivation handles auth method dispatch.
                    if dnc_data.is_some() {
                        if let Some(ref subject) = quota_subject {
                            state.refunds_by_subject.record_failure(subject);
                        }
                    }
                } else {
                    // Success path — consume the x402 nonce now that the
                    // delivery confirmation has passed. ConstraintViolation
                    // on a concurrent retry is fine (one of the two
                    // requests wins; the other gets the
                    // `x402_nonce_already_consumed` reject on its next
                    // entry).
                    if let Some(proof) = x402_proof.as_ref() {
                        let store = state.store.lock().expect("store mutex poisoned");
                        if let Err(e) =
                            payment::consume_x402_nonce_after_success(&store, &proof.tx_sig)
                        {
                            // Log only — by the time we reach here the
                            // anchor + DB write have already happened, so
                            // a nonce-consume failure cannot un-deliver
                            // the artefact. The operator may see a
                            // duplicate-charge later if the caller replays
                            // the same `X-Payment` and the original
                            // INSERT actually did succeed under a race.
                            tracing::warn!(
                                tx_sig = %proof.tx_sig,
                                error = %e,
                                "x402 nonce consume failed post-success"
                            );
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
        let resp = handle_request_with_resolved_mode(
            &req,
            &state,
            &owner_pubkey,
            jwt_sub.as_deref(),
            resolved_mode_for_gate,
        )
        .await;
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
    pre_resolved_mode: Option<crate::tools::ResolvedMode>,
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
            // T2 round-2: use the pre-resolved mode if `mcp_handler` already
            // parsed it for the paywall gate (HTTP transport — single call
            // site, drift impossible). The stdio dispatch path passes `None`
            // here and we resolve on demand below.
            let resolved = match pre_resolved_mode {
                Some(r) => r,
                None => match tools::resolve_write_mode(args.get("mode"), &state.storage_mode) {
                    Ok(r) => r,
                    Err(e) => {
                        // Log the rejection so probe traffic is visible to
                        // operators (security-auditor round-1 minor).
                        let received = args.get("mode").cloned().unwrap_or(Value::Null);
                        tracing::warn!(
                            field = "mode",
                            received = %received,
                            "rejected non-canonical mode value on stdio path"
                        );
                        return Err(e);
                    }
                },
            };
            // Task 4 — visibility (Decision 3 + AC14) and the
            // allow_fallback_to_participate opt-in (Decision 4). Both
            // resolved here so the public-write gate can fire BEFORE the
            // tool body and so soft-fall routing in Task 5 has the resolved
            // value. Visibility may NOT be present alongside `mode=local`
            // — `resolve_visibility` returns `-32602 InvalidParams` in that
            // case.
            let visibility = tools::resolve_visibility(args, resolved.write_mode)?;
            // `allow_fallback` is parsed here so any malformed value is
            // rejected at the dispatcher boundary before storage / payment
            // side effects. Task 5 wires this into `tools::sign_memory`'s
            // post-failure branch in `mcp/src/tools.rs`: when
            // `allow_fallback_to_participate=true` AND local execution fails
            // with one of the soft-fallable typed errors (`-32098
            // EmbedderInvalid`, `-32099 LocalStorageBusy`, `-32094
            // IdentityBootstrapFailed`), `sign_memory` re-dispatches the
            // same arguments through the hosted participate-mode proxy
            // (`state.hosted_endpoint`, resolved at process start and gated
            // behind `--allow-custom-endpoint` per Decision 12). The
            // response gains an `escalated: { from, to, reason }` marker
            // per Decision 4; on hosted unavailability the typed error is
            // `-32011 HostedUnavailable`, NOT the original local-failure
            // code.
            let allow_fallback = tools::resolve_allow_fallback(args)?;

            // Decision 5b — public-write confirmation gate. Fires only when
            // the caller has explicitly opted into `participate + public`;
            // the default `private` path is unaffected. Owner_pubkey is
            // server-derived (the dispatcher's `owner_pubkey` is sourced
            // from `claims.sub` on the HTTP path), never client-supplied —
            // a cross-owner replay would present mismatched owner here and
            // the consume returns `Invalid`.
            if resolved.write_mode == mnemonic_core::storage::WriteMode::Participate
                && visibility == mnemonic_core::storage::Visibility::Public
            {
                let token_b64 = args
                    .get("public_write_confirmation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let jti_raw = args.get("jti").and_then(|v| v.as_str()).unwrap_or("");
                let content_hash_arg = blake3::hash(content.as_bytes()).to_hex().to_string();
                let parsed_jti = uuid::Uuid::parse_str(jti_raw).ok();
                let consume_result = match parsed_jti {
                    Some(jti) if !token_b64.is_empty() => state.confirmation_ledger.consume(
                        token_b64,
                        &jti,
                        &content_hash_arg,
                        owner_pubkey,
                        mnemonic_core::storage::Visibility::Public,
                    ),
                    _ => Err(crate::confirmation_token::ConfirmError::Invalid),
                };
                if consume_result.is_err() {
                    tracing::warn!(
                        owner_pubkey = %owner_pubkey,
                        content_hash = %content_hash_arg,
                        "public-write confirmation rejected — missing, expired, replayed, or HMAC-mismatched token"
                    );
                    return Err(public_write_requires_confirmation(&content_hash_arg));
                }
            }

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
                resolved,
                visibility,
                &state.envelope,
                state.delivery_refetch_timeout,
                allow_fallback,
                &state.hosted_endpoint,
                &state.hosted_client,
                args,
            )
            .await
            .map_err(tool_error_to_json_rpc)?
        }
        "mnemonic_verify" => {
            let sol = args.get("solana_tx").and_then(|v| v.as_str());
            let ar = args.get("arweave_tx").and_then(|v| v.as_str());
            // T4: pass `owner_pubkey` so the storage routing lookup
            // (`find_write_mode_by_tx`) is tenant-scoped. The
            // `storage_mode` argument is retained for ABI compatibility
            // but ignored — routing is by stored `write_mode` now.
            tools::verify(
                &state.solana,
                &state.arweave,
                &state.store,
                sol,
                ar,
                owner_pubkey,
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
            // Decision 5 / AC13 — agent-native-distribution (round 2 / SAR1-M1):
            //
            //   - Anonymous caller (`jwt_sub.is_none()`): scope is the
            //     CROSS-OWNER public pool. Pass `owner_pubkey = None` AND
            //     `visibility_filter = Some(Public)`. The storage layer
            //     drops the owner predicate; only `visibility = 'public'`
            //     rows surface (private rows stay invisible regardless of
            //     owner — privacy contract preserved).
            //   - Authenticated caller: scope is the caller's own corpus
            //     across both visibilities. Pass `owner_pubkey = Some(sub)`
            //     AND `visibility_filter = None`.
            //
            // SAR1-M1 round-1 had `owner_pubkey = owner_pubkey` (server
            // keypair fallback) for anonymous — that scoped anonymous recall
            // to server-keypair rows only, contradicting the user-spec
            // "public part of the pool". Fixed here.
            let (recall_owner, visibility_filter): (Option<&str>, _) = if jwt_sub.is_none() {
                (None, Some(mnemonic_core::storage::Visibility::Public))
            } else {
                (Some(owner_pubkey), None)
            };
            // DB-only: lock, query, release
            let store = state.store.lock().unwrap();
            tools::recall(
                &state.keypair,
                &store,
                state.embedder.as_ref(),
                query,
                limit,
                recall_owner,
                visibility_filter,
            )
        }
        "mnemonic_check_pending" => {
            let cid = args["correlation_id"]
                .as_str()
                .ok_or_else(|| JsonRpcError::simple(-32603, "correlation_id required"))?
                .to_string();
            tools::check_pending(&state.pending, &state.store, &cid).await
        }
        // request_public_write_confirmation — Decision 5b. Mints an
        // HMAC-bound, single-use confirmation token for a specific
        // (content_hash, owner_pubkey, visibility=Public) tuple. JWT is
        // REQUIRED at mint time: the tool is NOT in `ALLOWLIST_METHODS`,
        // so the bearer-auth middleware already rejected callers without
        // valid `Claims` with `-32001`. `owner_pubkey` here is server-
        // derived from `claims.sub` (the dispatcher's resolution), so the
        // HMAC binds the token to the authenticated owner — a cross-owner
        // replay attempts at consume time will fail HMAC reconstruction.
        "request_public_write_confirmation" => {
            // Belt-and-braces: even though the middleware allowlist guards
            // this method, double-check we have an authenticated `jwt_sub`.
            // The dispatcher's `owner_pubkey` fallback (server keypair)
            // would let an anonymous mint slip through if this guard were
            // missing — defending in depth.
            if jwt_sub.is_none() {
                return Err(JsonRpcError::simple(
                    -32001,
                    "request_public_write_confirmation requires authentication",
                ));
            }
            let content_hash = args
                .get("content_hash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    invalid_params(
                        "content_hash",
                        &args.get("content_hash").cloned().unwrap_or(Value::Null),
                    )
                })?;
            // SAR1-L2 (round 1 security audit, agent-native-distribution Task 4):
            // require the `content_hash` to be exactly 64 lowercase-or-uppercase
            // hex characters — the canonical blake3 hex shape. Without this,
            // an authenticated caller can spam `mint()` with arbitrary garbage
            // hashes to inflate the in-process DashMap until the 60s eviction
            // sweep catches up; the validation moves the boundary up to the
            // dispatcher so only well-formed blake3 hex tokens land in the
            // ledger. A consume against a garbage-bound token would still
            // fail (content_hash recomputed from actual content at consume
            // time), but accepting bad inputs at mint is a DoS amplifier we
            // can close cheaply.
            if content_hash.len() != 64 || !content_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(invalid_params(
                    "content_hash",
                    &Value::String(content_hash.to_string()),
                ));
            }
            let (token, jti, expires_at) = state.confirmation_ledger.mint(
                content_hash,
                owner_pubkey,
                mnemonic_core::storage::Visibility::Public,
            );
            serde_json::json!({
                "confirmation_token": token,
                "jti": jti.to_string(),
                "expires_at": expires_at,
            })
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

/// Translate a [`tools::ToolError`] into a `JsonRpcError`.
///
/// Round-2 (security-auditor minor): replaces the round-1 parser that
/// round-tripped JsonRpcError through `anyhow::Error.to_string()` as
/// JSON. That approach let any downstream error whose `Display` happened
/// to be valid JSON with a numeric `code` forge a typed error code. The
/// typed [`tools::ToolError`] carrier makes the dispatch decision
/// type-safe at the language level — the JsonRpcError is never a string
/// until it reaches the wire.
fn tool_error_to_json_rpc(e: crate::tools::ToolError) -> JsonRpcError {
    match e {
        crate::tools::ToolError::TypedRpc(rpc) => rpc,
        crate::tools::ToolError::Other(any) => JsonRpcError::simple(-32603, any.to_string()),
    }
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
            admin_token: String::new(),
            evm_payment: None,
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

    /// 32-byte test secret for OAuth JWT verification. Matches the
    /// production length requirement (Decision 11).
    const TEST_JWT_SECRET: &[u8; 32] = b"unit-test-secret-32-bytes-long!!";

    /// Build a `Router` with the `/mcp` route plus the bearer-auth middleware
    /// (Task 4 — `oauth::bearer_auth_middleware`). Mirrors the production
    /// wiring in `main.rs::run_http`. The middleware allows JSON-RPC
    /// `initialize` and `tools/list` without a JWT (per Decision 9) so the
    /// existing `test_chunked_response_encoding` test keeps passing.
    fn build_test_router(state: Arc<McpState>) -> Router {
        let oauth_state = Arc::new(crate::oauth::OAuthState::with_defaults(TEST_JWT_SECRET));
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
            7,
            "expected 7 MCP tools in tools/list response (whoami, sign_memory, verify, prove_identity, recall, check_pending, request_public_write_confirmation)",
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
        let oauth_state = crate::oauth::OAuthState::with_defaults(TEST_JWT_SECRET);
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
    /// allowlisted (Decision 9). `mnemonic_recall` is allowlisted by
    /// Task 4 / AC13 (agent-native-distribution) for anonymous-public
    /// discovery, so this test uses `mnemonic_sign_memory` — which is
    /// NOT allowlisted and must still 401 without a Bearer JWT.
    #[tokio::test]
    async fn test_missing_authorization_header_returns_401() {
        let state = build_test_state();
        let app = build_test_router(state);

        let req_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_sign_memory", "arguments": {"content": "x"}},
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
