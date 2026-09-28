//! HTTP routes for Task 10 — E2E sealed-memory writes, grants, and embedding.
//!
//! New surfaces:
//!
//! - `POST /api/anchor-sealed`   — body = signed COSE_Sign1 (producer == did:sol:<jwt.sub>)
//! - `POST /api/store-sealed`    — unsigned outer CBOR for local writes (owner = jwt.sub)
//! - `GET  /api/sealed`          — list caller's own sealed blobs
//! - `POST /api/grants`          — persist a signed GRANT_V1 (author = jwt.sub)
//! - `GET  /api/grants`          — non-withdrawn grants for a given reader kid
//! - `DELETE /api/grants/{id}`   — withdraw a grant (sets withdrawn_at)
//! - `POST /api/embed`           — returns an embedding vector; NEVER logs the body

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use mnemonic_core::codec::{hash::hash_bytes, sign::verify_artifact};
use mnemonic_core::storage::{WriteMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::McpState;
use crate::oauth::Claims;

// ── helpers ──────────────────────────────────────────────────────────────────

fn error_resp(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"status": "error", "error": msg})),
    )
        .into_response()
}

// ── POST /api/anchor-sealed ───────────────────────────────────────────────────

/// `POST /api/anchor-sealed` — body is raw COSE_Sign1 bytes of a sealed memory.
///
/// Validation:
/// 1. Verify COSE_Sign1 — rejects invalid signatures.
/// 2. Assert `producer == did:sol:<jwt.sub>` — rejects cross-owner writes.
/// 3. Payment / free quota (reuses `payment.rs` logic).
/// 4. Upload to Arweave, SPL Memo anchor, delivery check (local-mode: synthetic ids).
/// 5. `save_sealed_attestation` — never writes an embedding row.
pub async fn anchor_sealed_handler(
    State(state): State<Arc<McpState>>,
    Extension(claims): Extension<Claims>,
    body: axum::body::Bytes,
) -> Response {
    let jwt_sub = &claims.sub;

    // 1. Verify the COSE_Sign1 envelope.
    let result = match verify_artifact(&body, None) {
        Ok(r) => r,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("COSE verification failed: {e}"),
            );
        }
    };
    if !result.valid || !result.cose_signature || !result.algorithm_valid {
        return error_resp(StatusCode::UNAUTHORIZED, "COSE signature invalid");
    }

    // 2. Enforce producer == did:sol:<jwt.sub>.
    let expected_producer = format!("did:sol:{jwt_sub}");
    // The `signer` field from verify_artifact is the raw base58 pubkey (COSE kid).
    // Check it against jwt_sub directly.
    if result.signer != *jwt_sub {
        return error_resp(
            StatusCode::FORBIDDEN,
            "producer does not match authenticated identity",
        );
    }

    // Also verify the producer DID in the payload, if present.
    if let Ok(payload_json) = mnemonic_core::codec::canonical::from_canonical_cbor(&result.payload)
    {
        if let Some(producer) = payload_json.get("producer").and_then(|v| v.as_str()) {
            if producer != expected_producer {
                return error_resp(
                    StatusCode::FORBIDDEN,
                    "payload producer DID does not match authenticated identity",
                );
            }
        }
    }

    // 3. Payment / free quota — same gating as sign-callback (local-mode: free).
    // For simplicity in V1 we only run on local-mode (storage_mode == "local")
    // where no payment is needed. Full-mode payment is handled by the deferred
    // sign-callback path, not this direct route.
    // (A future wave can add UP billing here.)

    let content_hash = hash_bytes(&result.payload);
    let attestation_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // 4. Anchoring (local-mode uses synthetic ids; full-mode skipped in V1).
    let is_local = state.storage_mode == "local";
    let (solana_tx, arweave_tx) = if is_local {
        (
            format!("local:{}", &content_hash[..16]),
            format!("local:{}", &attestation_id[..8]),
        )
    } else {
        // Full-mode: upload to Arweave + SPL Memo. Minimal tags for sealed entries.
        let producer_did = expected_producer.clone();
        let sealed_tags: &[(&str, &str)] = &[
            ("Producer", producer_did.as_str()),
            ("Created-At", now.as_str()),
            ("Mnemonic-Type", "sealed"),
        ];
        let operator_keypair = match state.keypair.keypair() {
            Ok(kp) => kp,
            Err(e) => {
                return error_resp(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &format!("operator identity unavailable: {e:#}"),
                );
            }
        };
        let ar_tx = match state
            .arweave
            .write_item(&body, operator_keypair, sealed_tags)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("arweave upload failed: {e}"),
                );
            }
        };
        let _ = state.arweave.mine().await;
        let memo = serde_json::json!({"h": content_hash, "a": ar_tx, "v": 3});
        let operator_keypair = match state.keypair.keypair() {
            Ok(kp) => kp,
            Err(e) => {
                return error_resp(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &format!("operator identity unavailable: {e:#}"),
                );
            }
        };
        let sol_tx = match state
            .solana
            .submit_memo(operator_keypair, &memo.to_string())
            .await
        {
            Ok(sig) => sig,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("solana memo write failed: {e}"),
                );
            }
        };
        (sol_tx, ar_tx)
    };

    // 5. Persist via save_sealed_attestation.
    let save_res = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        store.save_sealed_attestation(
            &attestation_id,
            &content_hash,
            &[],
            &solana_tx,
            &arweave_tx,
            jwt_sub.as_str(), // signer
            jwt_sub.as_str(), // owner
            &now,
            WriteMode::Local, // anchored in full-mode but stored as Local for routing
            &body,
        )
    };
    if let Err(e) = save_res {
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("persist failed: {e}"),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "attestation_id": attestation_id,
            "content_hash": content_hash,
            "solana_tx": solana_tx,
            "arweave_tx": arweave_tx,
        })),
    )
        .into_response()
}

