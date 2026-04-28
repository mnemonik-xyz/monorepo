//! `/api/pending/*` and `/api/sign-callback` routes — Decision 12 webapp surface.
//!
//! These two HTTP endpoints close the browser-mediated signing loop:
//!
//! - **`GET /api/pending/{correlation_id}`** — webapp fetches the unsigned
//!   canonical-CBOR bytes the user must sign (Content-Type: application/cbor).
//!   Auth required; `jwt.sub` must match the entry's stored `jwt_sub` (403
//!   otherwise). Returns 404 if absent, 410 if expired or already consumed.
//!
//! - **`POST /api/sign-callback`** — webapp delivers the COSE_Sign1 bytes
//!   produced by the WASM signer. Body schema:
//!   `{"correlation_id": String, "cose_signed_bytes": base64, "signer_pubkey": base58}`.
//!   Validation order:
//!   1. lookup pending (410 if missing/expired/consumed)
//!   2. assert `signer_pubkey == jwt.sub` (403 otherwise)
//!   3. verify COSE_Sign1 against `entry.content_hash` (401 otherwise)
//!   4. atomically `consume` (single-use; replay → 410)
//!   5. persist via `AttestationStore::save_attestation(... owner_pubkey = jwt.sub)`
//!   6. return `{"status": "ok", "attestation_id": <uuid>}`
//!
//! Auth model: both endpoints sit behind `oauth::bearer_auth_middleware`;
//! `Claims` is read from request extensions.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use mnemonic_core::codec::{hash::hash_bytes, sign::verify_artifact};
use mnemonic_core::storage::AttestationStore;
use serde::{Deserialize, Serialize};

use crate::mcp::McpState;
use crate::pending::PendingError;

/// `GET /api/pending/{correlation_id}` — webapp fetches the unsigned
/// canonical-CBOR bytes for the user to sign.
///
/// Auth: capability-based — `correlation_id` IS the capability. No Bearer
/// JWT required because the webapp on `mnemonik.xyz` does not hold the JWT
/// that the AI-tool client received from `/oauth/token`. The unsigned CBOR
/// leak is bounded (5-min TTL, content the user just typed); the actual
/// signing chain is gated by `consume_by_id` + COSE verification in
/// `sign_callback_handler`, where keypair ownership is the auth.
pub async fn get_pending_handler(
    State(state): State<Arc<McpState>>,
    Path(correlation_id): Path<String>,
) -> Response {
    let entry = match state.pending.peek_by_id(&correlation_id).await {
        Ok(e) => e,
        Err(e) => return e.into_response(),
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/cbor"),
    );
    // Surface the expected hash + correlation id so the webapp can sanity-
    // check before signing without a second round-trip. These are not
    // load-bearing (the COSE_Sign1 envelope already carries the hash); they
    // are convenience headers.
    if let Ok(hv) = HeaderValue::from_str(&entry.content_hash) {
        headers.insert("x-mnemonic-content-hash", hv);
    }
    if let Ok(hv) = HeaderValue::from_str(&correlation_id) {
        headers.insert("x-mnemonic-correlation-id", hv);
    }

    let mut resp = Response::new(Body::from(entry.canonical_cbor));
    *resp.status_mut() = StatusCode::OK;
    *resp.headers_mut() = headers;
    resp
}

/// Body of `POST /api/sign-callback`.
#[derive(Debug, Deserialize)]
pub struct SignCallbackRequest {
    pub correlation_id: String,
    /// base64 (standard alphabet, with padding) of the COSE_Sign1 envelope.
    pub cose_signed_bytes: String,
    /// base58 user pubkey — must equal `jwt.sub` and the COSE kid.
    pub signer_pubkey: String,
}

/// Successful response of `POST /api/sign-callback`.
#[derive(Debug, Serialize)]
pub struct SignCallbackResponse {
    pub status: &'static str,
    pub attestation_id: String,
    pub content_hash: String,
}

