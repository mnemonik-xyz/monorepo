//! Browser approval page routes for the Universal Paywall exact x402 rail.
//!
//! These routes are mounted inside `mnemonic-mcp` so the production binary
//! serves the approval UI and proxies facilitator calls without exposing the
//! facilitator API key to the browser.
//!
//! The `/api/mock-sign` route is test-only: it is compiled only when the
//! `approval-mock-signer` feature is enabled and enabled at runtime only when
//! `MNEMONIC_APPROVAL_MOCK_SIGNER` is set.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::McpState;
use crate::paid_operation;
use crate::universal_paywall::{
    OperationBinding, PaymentAuthorization, PaymentReceipt, UniversalPaywallClient,
};
use crate::wallet_link;

const MAX_OPERATION_ID_LEN: usize = 128;

#[derive(Debug, Deserialize)]
struct ResumeQuery {
    resume_token: String,
}

fn error_resp(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({ "error": message });
    (status, Json(body)).into_response()
}

fn parse_eip155(network: &str) -> Option<u64> {
    network.strip_prefix("eip155:").and_then(|s| s.parse().ok())
}

/// GET /approve — serve the built approval page.
async fn approve_page_handler(State(state): State<Arc<McpState>>) -> Response {
    let Some(dist) = &state.approval_ui_dist else {
        return error_resp(StatusCode::NOT_FOUND, "approval UI not configured");
    };
    let path = dist.join("index.html");
    match tokio::fs::read_to_string(&path).await {
        Ok(html) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            // Strict CSP: only same-origin scripts/styles, no inline scripts,
            // and only the configured chain RPC for connect-src.
            let csp = format!(
                "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' {}; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'",
                state.approval_chain_rpc_url
            );
            headers.insert(
                axum::http::header::CONTENT_SECURITY_POLICY,
                HeaderValue::try_from(csp)
                    .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'")),
            );
            (StatusCode::OK, headers, html).into_response()
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to read approval page");
            error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "approval page unavailable",
            )
        }
    }
}

/// GET /api/quote/:operation_id — proxy to the facilitator.
async fn quote_handler(
    State(state): State<Arc<McpState>>,
    Path(operation_id): Path<String>,
) -> Response {
    if operation_id.is_empty() || operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    let Some(cfg) = state.universal_paywall.clone() else {
        return error_resp(
            StatusCode::SERVICE_UNAVAILABLE,
            "universal paywall not configured",
        );
    };
    let client = UniversalPaywallClient::new(cfg);
    match client.get_quote_by_operation_id(&operation_id).await {
        Ok(quote) => (StatusCode::OK, Json(quote)).into_response(),
        Err(e) => {
            tracing::warn!(operation_id, error = %e, "facilitator quote lookup failed");
            error_resp(StatusCode::BAD_GATEWAY, "facilitator unavailable")
        }
    }
}

#[derive(Debug, Deserialize)]
struct WalletLinkRequest {
    signature: String,
}

async fn wallet_link_get_handler(
    State(state): State<Arc<McpState>>,
    Path(operation_id): Path<String>,
) -> Response {
    if operation_id.is_empty() || operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    let challenge = match state.store.lock() {
        Ok(store) => match get_wallet_link_challenge(store.conn(), &operation_id) {
            Ok(challenge) => challenge,
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "wallet link unavailable")
            }
        },
        Err(_) => return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "wallet link unavailable"),
    };
    let Some(challenge) = challenge else {
        return error_resp(StatusCode::NOT_FOUND, "wallet link challenge not found");
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challenge": challenge,
            "message": wallet_link::challenge_message(&challenge),
        })),
    )
        .into_response()
}

