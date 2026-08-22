//! Implementation of the 5 Mnemonic MCP tools.
//!
//! Week 3: sign_memory and verify now use the CBOR/COSE codec pipeline.
//! - Content hash: blake3(canonical_cbor) instead of SHA-256(content)
//! - Arweave payload: COSE_Sign1 envelope (not raw JSON)
//! - Solana anchor: {"h": blake3_hash, "a": arweave_tx, "v": 2}

use solana_sdk::signature::Keypair;

use std::time::Duration;

use mnemonic_core::arweave::{recovery::RecoveredItem, ArweaveClient};
use mnemonic_core::codec::{
    canonical::{from_canonical_cbor, to_canonical_cbor},
    hash::hash_bytes as blake3_hash,
    schema,
    sign::{sign_artifact, verify_artifact as cose_verify},
};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::embed::{cosine_similarity, Embedder};
use mnemonic_core::identity;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};

use crate::mcp::{
    delivery_not_confirmed, hosted_unavailable, invalid_params, public_write_requires_confirmation,
    token_expired, unsupported_mode, Envelope, JsonRpcError,
};
use crate::pending::PendingBundles;
use crate::{payment, pricing::CostHint};

/// Outcome of resolving the per-request `mode` field. Carries the resolved
/// [`WriteMode`] **plus** whether it came from the caller's explicit input
/// or from the env-var fallback path.
///
/// Round-2 review (security-auditor major): keeping these two dimensions
/// distinct lets the routing rule in `sign_memory` say "**explicit local
/// always goes inline**" without bringing the envelope into the predicate
/// — the envelope's `supports_participate` check was a workaround for the
/// missing explicit-vs-fallback distinction and silently broke scenario
/// (c) (explicit `mode: "local"` on a local-only deploy went to the
/// deferred branch instead of the free inline path the user-spec
/// invariant promises).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedMode {
    pub write_mode: WriteMode,
    /// True when the caller sent an explicit `"mode": "local"` /
    /// `"mode": "participate"` field. False when the caller omitted the
    /// field and the resolver applied env-var fallback.
    pub explicit: bool,
}

impl ResolvedMode {
    fn explicit(write_mode: WriteMode) -> Self {
        Self {
            write_mode,
            explicit: true,
        }
    }

    fn fallback(write_mode: WriteMode) -> Self {
        Self {
            write_mode,
            explicit: false,
        }
    }

    /// True when the caller explicitly asked for `Local`. Used by
    /// `sign_memory` to bypass the deferred-signing (Cloud-tier) branch in
    /// favour of the inline free-local path — the user-spec invariant
    /// "Личная память бесплатна всегда" applies regardless of deploy
    /// variant (full or local-only).
    pub fn is_explicit_local(&self) -> bool {
        self.explicit && self.write_mode == WriteMode::Local
    }
}

/// Resolve the per-request `mode` field on `mnemonic_sign_memory` to a
/// concrete [`ResolvedMode`]. This is the **single source of truth** that
/// drives BOTH the paywall gate in `mcp_handler` AND the persisted
/// `write_mode` column on the attestation row — by construction they
/// cannot drift (Decision 1 in work/modes-user-choice/tech-spec.md).
///
/// Resolution rules (tech-spec §"API contract changes / Resolution rule"):
///
/// | Input             | Output                                                        |
/// |-------------------|---------------------------------------------------------------|
/// | `None` (absent)   | env-var fallback: `local` iff `env_storage_mode == "local"`,  |
/// |                   | else `Participate` (marked `explicit = false`)                |
/// | `"local"`         | `WriteMode::Local` (`explicit = true`)                        |
/// | `"participate"`   | `WriteMode::Participate` (`explicit = true`)                  |
/// | anything else     | `Err(invalid_params("mode", received_verbatim))`              |
///
/// "Anything else" covers: JSON `null`, non-string types (integer, array,
/// object), empty `""`, whitespace `" "`, capitalised `"Local"` /
/// `"PARTICIPATE"`, unknown strings. The verbatim received `Value` is
/// echoed in the error's `data.received` so a misbehaving client can diff.
///
/// Pure function — no I/O, no globals. The full resolution table is
/// table-driven-tested in `mcp::tests::resolve_write_mode_*`.
pub fn resolve_write_mode(
    input_mode: Option<&serde_json::Value>,
    env_storage_mode: &str,
) -> Result<ResolvedMode, JsonRpcError> {
    match input_mode {
        None => {
            // Backward-compat: the shipped chrome-extension and pre-T2 stdio
            // clients never send `mode`. Resolve from env-var so their
            // behaviour is byte-for-byte unchanged. Marked `explicit = false`
            // so the routing rule in `sign_memory` knows this is the legacy
            // fallback path (deferred branch still applies when JWT is set).
            if env_storage_mode == "local" {
                Ok(ResolvedMode::fallback(WriteMode::Local))
            } else {
                Ok(ResolvedMode::fallback(WriteMode::Participate))
            }
        }
        Some(serde_json::Value::String(s)) => match WriteMode::from_str_strict(s) {
            Some(m) => Ok(ResolvedMode::explicit(m)),
            // `from_str_strict` rejects `"Local"`, `"PARTICIPATE"`, `""`,
            // `" "`, trailing whitespace, and any unknown string. Echo the
            // raw string back through `data.received` (not `s` directly —
            // we want the JSON Value variant preserved). Caller is
            // expected to also emit a `tracing::warn!` line — done at the
            // dispatcher boundary (`mcp_handler`) so logging discipline
            // stays in one place, not scattered across resolver callers.
            None => Err(invalid_params(
                "mode",
                input_mode.expect("Some matched above"),
            )),
        },
        // Non-string (null, integer, array, object) — strict rejection.
        Some(v) => Err(invalid_params("mode", v)),
    }
}

