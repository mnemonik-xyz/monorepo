//! HTTP routes for Task 10 — E2E sealed-memory writes, grants, and embedding.
//!
//! New surfaces:
//!
//! - `POST /api/anchor-sealed`   — body = signed COSE_Sign1 (producer == did:sol:<jwt.sub>)
//! - `POST /api/store-sealed`    — retired (410), local writes belong on the client
//! - `GET  /api/sealed`          — list caller's own sealed blobs
//! - `POST /api/grants`          — retired (410), distribute grants client-side
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
use serde::{Deserialize, Serialize};

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
/// Compatibility adapter to shared validation, quota/payment and external delivery.
/// No synthetic local IDs, Solana memo, or artifact-byte SQL persistence.
pub async fn anchor_sealed_handler(
    State(state): State<Arc<McpState>>,
    request: axum::http::Request<axum::body::Body>,
) -> Response {
    crate::ingestion::ingest_handler(State(state), request).await
}

/// Hosted local writes are retired. Store signed bytes on the agent's device.
pub async fn store_sealed_handler() -> Response {
    error_resp(StatusCode::GONE, "hosted local storage retired; retain sealed bytes on the client")
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
/// `POST /api/grants` — retired (410), distribute grants client-side.
pub async fn create_grant_handler() -> Response {
    error_resp(StatusCode::GONE, "hosted grant storage retired; retain and deliver signed grants client-side, or use sealed A2A recipient grants")
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