async fn wallet_link_post_handler(
    State(state): State<Arc<McpState>>,
    Path(operation_id): Path<String>,
    Json(req): Json<WalletLinkRequest>,
) -> Response {
    let challenge = match state.store.lock() {
        Ok(store) => match get_wallet_link_challenge(store.conn(), &operation_id) {
            Ok(Some(challenge)) => challenge,
            Ok(None) => {
                return error_resp(StatusCode::NOT_FOUND, "wallet link challenge not found")
            }
            Err(_) => {
                return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "wallet link unavailable")
            }
        },
        Err(_) => return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "wallet link unavailable"),
    };
    match state.store.lock() {
        Ok(store) => match wallet_link::verify_and_record(
            store.conn(),
            &challenge,
            &req.signature,
            chrono::Utc::now(),
        ) {
            Ok(link) => (
                StatusCode::OK,
                Json(serde_json::json!({"wallet_address": link.wallet_address})),
            )
                .into_response(),
            Err(error) => {
                tracing::warn!(operation_id, error = %error, "wallet link verification failed");
                error_resp(StatusCode::UNAUTHORIZED, "wallet link signature rejected")
            }
        },
        Err(_) => error_resp(StatusCode::INTERNAL_SERVER_ERROR, "wallet link unavailable"),
    }
}

fn get_wallet_link_challenge(
    conn: &rusqlite::Connection,
    operation_id: &str,
) -> anyhow::Result<Option<wallet_link::WalletLinkChallenge>> {
    // Reconstruct via the public create-or-get function only after first
    // looking up the subject and chain from the durable row.
    let row: Option<(String, u64)> = conn
        .query_row(
            "SELECT subject_hash, chain_id FROM paid_wallet_links WHERE operation_id = ?1",
            rusqlite::params![operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((subject_hash, chain_id)) => wallet_link::create_or_get_challenge(
            conn,
            operation_id,
            &subject_hash,
            chain_id,
            chrono::Utc::now(),
        )
        .map(Some),
        None => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
struct SettleRequest {
    operation_id: String,
    binding: OperationBinding,
    authorization: PaymentAuthorization,
}

/// POST /api/settle — proxy settlement without retaining the raw wallet
/// authorization. Only the signed provider receipt is persisted for resume.
async fn settle_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<SettleRequest>,
) -> Response {
    if req.operation_id.is_empty() || req.operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    if req.binding.operation_id != req.operation_id {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "operation_id does not match payment binding",
        );
    }
    if !same_address(&req.binding.payer_wallet, &req.authorization.payer_wallet)
        || !same_address(
            &req.binding.payer_wallet,
            &req.authorization.authorization.authorization.from,
        )
    {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "authorization payer does not match payment binding",
        );
    }
    let Some(cfg) = state.universal_paywall.clone() else {
        return error_resp(
            StatusCode::SERVICE_UNAVAILABLE,
            "universal paywall not configured",
        );
    };
    let client = UniversalPaywallClient::new(cfg);
    let existing_receipt = match state.store.lock() {
        Ok(store) => match paid_operation::get(store.conn(), &req.operation_id) {
            Ok(Some(operation)) => operation.provider_receipt_json,
            Ok(None) => return error_resp(StatusCode::NOT_FOUND, "operation not found"),
            Err(_) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "payment state unavailable",
                )
            }
        },
        Err(_) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "payment state unavailable",
            )
        }
    };
    if let Some(receipt_json) = existing_receipt {
        let receipt = match serde_json::from_str::<PaymentReceipt>(&receipt_json) {
            Ok(receipt) => receipt,
            Err(_) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "stored receipt unavailable",
                )
            }
        };
        return settled_response(receipt);
    }

    // The browser must settle precisely the provider-issued immutable quote,
    // never a binding reconstructed from URL parameters or client input.
    let quote = match client.get_quote_by_operation_id(&req.operation_id).await {
        Ok(quote) => quote,
        Err(error) => {
            tracing::warn!(operation_id = req.operation_id, error = %error, "facilitator quote lookup failed before settlement");
            return error_resp(StatusCode::BAD_REQUEST, "payment quote unavailable");
        }
    };
    if quote.binding != req.binding {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "payment binding does not match provider quote",
        );
    }
    let marked = match state.store.lock() {
        Ok(store) => paid_operation::mark_payment_authorizing(
            store.conn(),
            &req.operation_id,
            &chrono::Utc::now().to_rfc3339(),
        ),
        Err(_) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "payment state unavailable",
            )
        }
    };
    if let Err(error) = marked {
        tracing::warn!(operation_id = req.operation_id, error = %error, "payment operation cannot be settled in its current state");
        return error_resp(
            StatusCode::CONFLICT,
            "payment operation is not ready to settle",
        );
    }
    match client
        .settle_exact(&req.binding, &req.authorization.authorization)
        .await
    {
        Ok(receipt) => {
            let receipt_json = match serde_json::to_string(&receipt) {
                Ok(value) => value,
                Err(_) => {
                    return error_resp(StatusCode::INTERNAL_SERVER_ERROR, "receipt unavailable")
                }
            };
            let persisted = match state.store.lock() {
                Ok(store) => paid_operation::record_provider_receipt(
                    store.conn(),
                    &req.operation_id,
                    &receipt_json,
                    &chrono::Utc::now().to_rfc3339(),
                ),
                Err(_) => {
                    return error_resp(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "payment state unavailable",
                    )
                }
            };
            if let Err(error) = persisted {
                tracing::error!(operation_id = req.operation_id, error = %error, "persist settled provider receipt failed");
                // A concurrent duplicate browser callback can settle the same
                // provider operation while this request is in flight. The
                // receipt is immutable; return the winner's durable receipt
                // instead of turning an already-paid operation into an error.
                let concurrent_receipt = match state.store.lock() {
                    Ok(store) => paid_operation::get(store.conn(), &req.operation_id)
                        .ok()
                        .flatten()
                        .and_then(|operation| operation.provider_receipt_json),
                    Err(_) => None,
                };
                if let Some(receipt_json) = concurrent_receipt {
                    if let Ok(receipt) = serde_json::from_str::<PaymentReceipt>(&receipt_json) {
                        return settled_response(receipt);
                    }
                }
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "payment state unavailable",
                );
            }
            settled_response(receipt)
        }
        Err(e) => {
            tracing::warn!(operation_id = req.operation_id, error = %e, "facilitator settle failed");
            error_resp(StatusCode::BAD_GATEWAY, "settlement failed")
        }
    }
}