/// Resolve the per-request `visibility` field on `mnemonic_sign_memory`
/// (Decision 3 / AC14 — agent-native-distribution).
///
/// Rules:
///
/// | Input                                            | Output                                           |
/// |--------------------------------------------------|--------------------------------------------------|
/// | absent                                           | `Visibility::Private`                            |
/// | `"private"` / `"public"` AND mode = participate  | parsed variant                                   |
/// | any present value AND mode = local               | `Err(invalid_params("visibility", ...))` (AC14)  |
/// | non-string / non-canonical (under participate)   | `Err(invalid_params("visibility", received))`    |
///
/// The local-mode rejection fires for ANY present `visibility` value
/// (including the literal `"private"`), not only `"public"`. Visibility is a
/// participate-only concept; allowing `"private"` on local writes would leak
/// dead metadata into a column the row never consults.
///
/// Pure function — no I/O, no globals.
pub fn resolve_visibility(
    args: &serde_json::Value,
    resolved_mode: WriteMode,
) -> Result<Visibility, JsonRpcError> {
    let raw = args.get("visibility");
    match raw {
        None => Ok(Visibility::default()),
        Some(v) => {
            // AC14 — `visibility` is invalid params on local writes regardless
            // of the underlying value. Rejecting at the boundary keeps the
            // matrix `{local, public}` cell undefined-by-construction.
            if resolved_mode == WriteMode::Local {
                return Err(invalid_params("visibility", v));
            }
            match v {
                serde_json::Value::String(s) => match Visibility::from_str_strict(s) {
                    Some(vis) => Ok(vis),
                    None => Err(invalid_params("visibility", v)),
                },
                _ => Err(invalid_params("visibility", v)),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointScope {
    pub checkpoint_type: String,
    pub workspace: Option<String>,
}

/// Validate the optional payment/session scope carried by sign-memory calls.
/// Automatic checkpoint kinds require a workspace so a future paid session
/// cannot accidentally authorize background work across every project.
pub fn resolve_checkpoint_scope(args: &serde_json::Value) -> Result<CheckpointScope, JsonRpcError> {
    let checkpoint_type = match args.get("checkpoint_type") {
        None => "manual".to_string(),
        Some(serde_json::Value::String(value))
            if matches!(value.as_str(), "manual" | "pre_compaction" | "session_end") =>
        {
            value.clone()
        }
        Some(value) => return Err(invalid_params("checkpoint_type", value)),
    };
    let workspace = match args.get("workspace") {
        None => None,
        Some(serde_json::Value::String(value))
            if !value.trim().is_empty() && value.len() <= 256 =>
        {
            Some(value.clone())
        }
        Some(value) => return Err(invalid_params("workspace", value)),
    };
    if checkpoint_type != "manual" && workspace.is_none() {
        return Err(invalid_params("workspace", &serde_json::Value::Null));
    }
    Ok(CheckpointScope {
        checkpoint_type,
        workspace,
    })
}

/// Resolve the per-request `allow_fallback_to_participate` field
/// (Decision 4 — agent-native-distribution soft-fall opt-in).
///
/// Strict bool. Absent → `false`. Non-bool returns `invalid_params` with the
/// verbatim received value echoed back so a misbehaving client can diff
/// against its own outgoing payload.
pub fn resolve_allow_fallback(args: &serde_json::Value) -> Result<bool, JsonRpcError> {
    let raw = args.get("allow_fallback_to_participate");
    match raw {
        None => Ok(false),
        Some(serde_json::Value::Bool(b)) => Ok(*b),
        Some(v) => Err(invalid_params("allow_fallback_to_participate", v)),
    }
}

/// Typed error returned from `sign_memory` so the dispatcher can
/// distinguish a typed JSON-RPC error (e.g. `-32010 UnsupportedMode`)
/// from a generic `anyhow::Error` (e.g. an Arweave write failure).
///
/// Round-2 review (security-auditor minor): the round-1 implementation
/// smuggled JsonRpcError through `anyhow::Error.to_string()` and
/// reconstituted it by parsing the Display output as JSON. That parser
/// would happily reconstitute any error whose `Display` happened to be
/// a valid JSON object with a numeric `code` — an attacker-controlled
/// content path (e.g. a downstream service error containing JSON in
/// its message) could forge a typed error code. The typed carrier
/// here makes the dispatch decision type-safe; the JsonRpcError is
/// never a string until it reaches the wire.
#[derive(Debug)]
pub enum ToolError {
    /// Already-typed JSON-RPC error — propagate verbatim through the
    /// dispatcher.
    TypedRpc(JsonRpcError),
    /// Opaque error (Arweave/Solana/SQLite failure, etc.). The
    /// dispatcher wraps it in `-32603 InternalError`.
    Other(anyhow::Error),
}

impl From<anyhow::Error> for ToolError {
    fn from(e: anyhow::Error) -> Self {
        ToolError::Other(e)
    }
}

impl From<JsonRpcError> for ToolError {
    fn from(e: JsonRpcError) -> Self {
        ToolError::TypedRpc(e)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::TypedRpc(e) => write!(f, "{} (code {})", e.message, e.code),
            ToolError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Tool 1: whoami (sync — DB only)
///
/// T2 extension: returns the discoverability envelope (`supported_modes`,
/// `default_mode`, `participate_cost`) alongside the existing fields so
/// clients can choose `local` vs `participate` BEFORE attempting to write.
/// The legacy `storage_mode` field is kept verbatim for pre-envelope clients
/// (chrome-extension Cloud tier still reads it).
pub fn whoami(
    keypair: &Keypair,
    store: &SqliteStore,
    storage_mode: &str,
    envelope: &Envelope,
) -> serde_json::Value {
    let pubkey = identity::pubkey_base58(keypair);
    let count = store.count(&pubkey).unwrap_or(0);
    // Serialize the envelope through serde_json so the `null` rendering of
    // `participate_cost: Option<ParticipateCost>` and the static `&'static
    // str` arrays in `supported_modes` come out byte-identical to the
    // spec'd wire shape (no manual JSON construction drift).
    let envelope_value = serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null);
    let envelope_obj = envelope_value.as_object().cloned().unwrap_or_default();
    let mut out = serde_json::json!({
        "public_key": pubkey,
        "did_sol": identity::did_sol(keypair),
        "did_key": identity::did_key(keypair),
        "attestation_count": count,
        "storage_mode": storage_mode,
    });
    // Merge envelope keys (`supported_modes`, `default_mode`,
    // `participate_cost`) into the response. Done as a post-merge rather
    // than inline so the field order in the json! macro stays stable for
    // the golden fixture.
    if let Some(map) = out.as_object_mut() {
        for (k, v) in envelope_obj {
            map.insert(k, v);
        }
    }
    out
}

/// Tool 2: sign_memory — branches on `jwt_sub`.
///
/// **HTTP/JWT path** (`jwt_sub.is_some()`, Decision 12):
///   embed content → compress → build canonical-CBOR over the unsigned
///   artifact → blake3-hash → park in `PendingBundles` and return
///   `{status: "awaiting_signature", approve_url, correlation_id, expires_in: 300}`.
///   No COSE signing, no Arweave/Solana writes, no SQLite row created.
///   The webapp finishes the flow by signing locally and POSTing
///   `/api/sign-callback` (handled in `mcp.rs`).
///
/// **Stdio path** (`jwt_sub.is_none()`):
///   preserves the existing inline pipeline byte-for-byte:
///   JSON → canonical CBOR → blake3 → COSE_Sign1 → Arweave + Solana (full
///   mode) or synthetic tx IDs (local mode) → SQLite. Backward-compat for
///   single-tenant CLI / Claude Code.
///
/// `owner_pubkey` (Decision 9) is the OAuth-resolved tenant scope used by
/// `recall`. HTTP transport passes `claims.sub`; stdio transport passes
/// the local keypair pubkey.
#[allow(clippy::too_many_arguments)]
pub async fn sign_memory(
    keypair: &Keypair,
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    pending: &PendingBundles,
    content: &str,
    tags: &[String],
    cost_hint: &CostHint,
    storage_mode: &str,
    owner_pubkey: &str,
    jwt_sub: Option<&str>,
    resolved: ResolvedMode,
    visibility: Visibility,
    envelope: &Envelope,
    delivery_refetch_timeout: Duration,
    // Task 5 — agent-native-distribution Decision 4. The soft-fall router
    // sits BETWEEN this entrypoint and the failing inline path. Set to
    // `true` only when the caller has explicitly opted in via
    // `allow_fallback_to_participate`. The hosted_endpoint + hosted_client
    // come from `McpState`; an empty `hosted_endpoint` disables soft-fall
    // (test fixture sentinel).
    allow_fallback: bool,
    hosted_endpoint: &str,
    hosted_client: &reqwest::Client,
    args: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    // T2 — UnsupportedMode check fires BEFORE the JWT-deferred branch so a
    // browser client asking for `participate` against a local-only deploy
    // gets the typed error even when it would otherwise enter the
    // deferred-signing path. The user explicitly asked to anchor on-chain;
    // the server cannot fulfil that intent regardless of whether the
    // signing path is server-side or browser-side.
    if resolved.write_mode == WriteMode::Participate && !envelope.supports_participate() {
        return Err(ToolError::TypedRpc(unsupported_mode(
            "participate",
            &envelope.supported_modes,
        )));
    }
    // Routing rule (Wave 3 — remove operator signing for remote users):
    //
    // The operator's keypair must NEVER produce a COSE signature over a
    // memory authored by a *different* identity. Inline signing
    // (`sign_memory_inline` → `sign_artifact(.., keypair)`) is therefore
    // legal ONLY when the writer IS the operator itself — i.e. the resolved
    // `owner_pubkey` equals the operator pubkey. That covers:
    //   - the stdio / Claude Code single-tenant path (no JWT; owner is the
    //     local identity == `keypair`), and
    //   - a self-call where a JWT subject happens to be the operator's own
    //     pubkey (e.g. RAG self-knowledge, test harness).
    //
    // Any JWT write owned by a different identity is routed to the
    // client-signing (deferred) path — regardless of write_mode, INCLUDING
    // explicit `mode: "local"`. A remote user's free local write is now
    // client-signed too (the deferred bundle carries `write_mode` so the
    // sign-callback persists it as `Local` with synthetic ids, still free).
    // This closes the last custodial gap: previously explicit-local + JWT
    // fell through to inline and the operator signed the user's content.
    let operator_pubkey = identity::pubkey_base58(keypair);
    if let Some(sub) = jwt_sub {
        let is_self_write = owner_pubkey == operator_pubkey;
        if !(resolved.is_explicit_local() && is_self_write) {
            let checkpoint = resolve_checkpoint_scope(args).map_err(ToolError::TypedRpc)?;
            return sign_memory_deferred(
                embedder,
                compressor,
                pending,
                content,
                tags,
                sub,
                resolved.write_mode,
                visibility,
                checkpoint,
            )
            .await
            .map_err(ToolError::Other);
        }
    }
    let inline_result = sign_memory_inline(
        keypair,
        solana,
        arweave,
        store,
        embedder,
        compressor,
        content,
        tags,
        cost_hint,
        storage_mode,
        owner_pubkey,
        resolved.write_mode,
        visibility,
        delivery_refetch_timeout,
    )
    .await;

    match inline_result {
        Ok(v) => Ok(v),
        Err(e) => {
            // Decision 4 — soft-fall opt-in. The router runs ONLY if all of:
            //   (1) caller passed `allow_fallback_to_participate=true`
            //   (2) the local error is in the soft-fallable catalogue
            //       (EmbedderInvalid / LocalStorageBusy / IdentityBootstrapFailed)
            //   (3) a non-empty hosted endpoint is configured
            // Any other failure (UnsupportedMode, DeliveryNotConfirmed,
            // PublicWriteRequiresConfirmation, opaque Other) flows through
            // verbatim — soft-fall is for *local capability* failures only.
            if !allow_fallback || hosted_endpoint.is_empty() {
                return Err(e);
            }
            let reason = match &e {
                ToolError::TypedRpc(rpc) => softfall_reason_from_error(rpc),
                ToolError::Other(_) => None,
            };
            let Some(reason) = reason else {
                return Err(e);
            };
            // Proxy the same arguments through the hosted endpoint with
            // `mode` swapped to `participate`. Visibility resolution runs
            // AGAIN on the hosted side (Decision 4 — the public-write
            // confirmation gate from Task 4 still fires). On hosted
            // unavailability we return `-32011 HostedUnavailable` so the
            // agent sees the actual failure point, NOT the original local
            // failure code.
            tracing::warn!(
                target: "mnemonic_mcp::tools",
                reason = reason.as_str(),
                "sign_memory: soft-fall escalating to participate via hosted endpoint"
            );
            proxy_participate(hosted_client, hosted_endpoint, args, jwt_sub, reason).await
        }
    }
}

/// Map a typed JSON-RPC error returned from the local `sign_memory_inline`
/// path to its `escalated.reason` enum value (Decision 4 —
/// agent-native-distribution). Errors that are NOT in the soft-fallable set
/// (delivery failures, unsupported mode, public-write gate violations)
/// return `None` so the caller propagates the error verbatim instead of
/// escalating.
fn softfall_reason_from_error(rpc: &JsonRpcError) -> Option<EscalationReason> {
    let kind = rpc
        .data
        .as_ref()
        .and_then(|d| d.get("kind"))
        .and_then(|v| v.as_str())?;
    match (rpc.code, kind) {
        (-32098, "EmbedderInvalid") => Some(EscalationReason::EmbedderUnavailable),
        (-32099, "LocalStorageBusy") => Some(EscalationReason::LocalStorageBusy),
        (-32094, "IdentityBootstrapFailed") => Some(EscalationReason::IdentityBootstrapFailed),
        _ => None,
    }
}

/// Machine-readable reason for `escalated.reason` in the soft-fall response.
/// Each variant maps 1:1 to a typed JSON-RPC error in the local catalogue
/// (Error Catalogue table — agent-native-distribution tech-spec).
#[derive(Debug, Clone, Copy)]
pub enum EscalationReason {
    /// `-32098 EmbedderInvalid` — local embedder unusable.
    EmbedderUnavailable,
    /// `-32099 LocalStorageBusy` — SQLite busy after the 5s busy_timeout.
    LocalStorageBusy,
    /// `-32094 IdentityBootstrapFailed` — `identity::ensure()` returned err.
    IdentityBootstrapFailed,
}

impl EscalationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmbedderUnavailable => "embedder_unavailable",
            Self::LocalStorageBusy => "local_storage_busy",
            Self::IdentityBootstrapFailed => "identity_bootstrap_failed",
        }
    }
}

/// Scrub a `reqwest::Error` for inclusion in a JSON-RPC `data.last_error`
/// field returned to the agent. SAR5-L1 (round-1 security audit): reqwest's
/// `Display` impl includes the full URL, which may contain credentials in
/// the userinfo component or sensitive path segments that should not leak
/// into agent context or downstream log aggregation. We render only:
///
/// - The error kind (`request`, `connect`, `timeout`, etc. via the canned
///   `is_*` accessors), and
/// - The host name (NOT the full URL — no userinfo, no path, no query).
///
/// `SAR5-M1`'s URL validation already rejects userinfo at the input
/// boundary; this is defence-in-depth in case future code paths build a
/// reqwest::Request without going through the validation gate.
fn scrub_reqwest_error(e: &reqwest::Error) -> String {
    let host = e
        .url()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_request() {
        "request"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else if e.is_status() {
        "status"
    } else {
        "transport"
    };
    format!("{kind} error to host {host}")
}

/// Proxy the caller's `sign_memory` arguments to the resolved hosted MCP
/// endpoint as a JSON-RPC `tools/call` for `mnemonic_sign_memory` with
/// `mode` rewritten to `"participate"`. Reuses the caller's `jwt_sub` for
/// Bearer auth when present; if no token is cached the hosted side will
/// return `-32001 unauthorized` and the agent must re-OAuth (Decision 7).
///
/// On any transport failure (DNS, TCP, TLS, non-2xx) returns
/// `-32011 HostedUnavailable` per Decision 4 — the agent sees the actual
/// failure point, not the original local-failure code. On a successful
/// hosted call the inner `result` is unwrapped, an `escalated` field is
/// injected, and the augmented JSON is returned. If the hosted side
/// returned a JSON-RPC error (e.g. `-32095 PublicWriteRequiresConfirmation`
/// because the caller didn't supply `public_write_confirmation`), that
/// error is propagated verbatim — Decision 4 + 5b interaction.
async fn proxy_participate(
    client: &reqwest::Client,
    endpoint: &str,
    args: &serde_json::Value,
    jwt_sub: Option<&str>,
    reason: EscalationReason,
) -> Result<serde_json::Value, ToolError> {
    // Build the re-dispatch arguments: clone the caller's args, override
    // `mode` to participate. `allow_fallback_to_participate` is dropped to
    // prevent recursive escalation if the hosted side itself reports a
    // local failure (mock-server bug, partial deploy). Visibility flows
    // through verbatim so the hosted public-write gate sees the same
    // intent the caller declared.
    let mut proxied_args = args.clone();
    if let Some(obj) = proxied_args.as_object_mut() {
        obj.insert(
            "mode".to_string(),
            serde_json::Value::String("participate".to_string()),
        );
        obj.remove("allow_fallback_to_participate");
    }

    // Decision 4 + 5b — post-escalation visibility re-resolution.
    //
    // The local request resolved with `mode=local + visibility=public +
    // allow_fallback_to_participate=true`; the dispatcher boundary at
    // `resolve_visibility` rejects local+public via AC14 before any of
    // this code runs, so reaching this branch implies an internal caller
    // that already pre-resolved a `Visibility::Public` value (or a future
    // refactor that admits public on the local path). EITHER way, the
    // soft-fall would now effectively land as a participate write —
    // exactly the path Decision 5b's HMAC-bound `public_write_confirmation`
    // gate exists to authorise. The hosted side will fire the gate AGAIN
    // (defence-in-depth, see test `opt_in_escalation_no_confirmation_token`),
    // but we also gate it LOCALLY so a buggy or compromised hosted operator
    // that returns success on a missing token cannot bypass the user-
    // approval ceremony.
    //
    // The local gate fires when the request's `visibility` field is
    // `"public"` AND either `public_write_confirmation` OR `jti` is
    // missing. We don't try to validate the token cryptographically here
    // (that is the hosted side's ledger's job); we only ensure the agent
    // surfaced the content to the user via the ceremony.
    let visibility_public = proxied_args
        .get("visibility")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("public"))
        .unwrap_or(false);
    if visibility_public {
        let has_token = proxied_args
            .get("public_write_confirmation")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let has_jti = proxied_args
            .get("jti")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_token || !has_jti {
            // Compute the content_hash from the request's content field so
            // the caller's typed-error envelope matches what the local
            // dispatcher would have returned without escalation. The
            // hosted side would otherwise compute the same hash; emitting
            // it locally avoids a tautological network round-trip.
            let content_hash = proxied_args
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| blake3::hash(s.as_bytes()).to_hex().to_string())
                .unwrap_or_default();
            tracing::warn!(
                target: "mnemonic_mcp::tools",
                reason = reason.as_str(),
                content_hash = %content_hash,
                "soft-fall escalation aborted before proxy: visibility=public without public_write_confirmation"
            );
            return Err(ToolError::TypedRpc(public_write_requires_confirmation(
                &content_hash,
            )));
        }
    }

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "mnemonic_sign_memory",
            "arguments": proxied_args,
        },
    });

    // Cached token lookup. Three branches — SAR5-INFO3 (round-1 security
    // audit) closure of the Task 6 forward flag:
    //   - `Ok(Some(token))` → cached JWT sent as Bearer; hosted side
    //     validates the signature/exp.
    //   - `Ok(None)` (file absent OR malformed JSON) → no Bearer header;
    //     hosted side returns `-32001 unauthorized` and the agent must
    //     re-OAuth.
    //   - `Err(Expired)` → SHORT-CIRCUIT with `-32099 TokenExpired`
    //     verbatim from the local catalogue. This is the canonical error
    //     code AC16 specifies for expired-token conditions; falling
    //     through to "no Bearer" would surface `-32001 unauthorized`
    //     instead and an agent programmed against the catalogue would
    //     not recognise it as the same condition.
    //   - `Err(Io/Parse)` (path resolution failed, etc.) → treat as
    //     "no token" for forward compatibility; the hosted side rejects
    //     and the agent re-OAuths. Logged at debug so a misconfigured
    //     `MNEMONIC_CONFIG_DIR` is visible to the operator.
    // `jwt_sub` is plumbed through the signature for symmetry with the
    // HTTP-path Cloud-tier branch but isn't used in the no-token fall-
    // through — the hosted side reads the JWT from the Bearer header, not
    // from our process state. Suppress the unused-binding lint at the
    // boundary rather than carrying a dead `let _ = jwt_sub;` inside the
    // match arm (R1-002, code-reviewer round 1).
    let _ = jwt_sub;
    let mut req = client.post(endpoint).json(&body);
    match mnemonic_core::identity::read_token() {
        Ok(Some(token)) => {
            req = req.bearer_auth(token.jwt);
        }
        Ok(None) => {
            // No cached token — hosted side will return -32001
            // unauthorized and the agent re-OAuths.
        }
        Err(mnemonic_core::identity::TokenStoreError::Expired { expires_at, sub }) => {
            return Err(ToolError::TypedRpc(token_expired(&expires_at, &sub)));
        }
        Err(e) => {
            tracing::debug!(
                target: "mnemonic_mcp::tools",
                error = %e,
                "soft-fall: read_token returned non-Expired error; proceeding without Bearer (hosted will return -32001)"
            );
        }
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = reason;
            return Err(ToolError::TypedRpc(hosted_unavailable(
                &scrub_reqwest_error(&e),
                500,
            )));
        }
    };
    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return Err(ToolError::TypedRpc(hosted_unavailable(
                &format!("body read failed: {}", scrub_reqwest_error(&e)),
                500,
            )));
        }
    };
    if !status.is_success() {
        return Err(ToolError::TypedRpc(hosted_unavailable(
            &format!("hosted endpoint returned HTTP {status}"),
            500,
        )));
    }

    let parsed: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            return Err(ToolError::TypedRpc(hosted_unavailable(
                &format!("malformed hosted response: {e}"),
                500,
            )));
        }
    };

    // JSON-RPC error from the hosted side propagates verbatim — Decision 4
    // + 5b interaction: a `-32095 PublicWriteRequiresConfirmation` returned
    // by the hosted public-write gate must reach the agent unchanged so
    // the public-write ceremony from Task 4 still applies post-escalation.
    if let Some(err) = parsed.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603) as i32;
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("hosted error")
            .to_string();
        // If the hosted side echoed `kind == PublicWriteRequiresConfirmation`,
        // re-derive the canonical helper to keep `data.suggested_action`
        // exactly aligned with the local catalogue (defence-in-depth
        // against a hosted operator returning a slightly-different shape).
        let kind = err
            .get("data")
            .and_then(|d| d.get("kind"))
            .and_then(|v| v.as_str());
        if kind == Some("PublicWriteRequiresConfirmation") {
            // The hosted side computed the hash; honour whatever it returned
            // (the content text is identical so blake3 collisions are nil).
            let content_hash = err
                .get("data")
                .and_then(|d| d.get("content_hash"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Err(ToolError::TypedRpc(public_write_requires_confirmation(
                content_hash,
            )));
        }
        return Err(ToolError::TypedRpc(JsonRpcError {
            code,
            message,
            data: err.get("data").cloned(),
        }));
    }

    // Successful escalation. The hosted side wraps its tool result in the
    // MCP `content: [{type:"text", text:"<pretty-JSON>"}]` envelope; we
    // mirror that here so the dispatcher's downstream wrapping is a no-op
    // and the agent sees a uniform shape.
    let result = parsed
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    // The result is the wrapper `{content:[{text:"<inner JSON>"}]}`; pull
    // the inner JSON out so we can inject `escalated` onto the same
    // object the caller would have seen on a successful local write.
    let inner_text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str());
    let mut inner_value: serde_json::Value = match inner_text {
        Some(s) => serde_json::from_str(s).unwrap_or(serde_json::Value::Null),
        None => result.clone(),
    };
    // R1-004 (code-reviewer round 1): if the hosted response is malformed
    // (no `content[0].text`, or the text isn't valid JSON object) the
    // unwrap above produced a `Null` or a non-object. `as_object_mut()`
    // would then silently drop the `escalated` injection and we'd return
    // `Ok(Null)` — the agent could not distinguish success from a parse
    // failure. Map that case to `HostedUnavailable` so the caller sees a
    // typed error and can retry / re-OAuth.
    let Some(obj) = inner_value.as_object_mut() else {
        return Err(ToolError::TypedRpc(hosted_unavailable(
            "malformed hosted response: missing or non-object content[0].text",
            500,
        )));
    };
    obj.insert(
        "escalated".to_string(),
        serde_json::json!({
            "from": "local",
            "to": "participate",
            "reason": reason.as_str(),
        }),
    );
    Ok(inner_value)
}

