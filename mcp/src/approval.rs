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
    let operation = match state.store.lock() {
        Ok(store) => match paid_operation::get(store.conn(), &req.operation_id) {
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
    if operation.artifact_hash != req.binding.artifact_hash
        || operation.subject_hash != req.binding.payer_subject
        || !operation
            .payer_wallet
            .as_deref()
            .is_some_and(|wallet| same_address(wallet, &req.binding.payer_wallet))
    {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "payment binding differs from stored operation",
        );
    }
    match paid_operation::settled_receipt(&operation) {
        Ok(Some(receipt)) => return settled_response(receipt),
        Ok(None) => {}
        Err(_) => {
            return error_resp(
                StatusCode::SERVICE_UNAVAILABLE,
                "stored receipt requires reconciliation",
            )
        }
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
    if quote.binding != req.binding
        || operation.quote_id.as_deref() != Some(quote.quote_id.as_str())
        || operation.binding_digest.as_deref() != Some(quote.binding_digest.as_str())
    {
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
        .settle_exact(
            &req.binding,
            &quote.binding_digest,
            &req.authorization.authorization,
        )
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
                        .and_then(|operation| {
                            paid_operation::settled_receipt(&operation).ok().flatten()
                        }),
                    Err(_) => None,
                };
                if let Some(receipt) = concurrent_receipt {
                    return settled_response(receipt);
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

/// Financial settlement and external delivery are independent observations.
/// Call only after validating the operation's resume capability.
fn delivery_status_fields(
    conn: &rusqlite::Connection,
    operation: &paid_operation::PaidOperation,
) -> anyhow::Result<serde_json::Value> {
    if let Some(delivery) = crate::delivery_operation::get(conn, &operation.operation_id)? {
        let action = match delivery.delivery_status.as_str() {
            "verified" => "none",
            "failed_terminal" => "review_remedy",
            _ if delivery.payment_status == "required" => "complete_payment_then_resubmit",
            _ => "resubmit_original_signed_bytes",
        };
        return Ok(serde_json::json!({
            "payment_status": delivery.payment_status,
            "delivery_status": delivery.delivery_status,
            "retry_action": action,
            "error_code": delivery.error_code,
            "remedy_reference": delivery.remedy_reference,
        }));
    }
    let attempt: Option<(String, Option<String>)> = conn.query_row(
        "SELECT state, last_error FROM paid_artifact_delivery_attempts WHERE correlation_id = ?1",
        [&operation.operation_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let terminal = operation.state == paid_operation::PaidOperationState::Abandoned
        || operation.state == paid_operation::PaidOperationState::RefundPending
        || attempt
            .as_ref()
            .is_some_and(|(state, _)| state == "abandoned");
    let verified = operation.state == paid_operation::PaidOperationState::Anchored
        || attempt
            .as_ref()
            .is_some_and(|(state, _)| state == "completed");
    let settled = paid_operation::settled_receipt(operation)?.is_some();
    let payment = if terminal && settled {
        "remedy_pending"
    } else if settled {
        "settled"
    } else {
        "required"
    };
    let delivery = if verified {
        "verified"
    } else if terminal {
        "failed_terminal"
    } else if operation.state == paid_operation::PaidOperationState::DeliveryRetryable {
        "failed_retryable"
    } else {
        match attempt.as_ref().map(|(state, _)| state.as_str()) {
            Some("delivery_retryable") => "failed_retryable",
            Some("uploading") => "uploading",
            _ => "verification_pending",
        }
    };
    Ok(serde_json::json!({
        "payment_status": payment,
        "delivery_status": delivery,
        "retry_action": if verified { "none" } else if terminal { "review_remedy" }
            else if settled { "resubmit_original_signed_bytes" } else { "complete_payment_then_resubmit" },
        "error_code": attempt.and_then(|(_, error)| error),
        "remedy_reference": serde_json::Value::Null,
    }))
}

/// GET /api/operations/:operation_id — return durable, non-secret operation
/// state after validating the resume capability. Raw EIP-3009 payloads are
/// never returned.
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
    let delivery = match state.store.lock() {
        Ok(store) => match delivery_status_fields(store.conn(), &operation) {
            Ok(fields) => fields,
            Err(_) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "delivery state unavailable",
                )
            }
        },
        Err(_) => {
            return error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "delivery state unavailable",
            )
        }
    };
    (StatusCode::OK, Json(serde_json::json!({
        "operation_id": operation.operation_id,
        "state": operation.state.as_str(),
        "quote_id": operation.quote_id,
        "expires_at": operation.quote_expires_at,
        "receipt": operation.provider_receipt_json.and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok()),
        "payment_status": delivery["payment_status"],
        "delivery_status": delivery["delivery_status"],
        "retry_action": delivery["retry_action"],
        "error_code": delivery["error_code"],
        "remedy_reference": delivery["remedy_reference"],
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

#[cfg(test)]
mod delivery_status_tests {
    use super::*;

    fn fixture() -> (rusqlite::Connection, paid_operation::PaidOperation) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        paid_operation::migrate_paid_operations(&conn).unwrap();
        crate::paid_artifact::migrate_paid_artifact_staging(&conn).unwrap();
        crate::delivery_operation::migrate(&conn).unwrap();
        let operation = paid_operation::create_or_get(
            &conn,
            paid_operation::NewPaidOperation {
                operation_id: "status-op",
                subject_hash: "owner",
                artifact_hash: "digest",
                created_at: "2026-10-02T00:00:00Z",
            },
        )
        .unwrap();
        (conn, operation)
    }

    #[test]
    fn settled_legacy_failure_is_not_reported_as_unpaid_or_delivered() {
        let (conn, mut operation) = fixture();
        let payload = serde_json::json!({"version":1,"service_id":"mock-provider","operation_id":"status-op","scheme":"exact","binding_digest":"binding","payer_wallet":"wallet","amount":"1000","asset":"asset","network":"network","pay_to":"merchant","settlement_tx":"tx","settled_at":"2026-10-02T00:00:00Z"});
        let mut receipt = payload.clone();
        receipt.as_object_mut().unwrap().remove("version");
        receipt.as_object_mut().unwrap().remove("service_id");
        receipt["status"] = serde_json::json!("settled");
        receipt["receipt"] = serde_json::json!({"payload":payload,"signature":{"algorithm":"ed25519","key_id":"mock","value":"signature"}});
        operation.provider_receipt_json = Some(receipt.to_string());
        operation.binding_digest = Some("binding".into());
        operation.payer_wallet = Some("wallet".into());
        operation.state = paid_operation::PaidOperationState::DeliveryRetryable;
        let status = delivery_status_fields(&conn, &operation).unwrap();
        assert_eq!(status["payment_status"], "settled");
        assert_eq!(status["delivery_status"], "failed_retryable");
        assert_eq!(status["retry_action"], "resubmit_original_signed_bytes");
        operation.state = paid_operation::PaidOperationState::RefundPending;
        let status = delivery_status_fields(&conn, &operation).unwrap();
        assert_eq!(status["payment_status"], "remedy_pending");
        assert_eq!(status["delivery_status"], "failed_terminal");
    }

    #[test]
    fn malformed_legacy_receipt_is_not_reported_as_settled() {
        let (conn, mut operation) = fixture();
        operation.provider_receipt_json = Some("{}".into());
        operation.state = paid_operation::PaidOperationState::PaymentReady;
        assert!(delivery_status_fields(&conn, &operation).is_err());
    }

    #[test]
    fn durable_delivery_and_remedy_override_stale_aggregate_state() {
        let (conn, operation) = fixture();
        use crate::delivery_operation as delivery;
        delivery::bind(
            &conn,
            "status-op",
            "owner",
            &"a".repeat(64),
            12,
            "arweave",
            "locator",
            true,
            0,
        )
        .unwrap();
        delivery::acquire(&conn, "status-op", "lease", 1).unwrap();
        delivery::payment_settled(&conn, "status-op", "lease", 2).unwrap();
        delivery::release(
            &conn,
            "status-op",
            "lease",
            "failed_terminal",
            Some("provider_unavailable"),
            3,
        )
        .unwrap();
        delivery::record_remedy(&conn, "status-op", "credit:test-reference", 4).unwrap();
        let status = delivery_status_fields(&conn, &operation).unwrap();
        assert_eq!(status["payment_status"], "remedied");
        assert_eq!(status["delivery_status"], "failed_terminal");
        assert_eq!(status["remedy_reference"], "credit:test-reference");
        assert_eq!(status["error_code"], "provider_unavailable");
    }
}