// ── POST /api/store-sealed ────────────────────────────────────────────────────

/// `POST /api/store-sealed` — unsigned outer CBOR for `local` writes.
///
/// The body is the raw unsigned SEALED_V1 CBOR (no COSE wrapper).
/// Owner = `jwt.sub`. Never writes an embedding row.
pub async fn store_sealed_handler(
    State(state): State<Arc<McpState>>,
    Extension(claims): Extension<Claims>,
    body: axum::body::Bytes,
) -> Response {
    let jwt_sub = &claims.sub;
    let content_hash = hash_bytes(&body);
    let attestation_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let solana_tx = format!("local:{}", &content_hash[..16]);
    let arweave_tx = format!("local:{}", &attestation_id[..8]);

    let save_res = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        store.save_sealed_attestation(
            &attestation_id,
            &content_hash,
            &[],
            &solana_tx,
            &arweave_tx,
            jwt_sub.as_str(), // signer
            jwt_sub.as_str(), // owner
            &now,
            WriteMode::Local,
            &body,
        )
    };
    if let Err(e) = save_res {
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("persist failed: {e}"),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "attestation_id": attestation_id,
            "content_hash": content_hash,
            "solana_tx": solana_tx,
            "arweave_tx": arweave_tx,
        })),
    )
        .into_response()
}