/// HTTP/JWT branch — Decision 12 deferred-signing path.
///
/// Builds the same unsigned artifact JSON as the inline path but with
/// `producer = did:sol:<jwt_sub>` and `artifact_id = correlation_id` so the
/// browser-side WASM signer is signing bytes that already encode the user's
/// identity. Parks the bundle in `PendingBundles`; the webapp picks it up
/// via `GET /api/pending/{correlation_id}`.
#[allow(clippy::too_many_arguments)]
async fn sign_memory_deferred(
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    pending: &PendingBundles,
    content: &str,
    tags: &[String],
    jwt_sub: &str,
    write_mode: WriteMode,
    visibility: Visibility,
    checkpoint: CheckpointScope,
) -> anyhow::Result<serde_json::Value> {
    let now = chrono::Utc::now().to_rfc3339();
    // 1. Embed (CPU-bound, can't defer)
    let embedding = embedder.embed(content);

    // 2. Compress for the canonical-CBOR `metadata.embedding_compressed` field
    let compressed = compressor.compress(&embedding);
    let compressed_bytes = compressed.to_bytes();

    // 3. Generate the correlation_id up front so it can double as artifact_id.
    //    (Avoids two distinct UUIDs for the same logical pending bundle.)
    let correlation_id = uuid::Uuid::new_v4().to_string();

    // 4. Build artifact JSON. `producer` is derived from jwt.sub, NOT the
    //    server keypair — the user is the signer, not the server.
    let metadata = serde_json::json!({
        "embed_provider": embedder.provider_name(),
        "embed_dim": embedder.dim(),
        "turbo_bits": compressed.bit_width,
        "embedding_compressed": base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &compressed_bytes,
        ),
    });
    let artifact = serde_json::json!({
        "artifact_id": correlation_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{jwt_sub}"),
        "created_at": now,
        "tags": tags,
        "metadata": metadata.clone(),
    });

    // 5. Canonical CBOR + blake3 hash
    let canonical_cbor = to_canonical_cbor(&artifact, &schema::MEMORY_V1)
        .map_err(|e| anyhow::anyhow!("canonical CBOR encode failed: {e}"))?;
    let content_hash = blake3_hash(&canonical_cbor);

    // Wave 2 — programmatic client-signing. Hand the unsigned canonical CBOR
    // back inline (base64) so a non-browser client (SDK/CLI/agent) can
    // COSE_Sign1 it locally with the user's own Ed25519 key and POST the
    // signed envelope to `/api/sign-callback` — no browser `approve_url`
    // round-trip required. This is the SAME bytes `GET /api/pending/{id}`
    // serves; returning it inline saves the headless client one round-trip.
    // The browser flow is untouched (it still uses `approve_url`).
    let canonical_cbor_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &canonical_cbor);

    // 6. Park under the same UUID already embedded as `artifact_id`. Keeping
    //    artifact, operation, callback and eventual attestation identifiers
    //    identical makes retries deterministic and avoids duplicate rows.
    let assigned_id = pending
        .insert_scoped(
            Some(correlation_id.clone()),
            jwt_sub.to_string(),
            content.to_string(),
            embedding,
            content_hash.clone(),
            canonical_cbor,
            tags.to_vec(),
            metadata,
            write_mode,
            visibility,
            checkpoint.checkpoint_type,
            checkpoint.workspace,
        )
        .await
        .map_err(|e| anyhow::anyhow!("pending insert failed: {e}"))?;

    Ok(serde_json::json!({
        "status": "awaiting_signature",
        "approve_url": format!("https://mnemonik.xyz/sign/{assigned_id}"),
        "correlation_id": assigned_id,
        "expires_in": 300,
        "content_hash": content_hash,
        // Wave 2 — programmatic (non-browser) client-signing handoff. A client
        // that holds the user's identity key (SDK/CLI/extension/agent) signs
        // `canonical_cbor_b64` locally and submits via `client_sign.submit_path`,
        // bypassing the browser entirely. Paths are relative to the MCP server
        // the client is already connected to.
        "canonical_cbor_b64": canonical_cbor_b64,
        "client_sign": {
            "prepare_path": format!("/api/pending/{assigned_id}"),
            "submit_path": "/api/sign-callback",
            "alg": "COSE_Sign1 / Ed25519 (alg -8); kid = signer pubkey",
            "payload": "the base64-decoded canonical_cbor_b64 (sign these exact bytes)",
            "submit_body": {
                "correlation_id": assigned_id,
                "cose_signed_bytes": "<base64 of your COSE_Sign1 envelope>",
                "signer_pubkey": "<base58 of the Ed25519 pubkey that signed; must equal the COSE kid>"
            }
        },
        "next_step": format!(
            "Two ways to finish (the memory is client-signed either way): \
             (A) Programmatic — COSE_Sign1 the bytes in canonical_cbor_b64 with \
             your Ed25519 identity key and POST {{correlation_id, \
             cose_signed_bytes, signer_pubkey}} to client_sign.submit_path \
             (/api/sign-callback). (B) Browser — tell the user to open \
             approve_url and click Approve. Then call mnemonic_check_pending \
             with correlation_id={assigned_id} to retrieve the on-chain \
             solana_tx + arweave_tx."
        ),
    }))
}

