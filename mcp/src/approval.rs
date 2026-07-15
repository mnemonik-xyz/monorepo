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
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::McpState;
use crate::universal_paywall::{OperationBinding, PaymentAuthorization, UniversalPaywallClient};

const MAX_OPERATION_ID_LEN: usize = 128;

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
struct SettleRequest {
    operation_id: String,
    binding: OperationBinding,
    authorization: PaymentAuthorization,
}

/// POST /api/settle — proxy settlement to the facilitator and remember the
/// signed authorization so the e2e harness can pick it up.
async fn settle_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<SettleRequest>,
) -> Response {
    if req.operation_id.is_empty() || req.operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    let Some(cfg) = state.universal_paywall.clone() else {
        return error_resp(
            StatusCode::SERVICE_UNAVAILABLE,
            "universal paywall not configured",
        );
    };
    let client = UniversalPaywallClient::new(cfg);
    match client
        .settle_exact(&req.binding, &req.authorization.authorization)
        .await
    {
        Ok(_receipt) => {
            state
                .approval_authorizations
                .insert(req.operation_id.clone(), req.authorization);
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Err(e) => {
            tracing::warn!(operation_id = req.operation_id, error = %e, "facilitator settle failed");
            error_resp(StatusCode::BAD_GATEWAY, "settlement failed")
        }
    }
}

#[derive(Debug, Deserialize)]
struct AuthorizationQuery {
    operation_id: String,
}

/// GET /api/authorization?operation_id=... — retrieve a stored authorization.
async fn authorization_handler(
    State(state): State<Arc<McpState>>,
    Query(query): Query<AuthorizationQuery>,
) -> Response {
    if query.operation_id.is_empty() || query.operation_id.len() > MAX_OPERATION_ID_LEN {
        return error_resp(StatusCode::BAD_REQUEST, "invalid operation_id");
    }
    match state.approval_authorizations.get(&query.operation_id) {
        Some(entry) => (StatusCode::OK, Json(entry.value().clone())).into_response(),
        None => error_resp(StatusCode::NOT_FOUND, "authorization not found"),
    }
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
        .route("/api/settle", post(settle_handler))
        .route("/api/authorization", get(authorization_handler))
        .route("/api/chains/{chain_id}", get(chain_handler))
        .route("/api/mock-sign", post(mock_sign_handler))
        .with_state(state)
}