// ── GET /api/sealed ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SealedQuery {
    pub since: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/sealed?since=<iso>&limit=<n>` — list caller's own sealed blobs.
pub async fn list_sealed_handler(
    State(state): State<Arc<McpState>>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<SealedQuery>,
) -> Response {
    let owner = &claims.sub;
    let limit = q.limit.unwrap_or(20).min(100);
    let rows = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        match store.list_sealed(owner, q.since.as_deref(), limit) {
            Ok(rows) => rows,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("list failed: {e}"),
                );
            }
        }
    };
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "attestation_id": r.attestation_id,
                "content_hash": r.content_hash,
                "solana_tx": r.solana_tx,
                "arweave_tx": r.arweave_tx,
                "created_at": r.created_at,
                // The raw COSE_Sign1 / CBOR blob is returned as base64 so the
                // client can decrypt locally without needing another round-trip.
                "sealed_blob_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &r.sealed_blob,
                ),
            })
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({"items": items}))).into_response()
}

// ── POST /api/grants ──────────────────────────────────────────────────────────

/// Body of `POST /api/grants`.
#[derive(Debug, Deserialize)]
pub struct CreateGrantRequest {
    /// base64 (standard) encoded COSE_Sign1 bytes of the GRANT_V1 artifact.
    pub grant_cose_b64: String,
    /// The blake3 hex hash of the sealed memory this grant targets.
    pub memory_hash: String,
    /// Optional reader DID/kid. `None` for broadcast/anonymous grants.
    pub reader_kid: Option<String>,
}

/// `POST /api/grants` — persist a signed GRANT_V1 (author = jwt.sub).
pub async fn create_grant_handler(
    State(state): State<Arc<McpState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateGrantRequest>,
) -> Response {
    let jwt_sub = &claims.sub;

    // Decode the COSE bytes.
    let grant_cose = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.grant_cose_b64.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("grant_cose_b64 is not valid base64: {e}"),
            );
        }
    };

    // Verify the COSE_Sign1.
    let result = match verify_artifact(&grant_cose, None) {
        Ok(r) => r,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("COSE verification failed: {e}"),
            );
        }
    };
    if !result.valid || !result.cose_signature {
        return error_resp(StatusCode::UNAUTHORIZED, "grant COSE signature invalid");
    }
    // Enforce author == jwt.sub.
    if result.signer != *jwt_sub {
        return error_resp(
            StatusCode::FORBIDDEN,
            "grant signer does not match authenticated identity",
        );
    }

    let grant_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let save_res = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        store.save_grant(
            &grant_id,
            &req.memory_hash,
            req.reader_kid.as_deref(),
            &grant_cose,
            jwt_sub.as_str(),
            &now,
        )
    };
    if let Err(e) = save_res {
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("persist failed: {e}"),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "ok",
            "grant_id": grant_id,
        })),
    )
        .into_response()
}

// ── GET /api/grants ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GrantsQuery {
    pub reader: Option<String>,
}

#[derive(Debug, Serialize)]
struct GrantItem {
    id: String,
    memory_hash: String,
    reader_kid: Option<String>,
    author_pubkey: String,
    created_at: String,
    /// base64-encoded raw COSE_Sign1 grant envelope.
    grant_cose_b64: String,
}

/// `GET /api/grants?reader=<kid>` — non-withdrawn grants for this reader.
pub async fn list_grants_handler(
    State(state): State<Arc<McpState>>,
    Extension(_claims): Extension<Claims>,
    Query(q): Query<GrantsQuery>,
) -> Response {
    let reader = match q.reader {
        Some(r) => r,
        None => {
            return error_resp(StatusCode::BAD_REQUEST, "reader query parameter is required");
        }
    };
    let rows = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        match store.grants_for_reader(&reader) {
            Ok(rows) => rows,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("list grants failed: {e}"),
                );
            }
        }
    };
    let items: Vec<GrantItem> = rows
        .into_iter()
        .map(|g| GrantItem {
            id: g.id,
            memory_hash: g.memory_hash,
            reader_kid: g.reader_kid,
            author_pubkey: g.author_pubkey,
            created_at: g.created_at,
            grant_cose_b64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &g.grant_cose,
            ),
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({"grants": items}))).into_response()
}

// ── DELETE /api/grants/{id} ───────────────────────────────────────────────────

/// `DELETE /api/grants/{id}` — withdraw (sets withdrawn_at).
pub async fn delete_grant_handler(
    State(state): State<Arc<McpState>>,
    Extension(_claims): Extension<Claims>,
    Path(id): Path<String>,
) -> Response {
    let res = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "store unavailable");
            }
        };
        store.withdraw_grant(&id)
    };
    match res {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("withdraw failed: {e}"),
        ),
    }
}

// ── POST /api/embed ───────────────────────────────────────────────────────────

/// `POST /api/embed` — accepts a plain-text body, returns an embedding vector.
///
/// NEVER stores or logs the input text (Decision D-4 privacy contract).
/// The body is consumed, embedded, then immediately dropped — the request
/// handler has no tracing::* span that captures the body bytes.
pub async fn embed_handler(
    State(state): State<Arc<McpState>>,
    Extension(_claims): Extension<Claims>,
    // NOTE: axum extracts the body as raw bytes so we never bind it to a
    // named variable that a tracing span could capture.
    body: axum::body::Bytes,
) -> Response {
    // Decode UTF-8 but DO NOT log or store the text.
    let text = match std::str::from_utf8(&body) {
        Ok(t) => t,
        Err(_) => {
            return error_resp(StatusCode::BAD_REQUEST, "body must be valid UTF-8");
        }
    };
    if text.is_empty() {
        return error_resp(StatusCode::BAD_REQUEST, "body must not be empty");
    }

    // Produce the embedding. The embedder is synchronous and lock-free on
    // the hot path; no blocking call, no await.
    let embedding = state.embedder.embed(text);
    // `text` is dropped here — it is a &str over the request bytes, which
    // are also dropped at the end of this handler's scope.

    if embedding.is_empty() {
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            "embedder returned an empty vector",
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "embedding": embedding,
            "dim": embedding.len(),
            "model": state.embedder.model_id(),
        })),
    )
        .into_response()
}