/// `mnemonic_check_pending` — resolve a deferred-sign correlation_id to its
/// final on-chain state once the user has approved the COSE envelope in the
/// browser. Returns one of:
///
///   - `{status: "signed", attestation_id, content_hash, solana_tx,
///     arweave_tx, signer_pubkey, signed_at, solana_explorer_url,
///     arweave_url}` — sign-callback completed, row persisted.
///   - `{status: "awaiting_signature", correlation_id, expires_at}` —
///     bundle still parked in the LRU; user has not yet approved.
///   - `{status: "not_found", correlation_id}` — never issued, expired
///     past TTL without sign, or already consumed and never persisted
///     (rare — implies a sign-callback failure).
///
/// Capability auth: `correlation_id` is the only credential. Same model as
/// `/api/sign-callback` — the signed bytes are content-addressed via
/// blake3, so leaking the routing token does not enable forgery.
pub async fn check_pending(
    pending: &PendingBundles,
    store: &std::sync::Mutex<SqliteStore>,
    correlation_id: &str,
) -> serde_json::Value {
    // 1. DB lookup first — happy path is "row already persisted".
    let signed = {
        let store_g = match store.lock() {
            Ok(g) => g,
            Err(_) => {
                return serde_json::json!({
                    "status": "error",
                    "message": "store mutex poisoned",
                    "correlation_id": correlation_id,
                });
            }
        };
        store_g
            .find_by_correlation_id(correlation_id)
            .ok()
            .flatten()
    };
    if let Some((attestation_id, content_hash, solana_tx, arweave_tx, signer_pubkey, created_at)) =
        signed
    {
        let solana_explorer_url = if solana_tx.starts_with("local:") {
            String::new()
        } else {
            format!("https://solscan.io/tx/{solana_tx}")
        };
        let arweave_url = if arweave_tx.starts_with("local:") {
            String::new()
        } else {
            format!("https://gateway.irys.xyz/{arweave_tx}")
        };
        return serde_json::json!({
            "status": "signed",
            "attestation_id": attestation_id,
            "content_hash": content_hash,
            "solana_tx": solana_tx,
            "arweave_tx": arweave_tx,
            "signer_pubkey": signer_pubkey,
            "signed_at": created_at,
            "solana_explorer_url": solana_explorer_url,
            "arweave_url": arweave_url,
        });
    }

    // 2. Pending LRU — bundle parked, awaiting user approval.
    match pending.peek_by_id(correlation_id).await {
        Ok(entry) => serde_json::json!({
            "status": "awaiting_signature",
            "correlation_id": correlation_id,
            "expires_at": entry.exp.to_rfc3339(),
            "hint": "User has not clicked Approve yet. Poll again in a few seconds.",
        }),
        Err(_) => serde_json::json!({
            "status": "not_found",
            "correlation_id": correlation_id,
            "hint": "Either the correlation_id was never issued, the 5-minute TTL elapsed without user approval, or the sign-callback failed mid-write. Re-issue mnemonic_sign_memory if you want a fresh bundle.",
        }),
    }
}

/// Stdio branch — inline server-side signing (Decision 4 single-tenant flow).
///
/// T2 changes (routing now driven by per-request `write_mode`, not the
/// operator's `STORAGE_MODE` env-var):
///
/// - The `write_mode` parameter replaces `storage_mode` as the routing
///   decision. `WriteMode::Local` → synthetic-id no-anchor path
///   regardless of env-var. `WriteMode::Participate` → real Arweave +
///   Solana writes regardless of env-var (the paywall gate in
///   `mcp_handler` has already ensured the deploy supports it).
/// - `storage_mode` is retained ONLY for the legacy whoami-echo field in
///   the success envelope. It does NOT influence behaviour anymore — the
///   chrome-extension and other legacy clients that read `storage_mode`
///   from the response keep working byte-for-byte because the resolver
///   maps `None` (no `mode` field) to env-var fallback, producing the same
///   `WriteMode` value the env-var would have selected.
///
/// T3 changes (delivery guarantee on participate):
///
/// - After `solana.write_memo` returns, the participate path re-fetches the
///   COSE bytes from Arweave with an exponential-backoff loop capped by
///   `delivery_refetch_timeout`, then runs `verify_cose` over the re-fetched
///   bytes, then runs an in-process recall against `content_hash` and
///   confirms our `attestation_id` is in the result. On any failure (refetch
///   budget exhausted, verify mismatch, recall miss) the row is persisted
///   with `WriteMode::Local` — so the embed + signature aren't wasted —
///   and the function returns `ToolError::TypedRpc(delivery_not_confirmed)`.
///   `mcp_handler` consumes the typed error to drive refund + counter
///   bookkeeping (api_key is only available at the dispatcher boundary).
/// - On success the row is persisted with `WriteMode::Participate` and the
///   success envelope gains `delivery_receipt { arweave_tx, solana_tx,
///   recall_verified_at }`. `recall_verified_at` is operator-attested per
///   the tech-spec's trust-model note; the cryptographically verifiable
///   timestamp is the Solana memo's `block_time`.
/// - Critical-section discipline (Decision 8): two short scoped locks. The
///   success branch takes the SQLite mutex for save_attestation +
///   record_attestation_cost and drops it. The failure branch takes the
///   SQLite mutex for save_attestation(Local) and drops it BEFORE returning
///   the typed error. No `.await` is held while either lock is in scope.
///
/// The participate-on-local-only short-circuit lives in `sign_memory` (the
/// public entry point) — fires before deferred-vs-inline branching so the
/// user gets the typed error regardless of path. `sign_memory_inline` does
/// NOT take the envelope.
#[allow(clippy::too_many_arguments)]
async fn sign_memory_inline(
    keypair: &Keypair,
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    content: &str,
    tags: &[String],
    cost_hint: &CostHint,
    storage_mode: &str,
    owner_pubkey: &str,
    write_mode: WriteMode,
    visibility: Visibility,
    delivery_refetch_timeout: Duration,
) -> Result<serde_json::Value, ToolError> {
    let pubkey = identity::pubkey_base58(keypair);
    // Wave 3 invariant (defense in depth): inline signing uses the operator's
    // `keypair` to produce the COSE_Sign1, so it is only legitimate when the
    // memory is authored BY the operator — i.e. `owner_pubkey == pubkey`.
    // The dispatcher in `sign_memory` already routes any remote-owned write to
    // the client-signing path; this guard guarantees no future caller can
    // smuggle a remote owner into the operator-signed path (custodial forgery).
    if owner_pubkey != pubkey {
        return Err(ToolError::Other(anyhow::anyhow!(
            "refusing to operator-sign a memory owned by a different identity \
             (owner={owner_pubkey}, operator={pubkey}); remote writes must be \
             client-signed via the deferred path"
        )));
    }
    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Embed content. The `Embedder` trait is infallible by design; the
    // production code path treats an empty vector as the failure signal
    // (model file missing, ONNX crash, etc.) per Task 4's note in
    // agent-native-distribution tech-spec. Surface as typed
    // `-32098 EmbedderInvalid` so the agent can branch on the structured
    // error rather than parsing a free-text message. `fallback_available`
    // is `true` — the caller can retry with
    // `allow_fallback_to_participate=true` to proxy through the hosted
    // endpoint (Decision 4).
    let embedding = embedder.embed(content);
    if embedding.is_empty() {
        return Err(ToolError::TypedRpc(crate::mcp::embedder_invalid(
            "embedder returned empty vector",
            "Verify the local embedder model file is present and uncorrupted; \
             reinstall the binary if the integrity check fails.",
            true,
        )));
    }

    // 2. Compress with TurboQuant
    let compressed = compressor.compress(&embedding);
    let compressed_bytes = compressed.to_bytes();

    // 3. Build artifact JSON for CBOR canonicalization
    let artifact = serde_json::json!({
        "artifact_id": attestation_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": identity::did_sol(keypair),
        "created_at": now,
        "tags": tags,
        "metadata": {
            "embed_provider": embedder.provider_name(),
            "embed_dim": embedder.dim(),
            "turbo_bits": compressed.bit_width,
            "embedding_compressed": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &compressed_bytes,
            ),
        },
    });

    // 4. Sign with COSE_Sign1 (canonical CBOR → blake3 → Ed25519)
    let signed = sign_artifact(&artifact, &schema::MEMORY_V1, keypair)
        .map_err(|e| anyhow::anyhow!("COSE signing failed: {e}"))?;

    let content_hash = signed.content_hash.clone();
    let embed_model = embedder.model_id().to_string();

    // 5. Store on-chain (or locally) — routed by per-request `write_mode`,
    //    not the operator's `STORAGE_MODE`. A `local` request against a
    //    `STORAGE_MODE=full` deploy stays free (no Arweave/Solana writes).
    let (solana_tx, arweave_tx) = match write_mode {
        WriteMode::Local => {
            let local_ar = format!("local:{}", &attestation_id[..8]);
            let local_sol = format!("local:{}", &content_hash[..16]);
            (local_sol, local_ar)
        }
        WriteMode::Participate => {
            // Arweave: store COSE_Sign1 bytes (not raw JSON). `Producer` /
            // `Created-At` tags mirror fields already public inside the
            // payload; they make the item aggregatable via a single gateway
            // GraphQL query (recover-traction-from-chain) with no payload
            // fetch — the DB-loss recovery path depends on them.
            let producer_did = identity::did_sol(keypair);
            let ar_tx = arweave
                .write_item(
                    &signed.cose_bytes,
                    keypair,
                    &[
                        ("Producer", producer_did.as_str()),
                        ("Created-At", now.as_str()),
                    ],
                )
                .await?;
            arweave.mine().await?;

            // Solana: anchor blake3 hash + embedding model (v3 format)
            let memo = serde_json::json!({
                "h": content_hash,
                "a": ar_tx,
                "m": embed_model,
                "v": 3,
            });
            let sol_tx = solana.write_memo(keypair, &memo.to_string()).await?;
            (sol_tx, ar_tx)
        }
    };

    // 6a. Save row immediately after chain anchor.
    //
    // Two critical reasons for saving BEFORE the delivery check:
    //   (1) The in-process recall stage of the delivery check reads from
    //       SQLite — the row must exist by the time we query for it.
    //   (2) On delivery failure we re-save with `WriteMode::Local`
    //       (INSERT OR REPLACE) so the embed + signature aren't wasted
    //       even though the chain anchor isn't proved retrievable.
    //
    // ONE short critical section: take the SQLite mutex, write the
    // attestation row, drop the mutex. No `.await` while held (Decision 8).
    {
        let store = store.lock().unwrap();
        // T2: the persisted `write_mode` column is the SAME value the
        // paywall gate consulted (single source of truth — Decision 1).
        // Visibility is threaded from the resolver in `handle_tool_call`
        // (Task 4 / Decision 3+5). For `write_mode == Local` the resolver
        // has already rejected any explicit visibility request via AC14,
        // so we expect `Visibility::Private` here; for participate writes
        // the resolved value (`Private` default or `Public` after the
        // public-write ceremony) flows through verbatim.
        store.save_attestation(
            &attestation_id,
            content,
            &content_hash,
            tags,
            &solana_tx,
            &arweave_tx,
            &pubkey,
            owner_pubkey,
            &now,
            write_mode,
            visibility,
            &embedding,
        )?;
    }

    // 6b. Delivery confirmation — Participate ONLY. T3 (modes-user-choice).
    //
    // We just successfully wrote to Arweave + Solana AND persisted the row.
    // Before claiming "delivered" we must prove the chain bytes are
    // *retrievable*. Three checks, any failure demotes the row to `Local`
    // (via INSERT OR REPLACE under the same attestation_id) and returns
    // the typed error (refund handled by mcp_handler since api_key only
    // lives there).
    //
    // The `recall_verified_at` timestamp is captured at the moment the
    // round-trip passes; surfaced in the success envelope under
    // `delivery_receipt.recall_verified_at`. Operator-attested per the
    // tech-spec trust-model note.
    //
    // Round-2: the delivery + demote logic lives in
    // `confirm_delivery_or_demote` and is shared with the deferred-path
    // (`api::sign_callback_handler`). Same primitives, same behaviour,
    // one code path.
    let recall_verified_at: Option<String> = if write_mode == WriteMode::Participate {
        let ctx = DeliveryContext {
            arweave,
            store,
            timeout: delivery_refetch_timeout,
            attestation_id: &attestation_id,
            content,
            content_hash: &content_hash,
            tags,
            solana_tx: &solana_tx,
            arweave_tx: &arweave_tx,
            signer_pubkey: &pubkey,
            owner_pubkey,
            created_at: &now,
            embedding: &embedding,
        };
        match confirm_delivery_or_demote(ctx).await? {
            DeliveryOutcome::Confirmed { recall_verified_at } => Some(recall_verified_at),
            DeliveryOutcome::Demoted { stage } => {
                return Err(ToolError::TypedRpc(delivery_not_confirmed(
                    stage,
                    &arweave_tx,
                    &solana_tx,
                    &attestation_id,
                )));
            }
        }
    } else {
        None
    };

    // 6c. Cost recording — Participate-success ONLY. A `Local` request
    // can hit this code path against a `STORAGE_MODE=full +
    // PAYMENT_MODE=x402` server and MUST NOT produce an
    // `attestation_costs` row — that would charge the caller for a free
    // path. Also fires AFTER the delivery check passes (on a demotion we
    // return before reaching here).
    if write_mode == WriteMode::Participate {
        let store = store.lock().unwrap();
        let _ = payment::record_attestation_cost(
            &store,
            &attestation_id,
            cost_hint.irys_lamports,
            cost_hint.sol_tx_fee_lamports,
            cost_hint.sol_price_usdc,
            cost_hint.charge_micro_usdc,
        );
    }

    let ratio = compressor.compression_ratio();
    let mut out = serde_json::json!({
        "attestation_id": attestation_id,
        "content_hash": content_hash,
        "hash_algorithm": "blake3",
        "encoding": "cbor+cose",
        "solana_tx": solana_tx,
        "arweave_tx": arweave_tx,
        "signer": pubkey,
        "did_sol": identity::did_sol(keypair),
        "timestamp": now,
        "storage_mode": storage_mode,
        "write_mode": write_mode.as_str(),
        "visibility": visibility.as_str(),
        "embedding": {
            "model": embed_model,
            "provider": embedder.provider_name(),
            "dim": embedder.dim(),
            "verifiable": embedder.is_open_weights(),
        },
        "compression": {
            "algorithm": "TurboQuant",
            "bits": compressed.bit_width,
            "ratio": format!("{ratio:.1}x"),
            "original_bytes": embedding.len() * 4,
            "compressed_bytes": compressed_bytes.len(),
        },
    });

    // Participate success envelope addition — T3. `delivery_receipt`
    // documents the delivery proof: the chain tx ids plus the
    // operator-attested timestamp of the successful read-back.
    if let Some(ts) = recall_verified_at {
        // Snapshot the tx ids into owned strings BEFORE we obtain the
        // mutable borrow on `out`'s object map (avoids E0502 — `obj.insert`
        // would otherwise hold a mutable borrow while `out[...]` re-borrows
        // immutably).
        let receipt = serde_json::json!({
            "arweave_tx": arweave_tx,
            "solana_tx": solana_tx,
            "recall_verified_at": ts,
        });
        if let Some(obj) = out.as_object_mut() {
            obj.insert("delivery_receipt".to_string(), receipt);
        }
    }
    Ok(out)
}