fn same_address(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn settled_response(receipt: PaymentReceipt) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "settled",
            "operation_id": receipt.operation_id,
            "receipt": receipt,
        })),
    )
        .into_response()
}

/// GET /api/operations/:operation_id — return durable, non-secret operation
/// state. The operation ID is an unguessable capability issued only with the
/// client-signed artifact; raw EIP-3009 payloads are never returned.
async fn operation_status_handler(
    State(state): State<Arc<McpState>>,
    Path(operation_id): Path<String>,
    Query(query): Query<ResumeQuery>,
) -> Response {
    if operation_id.is_empty() || operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    let operation = match state.store.lock() {
        Ok(store) => match paid_operation::get(store.conn(), &operation_id) {
            Ok(Some(operation)) => operation,
            Ok(None) => return error_resp(StatusCode::NOT_FOUND, "operation not found"),
            Err(_) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "payment state unavailable",
                )
            }
        },
        Err(_) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "payment state unavailable",
            )
        }
    };
    let Some(config) = state.universal_paywall.clone() else {
        return error_resp(
            StatusCode::SERVICE_UNAVAILABLE,
            "universal paywall not configured",
        );
    };
    let (Some(quote_id), Some(expires_at)) = (&operation.quote_id, &operation.quote_expires_at)
    else {
        return error_resp(StatusCode::CONFLICT, "operation has no resumable quote");
    };
    if chrono::DateTime::parse_from_rfc3339(expires_at)
        .map(|expires| expires <= chrono::Utc::now())
        .unwrap_or(true)
    {
        return error_resp(StatusCode::GONE, "resume capability expired");
    }
    let expected =
        UniversalPaywallClient::new(config).resume_token(&operation_id, quote_id, expires_at);
    if query.resume_token.len() != expected.len() || query.resume_token != expected {
        return error_resp(StatusCode::UNAUTHORIZED, "invalid resume capability");
    }
    (StatusCode::OK, Json(serde_json::json!({
        "operation_id": operation.operation_id,
        "state": operation.state.as_str(),
        "quote_id": operation.quote_id,
        "expires_at": operation.quote_expires_at,
        "receipt": operation.provider_receipt_json.and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok()),
    }))).into_response()
}