/// `POST /api/sign-callback` — webapp delivers the user's COSE_Sign1.
///
/// Auth: capability + cryptographic chain — no Bearer JWT required. The
/// body's `signer_pubkey` MUST equal the pending entry's stored `jwt_sub`
/// (validated atomically by `consume_by_id`), AND the COSE signature MUST
/// verify against that same pubkey. An attacker holding only a guessed
/// `correlation_id` cannot forge this without the user's private key.
pub async fn sign_callback_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<SignCallbackRequest>,
) -> Response {
    // 2. Decode the COSE bytes.
    let cose_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.cose_signed_bytes.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("cose_signed_bytes is not valid base64: {e}"),
            );
        }
    };

    // 3. Atomic consume — pop the entry from the LRU under a single lock.
    //    A concurrent second callback for the same correlation_id observes
    //    `NotFound` after this point. We do NOT verify the COSE before
    //    popping; otherwise two concurrent valid callbacks would both
    //    proceed to step 5 (persistence), inserting two SQLite rows for the
    //    same logical attestation.
    let entry = match state
        .pending
        .consume_by_id(&req.correlation_id, &req.signer_pubkey)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            // PendingError::NotFound → 404. But per Decision 12 a missing
            // entry on the callback is "already consumed or expired", which
            // is semantically 410 Gone. Override here.
            if matches!(e, PendingError::NotFound) {
                return error_resp(
                    StatusCode::GONE,
                    "pending bundle missing or already consumed",
                );
            }
            return e.into_response();
        }
    };

    // 4. Verify COSE against the stored content hash. If verification fails
    //    the entry is already gone from the LRU; the user's bundle is
    //    effectively forfeit. This is the right tradeoff: tampered or
    //    replayed signatures should not allow retries.
    let result = match verify_artifact(&cose_bytes, Some(&entry.content_hash)) {
        Ok(r) => r,
        Err(e) => {
            return error_resp(
                StatusCode::UNAUTHORIZED,
                &format!("COSE verification failed: {e}"),
            );
        }
    };
    if !result.valid
        || !result.cose_signature
        || !result.content_integrity
        || !result.algorithm_valid
    {
        return error_resp(StatusCode::UNAUTHORIZED, "COSE signature invalid");
    }
    // The COSE kid (recovered as `result.signer`) must equal the body's
    // `signer_pubkey`, which `consume_by_id` already validated against the
    // pending entry's stored `jwt_sub`. This closes the chain:
    // "correlation_id capability → entry.jwt_sub → body.signer_pubkey → COSE kid → Ed25519 signature".
    if result.signer != req.signer_pubkey {
        return error_resp(
            StatusCode::UNAUTHORIZED,
            "COSE kid does not match signer_pubkey",
        );
    }
    // Defense in depth: independently recompute the hash of the recovered
    // COSE payload and compare with the stored entry hash. `verify_artifact`
    // already does this internally (via `expected_hash`), but a second
    // explicit check guarantees that even if `result.content_integrity` is
    // ever loosened we still catch a mismatch here.
    let recomputed = hash_bytes(&result.payload);
    if recomputed != entry.content_hash {
        return error_resp(
            StatusCode::UNAUTHORIZED,
            "stored content_hash differs from COSE payload hash",
        );
    }

    // 5. Persist. `attestation_id` is freshly generated for the SQLite
    //    primary key — `correlation_id` was the routing token only. Synthetic
    //    `local:` tx IDs per Decision 4 (no Arweave/Solana write on the
    //    browser-mediated path; storage_mode is forced to local for the
    //    HTTP/JWT trust model).
    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let local_ar = format!("local:{}", &attestation_id[..8]);
    let local_sol = format!("local:{}", &entry.content_hash[..16]);

    let persist_res = {
        // Short, await-free critical section.
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        store.save_attestation(
            &attestation_id,
            &entry.content,
            &entry.content_hash,
            &entry.tags,
            &local_sol,
            &local_ar,
            &req.signer_pubkey, // signer = pubkey we just verified via COSE
            &req.signer_pubkey, // owner = same pubkey (Decision 9 — webapp flow uses keypair as identity)
            &now,
            &entry.embedding,
        )
    };
    if let Err(e) = persist_res {
        // Persistence failed AFTER the LRU consumed the entry. The user's
        // bundle is gone. Surface a 500 — the failure mode here is
        // operator-visible (DB I/O / disk full) rather than user-fixable.
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("persist failed: {e}"),
        );
    }

    let body = SignCallbackResponse {
        status: "ok",
        attestation_id,
        content_hash: entry.content_hash,
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn error_resp(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"status": "error", "error": msg})),
    )
        .into_response()
}