/// Run the post-anchor delivery-confirmation pipeline for a participate
/// write. Three sequential checks; the first failure short-circuits with
/// the stage label suitable for the typed error envelope.
///
/// 1. **Refetch.** Pull the anchored COSE bytes back from Arweave with an
///    exponential-backoff retry capped by `timeout`. Catches the "anchor
///    accepted but bytes not retrievable" silent failure (Arweave's
///    eventual-consistency window).
/// 2. **Verify.** Run `verify_cose` over the re-fetched bytes with the
///    expected hash + tx ids. Catches tampering between write and read
///    (incl. operator-side adversary in shared-tenant deployments) and
///    catches the case where some other key signed the bytes.
/// 3. **Recall.** Confirm "we can read back the row we just wrote" via a
///    primary-key existence check scoped to the owner pubkey. Catches DB
///    write loss between `save_attestation` and the delivery check
///    completing (rare but possible if a concurrent tx in another mcp
///    process rolls back our write, or in the deferred-signing path if
///    `set_correlation_id` somehow drops the row).
///
///    NB (round-2 fix): the round-1 implementation here ran a cosine
///    similarity search using `embedder.embed(content_hash)` as the query
///    vector. That worked under `StubEmbedder` (constant vector → all
///    rows are neighbours) but produces no semantic match for real
///    embedders like `FastEmbedder`/`OpenAIEmbedder` — recall would miss
///    the target row in any non-trivial corpus. The check's *purpose* is
///    "can we read it back", which is a database existence question, not
///    a semantic-search question.
///
/// Pure async — no SQLite lock held across `.await`. The caller's lock
/// scopes (save_attestation in the success/failure branches) sit OUTSIDE
/// this function entirely.
#[allow(clippy::too_many_arguments)]
pub async fn perform_delivery_check(
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    arweave_tx: &str,
    solana_tx: &str,
    content_hash: &str,
    attestation_id: &str,
    owner_pubkey: &str,
    timeout: Duration,
) -> Result<(), &'static str> {
    // Stage 1: refetch the anchored bytes within a wall-clock budget.
    let refetched = match arweave_refetch_with_budget(arweave, arweave_tx, timeout).await {
        Ok(bytes) => bytes,
        Err(_) => return Err("refetch"),
    };

    // Stage 2: COSE verify the re-fetched bytes against the expected
    // content hash + the chain tx ids. `verify_cose` returns a JSON Value;
    // `status == "verified"` is the only pass condition. Anything else
    // (`"tampered"`, hash mismatch, decoder error) is a fail.
    let verify_result =
        match verify_cose(&refetched, Some(content_hash), Some(solana_tx), arweave_tx) {
            Ok(v) => v,
            Err(_) => return Err("verify"),
        };
    if verify_result["status"].as_str() != Some("verified") {
        return Err("verify");
    }

    // Stage 3: primary-key existence check scoped to owner_pubkey. Brief
    // SQLite lock; no `.await` while held. Owner scoping preserves the
    // tenant-isolation invariant (Decision 9 / T4) — even though we are
    // looking up by the row's own attestation_id, a future change that
    // dropped owner from the predicate would let any caller probe rows
    // by attestation_id; the explicit AND defends in depth.
    let exists: bool = {
        let store_g = store.lock().unwrap();
        let conn = store_g.conn();
        conn.query_row(
            "SELECT 1 FROM attestations WHERE attestation_id = ? AND owner_pubkey = ?",
            rusqlite::params![attestation_id, owner_pubkey],
            |_| Ok(true),
        )
        .unwrap_or(false)
    };
    if !exists {
        return Err("recall");
    }
    Ok(())
}

/// Re-fetch `arweave_tx` from Arweave with exponential backoff bounded by
/// `total_budget`. Returns the raw bytes on success or `Err` on either a
/// non-retryable read error OR budget exhaustion (whichever comes first).
///
/// Backoff schedule: 200ms → 400ms → 800ms → 1600ms → 2000ms (capped at
/// `MAX_BACKOFF`). The retry count is *not* fixed; the loop stops when
/// the next sleep would push the elapsed time past `total_budget`. This
/// keeps the wall-clock contract simple: the function returns to the
/// caller no later than `total_budget + one slow read` after it was
/// called.
///
/// Sized against Arweave's documented eventual-consistency window (seconds
/// to low tens of seconds); see tech-spec §"Risk & mitigations / DoS
/// amplification" mitigation (i).
async fn arweave_refetch_with_budget(
    arweave: &ArweaveClient,
    tx: &str,
    total_budget: Duration,
) -> anyhow::Result<Vec<u8>> {
    const INITIAL_DELAY: Duration = Duration::from_millis(200);
    const MAX_BACKOFF: Duration = Duration::from_secs(2);
    const FACTOR: u32 = 2;

    let start = std::time::Instant::now();
    let mut delay = INITIAL_DELAY;
    let mut attempt: u32 = 0;
    // Initialized inside the loop — the `None` placeholder satisfies the
    // unwrap_or at the end in the (impossible) case where we exit before
    // any read attempt.
    #[allow(unused_assignments)]
    let mut last_err: Option<anyhow::Error> = None;
    loop {
        attempt += 1;
        match arweave.read(tx).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                tracing::debug!(
                    arweave_tx = %tx,
                    attempt,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    error = %e,
                    "arweave_refetch_with_budget: read failed, retrying"
                );
                last_err = Some(e);
            }
        }

        // Decide whether another retry fits in the budget. If the next
        // sleep would push us past, give up now.
        let elapsed = start.elapsed();
        if elapsed >= total_budget {
            break;
        }
        let remaining = total_budget - elapsed;
        if delay > remaining {
            // One last partial sleep to use up the budget, then bail.
            tokio::time::sleep(remaining).await;
            // Final attempt before returning the timeout error.
            match arweave.read(tx).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    last_err = Some(e);
                }
            }
            break;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * FACTOR).min(MAX_BACKOFF);
    }
    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("arweave_refetch_with_budget: budget exhausted with no error captured")
    }))
}

/// Inputs to [`confirm_delivery_or_demote`]. The struct keeps the call-sites
/// (inline `sign_memory_inline` AND deferred `api::sign_callback_handler`)
/// reading like declarations rather than 11-argument soup.
pub struct DeliveryContext<'a> {
    pub arweave: &'a ArweaveClient,
    pub store: &'a std::sync::Mutex<SqliteStore>,
    pub timeout: Duration,
    pub attestation_id: &'a str,
    pub content: &'a str,
    pub content_hash: &'a str,
    pub tags: &'a [String],
    pub solana_tx: &'a str,
    pub arweave_tx: &'a str,
    pub signer_pubkey: &'a str,
    pub owner_pubkey: &'a str,
    pub created_at: &'a str,
    pub embedding: &'a [f32],
}