#[derive(Debug, Serialize)]
struct ChainConfigResponse {
    chain_id: u64,
    name: String,
    rpc_url: String,
    native_currency: NativeCurrencyResponse,
    eip712_name: String,
    eip712_version: String,
}

#[derive(Debug, Serialize)]
struct NativeCurrencyResponse {
    name: String,
    symbol: String,
    decimals: u8,
}

/// GET /api/chains/:chain_id — chain metadata for wallet_addEthereumChain.
async fn chain_handler(State(state): State<Arc<McpState>>, Path(chain_id): Path<u64>) -> Response {
    let configured_chain_id = state
        .universal_paywall
        .as_ref()
        .and_then(|c| parse_eip155(&c.network));
    if configured_chain_id != Some(chain_id) {
        return error_resp(StatusCode::NOT_FOUND, "chain not configured");
    }
    if state.approval_chain_rpc_url.is_empty() {
        return error_resp(StatusCode::NOT_FOUND, "chain RPC not configured");
    }
    let name = if state.approval_chain_name.is_empty() {
        "Unknown".to_string()
    } else {
        state.approval_chain_name.clone()
    };
    let resp = ChainConfigResponse {
        chain_id,
        name,
        rpc_url: state.approval_chain_rpc_url.clone(),
        native_currency: NativeCurrencyResponse {
            name: state.approval_chain_currency_symbol.clone(),
            symbol: state.approval_chain_currency_symbol.clone(),
            decimals: state.approval_chain_currency_decimals,
        },
        eip712_name: state.universal_paywall_eip712_name.clone(),
        eip712_version: state.universal_paywall_eip712_version.clone(),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "approval-mock-signer")]
struct MockSignRequest {
    typed_data: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[cfg(feature = "approval-mock-signer")]
struct MockSignResponse {
    signature: String,
}

/// POST /api/mock-sign — test-only EIP-712 signer.
#[cfg(feature = "approval-mock-signer")]
async fn mock_sign_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<MockSignRequest>,
) -> Response {
    use alloy_dyn_abi::TypedData;
    use alloy_primitives::hex;
    use alloy_signer::Signer;
    use alloy_signer_local::PrivateKeySigner;

    let Some(key_hex) = state.approval_mock_signer.clone() else {
        return error_resp(StatusCode::NOT_FOUND, "mock signer not configured");
    };

    let typed_data: TypedData = match serde_json::from_value(req.typed_data) {
        Ok(t) => t,
        Err(e) => {
            return error_resp(StatusCode::BAD_REQUEST, &format!("invalid typed data: {e}"));
        }
    };

    let key_bytes = match hex::decode(key_hex.trim_start_matches("0x")) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid mock signer key: {e}"),
            );
        }
    };
    let signer = match PrivateKeySigner::from_slice(&key_bytes) {
        Ok(s) => s,
        Err(e) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid mock signer key: {e}"),
            );
        }
    };

    match signer.sign_dynamic_typed_data(&typed_data).await {
        Ok(sig) => {
            let signature = hex::encode_prefixed(sig.as_bytes());
            (StatusCode::OK, Json(MockSignResponse { signature })).into_response()
        }
        Err(e) => error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("signing failed: {e}"),
        ),
    }
}

#[cfg(not(feature = "approval-mock-signer"))]
async fn mock_sign_handler(
    State(_state): State<Arc<McpState>>,
    Json(_req): Json<serde_json::Value>,
) -> Response {
    error_resp(StatusCode::NOT_FOUND, "mock signer not compiled in")
}

/// Build the approval-page router. Static assets are served by the caller
/// (see `main.rs`) so the UI paths do not shadow application routes.
pub fn router(state: Arc<McpState>) -> Router<()> {
    Router::new()
        .route("/approve", get(approve_page_handler))
        .route("/api/quote/{operation_id}", get(quote_handler))
        .route(
            "/api/wallet-link/{operation_id}",
            get(wallet_link_get_handler).post(wallet_link_post_handler),
        )
        .route("/api/settle", post(settle_handler))
        .route(
            "/api/operations/{operation_id}",
            get(operation_status_handler),
        )
        .route("/api/chains/{chain_id}", get(chain_handler))
        .route("/api/mock-sign", post(mock_sign_handler))
        .with_state(state)
}