/// Outcome of [`confirm_delivery_or_demote`].
pub enum DeliveryOutcome {
    /// All three stages passed. Caller persists cost (Participate) and
    /// emits the success envelope including `delivery_receipt`.
    Confirmed { recall_verified_at: String },
    /// One of the stages failed. Caller emits the typed `-32011
    /// DeliveryNotConfirmed` (or HTTP equivalent for the deferred path).
    /// The row has already been demoted to `WriteMode::Local` via
    /// `INSERT OR REPLACE` so the embed + signature aren't wasted.
    Demoted { stage: &'static str },
}

/// Single shared delivery-confirmation helper used by BOTH the inline
/// `sign_memory_inline` path AND the deferred `sign_callback_handler`
/// path. Performs the three-stage check (refetch → verify_cose →
/// primary-key recall) and, on failure, demotes the row in place to
/// `WriteMode::Local` so caller logic (refund / counter / typed-error
/// emission) is identical across both code paths.
///
/// Critical-section discipline (Decision 8): the SQLite mutex is taken
/// only inside `perform_delivery_check`'s primary-key existence check
/// AND inside this function's failure-branch `save_attestation(Local)`
/// call. Both are short, sync, and drop the lock before any `.await`.
/// No mutex is held across the network calls.
///
/// **Pre-condition:** the caller MUST have already persisted the row with
/// `WriteMode::Participate` BEFORE calling this. The recall stage performs
/// a primary-key existence check; if the row isn't there yet, the helper
/// will report stage = "recall" and demote (which on a fresh-row scenario
/// has no row to overwrite, leading to a confusing demotion-of-nothing).
pub async fn confirm_delivery_or_demote(
    ctx: DeliveryContext<'_>,
) -> anyhow::Result<DeliveryOutcome> {
    match perform_delivery_check(
        ctx.arweave,
        ctx.store,
        ctx.arweave_tx,
        ctx.solana_tx,
        ctx.content_hash,
        ctx.attestation_id,
        ctx.owner_pubkey,
        ctx.timeout,
    )
    .await
    {
        Ok(()) => Ok(DeliveryOutcome::Confirmed {
            recall_verified_at: chrono::Utc::now().to_rfc3339(),
        }),
        Err(stage) => {
            // Demote in place via INSERT OR REPLACE under the same
            // attestation_id. Short critical section, no `.await` while
            // held.
            {
                let store = ctx.store.lock().unwrap();
                // Demoted local rows are always private — `Visibility` is a
                // participate-only concept (AC14). Even if the original
                // participate write had `visibility=public`, demotion strips
                // it: a local row can't be anonymously discoverable.
                store.save_attestation(
                    ctx.attestation_id,
                    ctx.content,
                    ctx.content_hash,
                    ctx.tags,
                    ctx.solana_tx,
                    ctx.arweave_tx,
                    ctx.signer_pubkey,
                    ctx.owner_pubkey,
                    ctx.created_at,
                    WriteMode::Local,
                    Visibility::Private,
                    ctx.embedding,
                )?;
            }
            tracing::warn!(
                attestation_id = %ctx.attestation_id,
                arweave_tx = %ctx.arweave_tx,
                solana_tx = %ctx.solana_tx,
                stage = %stage,
                owner_pubkey = %ctx.owner_pubkey,
                "delivery not confirmed — row demoted to local"
            );
            Ok(DeliveryOutcome::Demoted { stage })
        }
    }
}

/// Tool 3: verify
///
/// Routes by the row's stored `write_mode` (Decision 9 / T4), not by env-var:
/// - `WriteMode::Local`  → `verify_local` (SQLite lookup + blake3 recompute).
/// - `WriteMode::Participate` → fetch COSE bytes from Arweave → COSE verify →
///   compare hash with the Solana anchor (`verify_cose` / `verify_legacy_json`
///   fallback for v1 rows).
///
/// Tenant isolation: the routing lookup is scoped by the caller's
/// `owner_pubkey`. A row owned by a different tenant returns the
/// `not_found` shape identical to a genuine miss — no `content_hash`,
/// `signer_pubkey`, or content preview leaks across tenants.
///
/// `storage_mode` is _unused — routing is by stored `write_mode`_. It is
/// kept in the signature for ABI compatibility with internal callers that
/// pre-date the routing change.
#[allow(clippy::too_many_arguments)]
pub async fn verify(
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    solana_tx: Option<&str>,
    arweave_tx: Option<&str>,
    owner_pubkey: &str,
    _storage_mode: &str,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
) -> anyhow::Result<serde_json::Value> {
    let lookup_id = match solana_tx.or(arweave_tx) {
        Some(id) => id,
        None => {
            return Ok(serde_json::json!({
                "status": "error",
                "message": "Provide solana_tx or arweave_tx",
            }));
        }
    };

    // Storage lock discipline: SqliteStore is !Send. Hold the mutex
    // briefly for the routing lookup and DROP before any `.await` on
    // Arweave / Solana clients.
    let routed_mode = {
        let store = store.lock().expect("store mutex poisoned");
        store.find_write_mode_by_tx(lookup_id, owner_pubkey)?
    };

    match routed_mode {
        Some(WriteMode::Local) => verify_local(
            store,
            solana_tx,
            arweave_tx,
            owner_pubkey,
            embedder,
            compressor,
        ),
        Some(WriteMode::Participate) => {
            verify_participate(solana, arweave, solana_tx, arweave_tx).await
        }
        // Tenant isolation: a row owned by a different tenant returns
        // `Ok(None)` from `find_write_mode_by_tx` — same shape as a
        // genuine miss. NO `content_hash`, `signer_pubkey`, `content`,
        // or `preview` is included; the response is indistinguishable
        // from "tx doesn't exist anywhere in the DB".
        None => Ok(serde_json::json!({
            "status": "not_found",
            "lookup_id": lookup_id,
        })),
    }
}

/// Participate-mode verification: fetch COSE bytes from Arweave, verify the
/// COSE signature, compare blake3 hash against the Solana anchor. Extracted
/// from the pre-T4 env-var branch so the routing decision in `verify`
/// remains a flat `match`.
async fn verify_participate(
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    solana_tx: Option<&str>,
    arweave_tx: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let mut expected_hash: Option<String> = None;
    let mut ar_tx = arweave_tx.map(|s| s.to_string());
    let mut anchor_version: u64 = 1;

    if let Some(sol_tx) = solana_tx {
        match solana.read_memo(sol_tx).await? {
            Some(memo) => {
                expected_hash = memo["h"].as_str().map(|s| s.to_string());
                if ar_tx.is_none() {
                    ar_tx = memo["a"].as_str().map(|s| s.to_string());
                }
                anchor_version = memo["v"].as_u64().unwrap_or(1);
            }
            None => {
                return Ok(serde_json::json!({"status": "anchor_not_found", "solana_tx": sol_tx}))
            }
        }
    }

    let ar_tx_id = ar_tx.as_deref().unwrap_or("");
    let raw_bytes = match arweave.read(ar_tx_id).await {
        Ok(b) => b,
        Err(_) => {
            return Ok(serde_json::json!({"status": "arweave_not_found", "arweave_tx": ar_tx_id}))
        }
    };

    // Detect artifact format:
    // - If anchor_version >= 2 from Solana memo → COSE
    // - If no Solana anchor but payload looks like COSE (CBOR array tag 0x84) → try COSE
    // - Otherwise → legacy JSON + SHA-256
    let is_cose = anchor_version >= 2 || (solana_tx.is_none() && looks_like_cose(&raw_bytes));

    if is_cose {
        return verify_cose(&raw_bytes, expected_hash.as_deref(), solana_tx, ar_tx_id);
    }

    // v1 artifacts (legacy): raw JSON + SHA-256
    verify_legacy_json(&raw_bytes, expected_hash.as_deref(), solana_tx, ar_tx_id)
}

/// Heuristic: COSE_Sign1 is a CBOR 4-element array.
/// CBOR array of 4 items starts with byte 0x84.
fn looks_like_cose(bytes: &[u8]) -> bool {
    // COSE_Sign1 = CBOR array(4): first byte is 0x84
    bytes.first() == Some(&0x84)
}

/// Verify a v2 COSE_Sign1 artifact from Arweave.
fn verify_cose(
    cose_bytes: &[u8],
    expected_hash: Option<&str>,
    solana_tx: Option<&str>,
    arweave_tx: &str,
) -> anyhow::Result<serde_json::Value> {
    let result = cose_verify(cose_bytes, expected_hash)
        .map_err(|e| anyhow::anyhow!("COSE verification failed: {e}"))?;

    // Try to recover content preview from the CBOR payload
    let content_preview = from_canonical_cbor(&result.payload)
        .ok()
        .and_then(|json| {
            json["content"]
                .as_str()
                .map(|s| s[..s.len().min(200)].to_string())
        })
        .unwrap_or_default();

    Ok(serde_json::json!({
        "status": if result.valid { "verified" } else { "tampered" },
        "encoding": "cbor+cose",
        "checks": {
            "content_integrity": result.content_integrity,
            "cose_signature": result.cose_signature,
            "algorithm_valid": result.algorithm_valid,
        },
        "content_hash": result.content_hash,
        "hash_algorithm": "blake3",
        "solana_tx": solana_tx.unwrap_or(""),
        "arweave_tx": arweave_tx,
        "signer": result.signer,
        "content_preview": content_preview,
    }))
}

/// Verify a v1 legacy artifact (raw JSON + SHA-256).
fn verify_legacy_json(
    raw_bytes: &[u8],
    expected_hash: Option<&str>,
    solana_tx: Option<&str>,
    arweave_tx: &str,
) -> anyhow::Result<serde_json::Value> {
    use sha2::{Digest, Sha256};

    let payload: serde_json::Value = serde_json::from_slice(raw_bytes).unwrap_or_default();
    let content = payload["content"].as_str().unwrap_or("");
    let actual_hash = hex::encode(Sha256::digest(content.as_bytes()));

    if let Some(expected) = expected_hash {
        if actual_hash == expected {
            return Ok(serde_json::json!({
                "status": "verified",
                "encoding": "json+sha256 (legacy v1)",
                "content_hash": actual_hash,
                "hash_algorithm": "sha256",
                "solana_tx": solana_tx.unwrap_or(""),
                "arweave_tx": arweave_tx,
                "signer": payload["signer"].as_str().unwrap_or(""),
                "content_preview": &content[..content.len().min(200)],
            }));
        }
        return Ok(serde_json::json!({
            "status": "tampered",
            "encoding": "json+sha256 (legacy v1)",
            "expected_hash": expected,
            "actual_hash": actual_hash,
        }));
    }

    Ok(serde_json::json!({"status": "hash_computed", "content_hash": actual_hash}))
}

/// Local-mode verification: rebuild the canonical CBOR artifact and compare
/// its blake3 against the stored `content_hash`.
///
/// Local rows keep only the *result* of signing — `content_hash` — and discard
/// the canonical CBOR and the COSE envelope that produced it. So the check has
/// to rebuild the artifact exactly as `sign_memory_inline` built it, over the
/// same `MEMORY_V1` field order, and hash that.
///
/// The one non-obvious input is `metadata.embedding_compressed`. It is not
/// stored, but it does not need to be: TurboQuant is a pure function of
/// `(dim, bit_width, seed, embedding)`, and the raw embedding *is* stored, so
/// re-compressing it reproduces those bytes exactly.
///
/// Two inputs are not recoverable from the row and are taken from the running
/// server: `embed_provider` (from `embedder`) and `bit_width`/`seed` (from
/// `compressor`). A row written under a different embedding provider or
/// `TURBO_BITS` therefore rebuilds to a different hash. That is
/// indistinguishable from tampering here, so the mismatch branch names the
/// assumptions instead of asserting foul play — a false accusation is worse
/// than an inconclusive answer.
///
/// `owner_pubkey` scopes the lookup so a tenant cannot probe another
/// tenant's row via `verify`. The wrapping `verify()` already routed here
/// because `find_write_mode_by_tx` returned `Some(Local)` under this
/// scope; we re-apply the predicate defensively so direct callsites
/// inherit the same isolation guarantee.
fn verify_local(
    store: &std::sync::Mutex<SqliteStore>,
    solana_tx: Option<&str>,
    arweave_tx: Option<&str>,
    owner_pubkey: &str,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
) -> anyhow::Result<serde_json::Value> {
    let lookup_id = solana_tx
        .or(arweave_tx)
        .ok_or_else(|| anyhow::anyhow!("provide solana_tx or arweave_tx"))?;

    let row = {
        let store = store.lock().expect("store mutex poisoned");
        store.reconstruction_inputs_by_tx(lookup_id, owner_pubkey)?
    };

    let Some(row) = row else {
        // Tenant-isolation parity (T4 round-1 security finding, CWE-203):
        // the `not_found` shape must match the top-level routing-miss
        // shape exactly. Including `storage_mode: "local"` here would
        // distinguish "row belongs to another local tenant" from "row
        // doesn't exist anywhere", giving an attacker an existence oracle.
        return Ok(serde_json::json!({
            "status": "not_found",
            "lookup_id": lookup_id,
        }));
    };

    let recomputed = rebuild_content_hash(&row, embedder, compressor);

    // Legacy fallback: pre-CBOR v1 rows stored sha256 over the bare content.
    let legacy_match = || {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(row.content.as_bytes())) == row.content_hash
    };

    if recomputed.as_deref() == Some(row.content_hash.as_str()) || legacy_match() {
        return Ok(serde_json::json!({
            "status": "verified",
            "storage_mode": "local",
            "content_hash": row.content_hash,
            "solana_tx": row.solana_tx,
            "arweave_tx": row.arweave_tx,
            "signer": row.signer_pubkey,
            "content_preview": &row.content[..row.content.len().min(200)],
            "checks": {
                "content_integrity": true,
                "artifact_reconstructed": recomputed.is_some(),
            },
            "note": "local mode rebuilds the canonical CBOR artifact and checks its blake3 \
                     against the stored content_hash; the COSE signature itself is not \
                     retained for local rows, so this proves integrity, not authorship",
        }));
    }

    Ok(serde_json::json!({
        "status": "tampered",
        "storage_mode": "local",
        "expected_hash": row.content_hash,
        "actual_content_hash": recomputed,
        "note": "rebuilt artifact hash does not match the stored content_hash. This means \
                 the row was modified — or that it was written under a different \
                 embed_provider/TURBO_BITS than this server runs, since those two inputs \
                 are not stored per row and are assumed from the running config.",
        "assumed": {
            "embed_provider": embedder.provider_name(),
            "turbo_bits": compressor.bit_width(),
            "embed_dim": row.embedding.len(),
        },
    }))
}

/// Rebuild `blake3(canonical_cbor(artifact))` for a stored row.
///
/// Mirrors the artifact construction in `sign_memory_inline` field for field;
/// the two must stay in lockstep or every local `verify` reports a mismatch.
/// Returns `None` if the row carries no embedding (nothing to re-compress) or
/// if CBOR encoding fails.
fn rebuild_content_hash(
    row: &mnemonic_core::storage::ReconstructionInputs,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
) -> Option<String> {
    if row.embedding.is_empty() {
        return None;
    }

    // Match the stored vector's width rather than the server's configured
    // dim, so a row written before a dim change still rebuilds.
    let dim = row.embedding.len();
    let sized;
    let compressor = if compressor.dim() == dim {
        compressor
    } else {
        sized = EmbeddingCompressor::new(dim, compressor.bit_width(), compressor.seed());
        &sized
    };
    let compressed = compressor.compress(&row.embedding);

    let artifact = serde_json::json!({
        "artifact_id": row.attestation_id,
        "type": "memory",
        "schema_version": 1,
        "content": row.content,
        "producer": format!("did:sol:{}", row.signer_pubkey),
        "created_at": row.created_at,
        "tags": row.tags,
        "metadata": {
            "embed_provider": embedder.provider_name(),
            "embed_dim": dim,
            "turbo_bits": compressed.bit_width,
            "embedding_compressed": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                compressed.to_bytes(),
            ),
        },
    });

    to_canonical_cbor(&artifact, &schema::MEMORY_V1)
        .ok()
        .map(|cbor| blake3_hash(&cbor))
}

/// Tool 4: prove_identity (sync — pure crypto)
pub fn prove_identity(keypair: &Keypair, challenge: &str) -> serde_json::Value {
    let sig = identity::sign_bytes(keypair, challenge.as_bytes());
    serde_json::json!({
        "public_key": identity::pubkey_base58(keypair),
        "did_sol": identity::did_sol(keypair),
        "challenge": challenge,
        "signature": hex::encode(&sig),
        "algorithm": "Ed25519",
    })
}

/// Tool 5: recall (sync — DB search)
///
/// `owner_pubkey` (Decision 9) is the mandatory tenant scope. HTTP transport
/// resolves it from the JWT subject; stdio transport passes the local
/// keypair pubkey. `keypair` remains in the signature for the `total_attestations`
/// count (per-signer, distinct from per-owner search) and forward
/// compatibility with the `signer_pubkey` field.
/// Build the per-owner Merkle commitment block for a recall response
/// (Wave 5 / design §16). The `root` commits to the *set* of an owner's
/// `content_hash`es (rebuildable from Arweave); `proofs` carries one inclusion
/// proof per returned result so a client can check — against the root it
/// independently anchored/observed on Solana — that the operator neither
/// omitted nor tampered with the row. Pure local computation over the
/// rebuildable SQLite cache; no chain call here.
///
/// Returns `Null` for the anonymous cross-owner public pool (no single owner →
/// no single commitment) or if the owner's hash set can't be read.
fn build_merkle_commitment(
    store: &SqliteStore,
    owner_pubkey: &str,
    results: &[mnemonic_core::storage::SearchResult],
) -> serde_json::Value {
    use mnemonic_core::merkle;
    let all_hashes = match store.owner_content_hashes(owner_pubkey) {
        Ok(h) => h,
        Err(_) => return serde_json::Value::Null,
    };
    let root = merkle::commitment_root(&all_hashes);
    let mut proofs = serde_json::Map::new();
    for r in results {
        if let Some((_proof_root, steps)) = merkle::prove(&all_hashes, &r.content_hash) {
            let steps_json: Vec<serde_json::Value> = steps
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "sibling": merkle::to_hex32(&s.sibling),
                        "right": s.sibling_is_right,
                    })
                })
                .collect();
            proofs.insert(r.content_hash.clone(), serde_json::Value::Array(steps_json));
        }
    }
    serde_json::json!({
        "root": merkle::to_hex32(&root),
        "proofs": serde_json::Value::Object(proofs),
        "alg": "blake3-merkle/v1: leaf=blake3(0x00||content_hash), node=blake3(0x01||l||r), sorted-set",
    })
}

pub fn recall(
    keypair: &Keypair,
    store: &SqliteStore,
    embedder: &dyn Embedder,
    query: &str,
    limit: usize,
    owner_pubkey: Option<&str>,
    visibility_filter: Option<Visibility>,
) -> serde_json::Value {
    recall_with_chain(
        keypair,
        store,
        embedder,
        query,
        limit,
        owner_pubkey,
        visibility_filter,
        &[],
    )
}

/// Semantic recall over the node cache plus the latest verified chain
/// recovery snapshot. SQLite remains a rebuildable cache: when it is empty,
/// anchored items with a readable COSE payload are embedded on demand and
/// returned directly from the Arweave/Solana snapshot.
#[allow(clippy::too_many_arguments)]
pub fn recall_with_chain(
    keypair: &Keypair,
    store: &SqliteStore,
    embedder: &dyn Embedder,
    query: &str,
    limit: usize,
    owner_pubkey: Option<&str>,
    visibility_filter: Option<Visibility>,
    chain_items: &[RecoveredItem],
) -> serde_json::Value {
    let signer_pubkey = identity::pubkey_base58(keypair);
    let query_emb = embedder.embed(query);
    // Visibility-aware recall (Decision 5 / AC13 — agent-native-distribution).
    //
    // - Authenticated callers (`owner_pubkey = Some(sub)`,
    //   `visibility_filter = None`): see all of their own rows regardless
    //   of visibility — owner predicate is the only tenant boundary.
    // - Anonymous callers (`owner_pubkey = None`,
    //   `visibility_filter = Some(Visibility::Public)`): the storage layer
    //   drops the owner predicate and matches every row with
    //   `visibility = 'public'`. This is the cross-owner public pool the
    //   user-spec describes (AC13 / Flow 4) — agent-native-distribution
    //   Task 4 round 2 / SAR1-M1.
    //
    // The trait doc on `AttestationStore::search` requires `None` owner to
    // be paired with `Some(visibility)` so the storage layer never exposes
    // every row to an anonymous caller. The dispatcher constructs the
    // pair correctly at the `handle_tool_call` boundary.
    let mut results = store
        .search(&query_emb, owner_pubkey, visibility_filter, limit)
        .unwrap_or_default();
    let db_chain_ids: std::collections::HashSet<String> = store
        .recovery_facts()
        .unwrap_or_default()
        .into_iter()
        .filter(|fact| {
            owner_pubkey
                .map(|owner| fact.owner_pubkey.as_deref() == Some(owner))
                .unwrap_or(true)
        })
        .map(|fact| fact.arweave_tx)
        .filter(|tx| !tx.is_empty() && !tx.starts_with("local:"))
        .collect();
    let mut recovered_count = 0usize;
    for item in chain_items {
        if db_chain_ids.contains(&item.arweave_tx) {
            continue;
        }
        if let Some(owner) = owner_pubkey {
            let producer_matches = item
                .producer
                .as_deref()
                .map(mnemonic_core::arweave::recovery::normalize_producer)
                .as_deref()
                == Some(owner);
            if !producer_matches {
                continue;
            }
        }
        let Some(content) = item.content.as_deref().filter(|value| !value.is_empty()) else {
            continue;
        };
        let relevance_score = cosine_similarity(&query_emb, &embedder.embed(content));
        results.push(mnemonic_core::storage::SearchResult {
            attestation_id: item.arweave_tx.clone(),
            content: content.to_string(),
            content_hash: item.content_hash.clone().unwrap_or_default(),
            tags: item.tags.clone(),
            solana_tx: item.solana_tx.clone().unwrap_or_default(),
            arweave_tx: item.arweave_tx.clone(),
            created_at: item
                .day
                .as_ref()
                .map(|day| format!("{day}T00:00:00Z"))
                .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string()),
            write_mode: WriteMode::Participate,
            // Once plaintext is anchored on Arweave it is independently
            // fetchable. Lost SQLite visibility metadata cannot make it
            // private again, so recovered rows use the public wire value.
            visibility: Visibility::Public,
            relevance_score,
        });
        recovered_count += 1;
    }
    results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    // count() is signer-scoped (legacy semantic); search() is owner-scoped
    // OR cross-owner depending on the dispatcher's input.
    let total = store.count(&signer_pubkey).unwrap_or(0);
    // Verifiable recall (§16): attach the per-owner Merkle commitment + an
    // inclusion proof per result so an authenticated caller can detect an
    // operator that omits/tampers. Null for the anonymous cross-owner pool
    // (no single-owner commitment exists there).
    let merkle_commitment = match (owner_pubkey, recovered_count) {
        (Some(owner), 0) => build_merkle_commitment(store, owner, &results),
        _ => serde_json::Value::Null,
    };
    serde_json::json!({
        "query": query,
        "results": results,
        "total_attestations": total + recovered_count as i64,
        // `owner_pubkey` echoes the resolved scope:
        // - authenticated → the JWT sub (server-side derived)
        // - anonymous → `null` to signal cross-owner public-pool search
        "owner_pubkey": owner_pubkey,
        "embed_provider": embedder.provider_name(),
        "embed_model": embedder.model_id(),
        "verifiable": embedder.is_open_weights(),
        "merkle_commitment": merkle_commitment,
        "chain_recovered": recovered_count,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod chain_recall_tests {
    use super::*;

    struct SemanticStub;

    impl Embedder for SemanticStub {
        fn embed(&self, text: &str) -> Vec<f32> {
            if text.contains("alpha") {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            }
        }

        fn dim(&self) -> usize {
            2
        }

        fn provider_name(&self) -> &str {
            "chain-test"
        }

        fn model_id(&self) -> &str {
            "chain-test/v1"
        }
    }

    fn recovered(tx: &str, owner: &str, content: &str) -> RecoveredItem {
        RecoveredItem {
            arweave_tx: tx.to_string(),
            solana_tx: Some(format!("sol-{tx}")),
            content_hash: Some(format!("hash-{tx}")),
            content: Some(content.to_string()),
            tags: vec!["recovered".to_string()],
            day: Some("2026-07-13".to_string()),
            producer: Some(format!("did:sol:{owner}")),
        }
    }

    #[test]
    fn anchored_owner_recall_works_with_an_empty_sqlite_cache() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let store = SqliteStore::open(file.path()).unwrap();
        let keypair = Keypair::new();
        let items = vec![
            recovered("ar-alpha", "alice", "alpha checkpoint"),
            recovered("ar-beta", "bob", "beta checkpoint"),
        ];

        let output = recall_with_chain(
            &keypair,
            &store,
            &SemanticStub,
            "alpha",
            5,
            Some("alice"),
            None,
            &items,
        );

        assert_eq!(output["chain_recovered"], 1);
        assert_eq!(output["total_attestations"], 1);
        assert_eq!(output["results"].as_array().unwrap().len(), 1);
        assert_eq!(output["results"][0]["arweave_tx"], "ar-alpha");
        assert_eq!(output["results"][0]["content"], "alpha checkpoint");
        assert!(output["merkle_commitment"].is_null());
    }

    #[test]
    fn anonymous_chain_recall_exposes_only_already_anchored_items() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let store = SqliteStore::open(file.path()).unwrap();
        let keypair = Keypair::new();
        let items = vec![
            recovered("ar-alpha", "alice", "alpha checkpoint"),
            recovered("ar-beta", "bob", "beta checkpoint"),
        ];

        let output = recall_with_chain(
            &keypair,
            &store,
            &SemanticStub,
            "alpha",
            5,
            None,
            Some(Visibility::Public),
            &items,
        );

        assert_eq!(output["chain_recovered"], 2);
        assert_eq!(output["results"].as_array().unwrap().len(), 2);
        assert_eq!(output["results"][0]["arweave_tx"], "ar-alpha");
    }
}

#[cfg(test)]
mod sign_memory_tests {
    //! Decision-12 unit tests: HTTP/JWT path defers to PendingBundles, stdio
    //! path keeps inline signing. Network-free (uses dummy SolanaClient and
    //! ArweaveClient with `http://localhost:0`; tests only exercise the
    //! local-mode branch + the deferred branch, which never call out).

    use super::*;
    use crate::pending::PendingBundles;
    use mnemonic_core::storage::SqliteStore;
    use solana_sdk::signature::{Keypair, Signer};

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

    fn fixtures() -> (
        Keypair,
        SolanaClient,
        ArweaveClient,
        std::sync::Mutex<SqliteStore>,
        StubEmbedder,
        EmbeddingCompressor,
        PendingBundles,
        crate::pricing::CostHint,
    ) {
        let kp = Keypair::new();
        let sol = SolanaClient::new("http://localhost:0");
        let ar = ArweaveClient::new("http://localhost:0");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = std::sync::Mutex::new(SqliteStore::open(tmp.path()).unwrap());
        let comp = EmbeddingCompressor::new(8, 4, 42);
        let pending = PendingBundles::with_defaults();
        let hint = crate::pricing::CostHint {
            irys_lamports: 0,
            sol_tx_fee_lamports: 0,
            sol_price_usdc: 0.0,
            charge_micro_usdc: 0,
        };
        // Keep tmp alive for the test duration via leaking the path keeper.
        std::mem::forget(tmp);
        (kp, sol, ar, store, StubEmbedder, comp, pending, hint)
    }

    fn local_envelope() -> Envelope {
        Envelope::from_config("local", "none", 0)
    }

    /// Soft-fall disabled — Task 5 unit tests in this module don't exercise
    /// the escalation router. Returns an empty endpoint sentinel + a dummy
    /// client; `sign_memory` treats either as "no soft-fall available" and
    /// propagates the local error verbatim.
    fn no_softfall() -> (reqwest::Client, serde_json::Value) {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap();
        (client, serde_json::json!({}))
    }

    #[tokio::test]
    async fn test_sign_memory_returns_awaiting_signature_for_jwt_path() {
        // T2 round-2: mode-absent JWT path against a local-only deploy
        // resolves to `Local` via env-var fallback (`explicit = false`)
        // and STILL takes the deferred branch — the routing rule
        // bypasses deferred only for *explicit* local requests. This
        // pins the legacy chrome-extension Cloud-tier shape byte-for-
        // byte (no `mode` field, deferred envelope).
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();
        let env = local_envelope();
        let resolved = resolve_write_mode(None, "local").unwrap();
        let (hosted_client, args) = no_softfall();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "hello",
            &[],
            &hint,
            "local",
            &owner,
            Some("user-jwt-sub"),
            resolved,
            Visibility::Private,
            &env,
            std::time::Duration::from_secs(15),
            false,
            "",
            &hosted_client,
            &args,
        )
        .await
        .unwrap();

        assert_eq!(result["status"], "awaiting_signature");
        assert!(result["correlation_id"].is_string());
        assert_eq!(result["expires_in"], 300);
        let url = result["approve_url"].as_str().unwrap();
        assert!(url.starts_with("https://mnemonik.xyz/sign/"));
        // No SQLite row should have been written.
        let s = store.lock().unwrap();
        assert_eq!(s.count(&owner).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_sign_memory_stdio_path_unchanged() {
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();
        let env = local_envelope();
        let resolved = resolve_write_mode(None, "local").unwrap();
        let (hosted_client, args) = no_softfall();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "stdio mem",
            &[],
            &hint,
            "local",
            &owner,
            None,
            resolved,
            Visibility::Private,
            &env,
            std::time::Duration::from_secs(15),
            false,
            "",
            &hosted_client,
            &args,
        )
        .await
        .unwrap();
        // Stdio path: produces an attestation_id and persists.
        assert!(result["attestation_id"].is_string());
        assert!(result["content_hash"].is_string());
        let s = store.lock().unwrap();
        assert_eq!(s.count(&owner).unwrap(), 1);
    }

    #[tokio::test]
    async fn test_explicit_local_with_jwt_takes_inline_path() {
        // T2 round-2 (security-auditor major): explicit `mode: "local"`
        // with a JWT MUST short-circuit to the inline path regardless
        // of deploy variant — both local-only AND full deploys honour
        // the "Личная память бесплатна всегда" invariant uniformly.
        // Round-1's `envelope.supports_participate()` workaround broke
        // this for local-only deploys.
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();

        // Sub-case A: explicit local on a local-only deploy.
        let env_local = local_envelope();
        let resolved_explicit_local =
            resolve_write_mode(Some(&serde_json::json!("local")), "local").unwrap();
        assert!(resolved_explicit_local.is_explicit_local());
        let (hosted_client, args) = no_softfall();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "explicit-local-on-local",
            &[],
            &hint,
            "local",
            &owner,
            Some("user-jwt-sub"),
            resolved_explicit_local,
            Visibility::Private,
            &env_local,
            std::time::Duration::from_secs(15),
            false,
            "",
            &hosted_client,
            &args,
        )
        .await
        .unwrap();
        assert!(
            result["attestation_id"].is_string(),
            "expected inline shape, got {result:?}"
        );
        assert_eq!(result["write_mode"], "local");
        // Sub-case B: explicit local on a full deploy.
        let env_full = Envelope::from_config("full", "none", 0);
        let resolved_explicit_local_full =
            resolve_write_mode(Some(&serde_json::json!("local")), "full").unwrap();
        assert!(resolved_explicit_local_full.is_explicit_local());
        let (hosted_client, args) = no_softfall();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "explicit-local-on-full",
            &[],
            &hint,
            "full",
            &owner,
            Some("user-jwt-sub"),
            resolved_explicit_local_full,
            Visibility::Private,
            &env_full,
            std::time::Duration::from_secs(15),
            false,
            "",
            &hosted_client,
            &args,
        )
        .await
        .unwrap();
        assert!(
            result["attestation_id"].is_string(),
            "expected inline shape on full deploy too"
        );
        assert_eq!(result["write_mode"], "local");
    }
}

#[cfg(test)]
mod resolve_write_mode_tests {
    //! T2 resolver unit tests. Pure function — no fixtures needed.
    //!
    //! Drives the SINGLE source of truth that feeds both the paywall gate
    //! in `mcp_handler` and the persisted `write_mode` column. Drift is
    //! impossible by construction because both call sites consume the
    //! return value of `resolve_write_mode`.

    use super::*;

    /// Helper: assert the error is `-32602 InvalidParams` with the expected
    /// `data.field` and `data.received` payload.
    fn assert_invalid_params(err: JsonRpcError, expected_received: &serde_json::Value) {
        assert_eq!(err.code, -32602, "expected -32602 InvalidParams");
        assert_eq!(err.message, "Invalid params");
        let data = err.data.expect("InvalidParams must carry `data`");
        assert_eq!(data["field"], "mode", "data.field must be \"mode\"");
        assert_eq!(
            &data["received"], expected_received,
            "data.received must echo input verbatim"
        );
    }

    #[test]
    fn none_with_env_local_resolves_to_local_fallback() {
        let r = resolve_write_mode(None, "local").expect("None+local resolves");
        assert_eq!(r.write_mode, WriteMode::Local);
        assert!(!r.explicit, "env-var fallback must not be marked explicit");
        assert!(!r.is_explicit_local());
    }

    #[test]
    fn none_with_env_full_resolves_to_participate_fallback() {
        // Legacy compat: pre-T2 clients (chrome-extension Cloud) on a full
        // deploy fall back to env-var behaviour — Participate.
        let r = resolve_write_mode(None, "full").expect("None+full resolves");
        assert_eq!(r.write_mode, WriteMode::Participate);
        assert!(!r.explicit);
    }

    #[test]
    fn explicit_local_string_resolves_to_local_explicit() {
        let v = serde_json::json!("local");
        let r = resolve_write_mode(Some(&v), "full").expect("explicit local");
        assert_eq!(r.write_mode, WriteMode::Local);
        assert!(r.explicit, "string-literal input must be marked explicit");
        assert!(r.is_explicit_local());
    }

    #[test]
    fn explicit_participate_string_resolves_to_participate_explicit() {
        let v = serde_json::json!("participate");
        let r = resolve_write_mode(Some(&v), "local").expect("explicit participate");
        // Note: even on a `STORAGE_MODE=local` env, the resolver returns
        // Participate; rejection happens later in `sign_memory` via the
        // envelope check. The resolver's job is parse-only.
        assert_eq!(r.write_mode, WriteMode::Participate);
        assert!(r.explicit);
    }

    #[test]
    fn null_rejects_with_invalid_params() {
        let v = serde_json::Value::Null;
        let err = resolve_write_mode(Some(&v), "local").expect_err("null rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_integer_rejects() {
        let v = serde_json::json!(42);
        let err = resolve_write_mode(Some(&v), "local").expect_err("integer rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_array_rejects() {
        let v = serde_json::json!(["local"]);
        let err = resolve_write_mode(Some(&v), "local").expect_err("array rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_object_rejects() {
        let v = serde_json::json!({"mode": "local"});
        let err = resolve_write_mode(Some(&v), "local").expect_err("object rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn empty_string_rejects() {
        let v = serde_json::json!("");
        let err = resolve_write_mode(Some(&v), "local").expect_err("empty rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn whitespace_string_rejects() {
        let v = serde_json::json!(" ");
        let err = resolve_write_mode(Some(&v), "local").expect_err("whitespace rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn capitalised_local_rejects() {
        let v = serde_json::json!("Local");
        let err = resolve_write_mode(Some(&v), "local").expect_err("Local rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn uppercase_participate_rejects() {
        let v = serde_json::json!("PARTICIPATE");
        let err = resolve_write_mode(Some(&v), "local").expect_err("PARTICIPATE rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn unknown_string_rejects() {
        let v = serde_json::json!("cloud");
        let err = resolve_write_mode(Some(&v), "local").expect_err("unknown rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn trailing_whitespace_rejects() {
        let v = serde_json::json!("local ");
        let err = resolve_write_mode(Some(&v), "local").expect_err("trailing space rejects");
        assert_invalid_params(err, &v);
    }
}
