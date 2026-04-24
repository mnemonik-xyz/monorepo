//! Payment gating — pre-funded balance (API key) and x402 per-call (autonomous agents).
//!
//! Two payment paths:
//!   • balance  — human users top up an API key; Cursor/Claude Desktop send
//!                `Authorization: Bearer mnm_<key>` on every MCP request.
//!   • x402     — autonomous agents pay per-call via a USDC Solana transfer and
//!                present the tx sig in `X-Payment: <json>` on the retry request.
//!   • both     — balance checked first; x402 accepted as fallback.
//!   • none     — open access (development / self-hosted).

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use mnemonic_core::storage::SqliteStore;
use mnemonic_core::solana::SolanaClient;
use crate::db;

// ── x402 wire types ──────────────────────────────────────────────────────────

/// Payload sent in the `X-Payment` header by the agent.
#[derive(Debug, Deserialize)]
pub struct X402PaymentProof {
    pub tx_sig: String,
    /// "solana-mainnet" | "solana-devnet"
    pub network: String,
}

/// Body returned with HTTP 402 to describe what payment is required.
#[derive(Debug, Serialize)]
pub struct X402Response {
    #[serde(rename = "x402Version")]
    pub x402_version: u8,
    pub accepts: Vec<PaymentOption>,
}

#[derive(Debug, Serialize)]
pub struct PaymentOption {
    /// "exact" — caller must send exactly this token + amount.
    pub scheme: String,
    pub network: String,
    /// Amount in smallest units (micro-USDC), as a decimal string.
    #[serde(rename = "maxAmountRequired")]
    pub max_amount_required: String,
    /// SPL mint address.
    pub asset: String,
    /// Treasury public key (recipient).
    #[serde(rename = "payTo")]
    pub pay_to: String,
    pub description: String,
}

// ── Gate result ──────────────────────────────────────────────────────────────

pub enum PaymentGate {
    /// Payment verified (or not required). Inner value = api_key if balance mode.
    Proceed(Option<String>),
    /// Return HTTP 402 with this body.
    NeedPayment(X402Response),
    /// Bad credentials / insufficient balance — return 401/402 error message.
    Unauthorized(String),
}

// ── Header helpers ───────────────────────────────────────────────────────────

/// Extract API key from `Authorization: Bearer mnm_...` header.
pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

/// Decode x402 payment proof from `X-Payment` header.
/// Accepts raw JSON or base64-encoded JSON.
pub fn extract_x402_proof(headers: &HeaderMap) -> Option<X402PaymentProof> {
    let raw = headers.get("x-payment").and_then(|v| v.to_str().ok())?;

    // Try raw JSON first
    if let Ok(p) = serde_json::from_str::<X402PaymentProof>(raw) {
        return Some(p);
    }
    // Fallback: base64-encoded JSON (Coinbase CDK sends this)
    if let Ok(decoded) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw) {
        if let Ok(p) = serde_json::from_slice::<X402PaymentProof>(&decoded) {
            return Some(p);
        }
    }
    None
}

// ── Main gate function ───────────────────────────────────────────────────────

/// Check payment for a paid tool call.
///
/// Called before executing `mnemonic_sign_memory` when `payment_mode != "none"`.
/// Returns `PaymentGate::Proceed(api_key)` if the caller may proceed,
/// otherwise the appropriate rejection.
pub async fn check_payment(
    headers: &HeaderMap,
    mode: &str,
    store: &std::sync::Mutex<SqliteStore>,
    solana: &SolanaClient,
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
) -> PaymentGate {
    match mode {
        "none" => PaymentGate::Proceed(None),

        "balance" => check_balance(headers, store, cost),

        "x402" => check_x402(headers, solana, store, treasury, usdc_mint, cost).await,

        "both" => {
            // If an API key header is present, try balance first
            if extract_api_key(headers).is_some() {
                match check_balance(headers, store, cost) {
                    PaymentGate::Proceed(k) => return PaymentGate::Proceed(k),
                    // fall through to x402
                    _ => {}
                }
            }
            // Otherwise gate via x402
            check_x402(headers, solana, store, treasury, usdc_mint, cost).await
        }

        unknown => {
            tracing::error!("Unknown PAYMENT_MODE={unknown:?} — rejecting request (fail-closed)");
            PaymentGate::Unauthorized(format!(
                "server misconfiguration: unknown PAYMENT_MODE={unknown:?}. Valid: none, balance, x402, both"
            ))
        }
    }
}

// ── Balance path ─────────────────────────────────────────────────────────────

fn check_balance(
    headers: &HeaderMap,
    store: &std::sync::Mutex<SqliteStore>,
    cost: i64,
) -> PaymentGate {
    let key = match extract_api_key(headers) {
        Some(k) => k,
        None => return PaymentGate::Unauthorized("missing Authorization: Bearer <api_key>".into()),
    };

    let store = store.lock().unwrap();
    match db::get_balance(&store, &key) {
        Ok(Some(bal)) if bal >= cost => PaymentGate::Proceed(Some(key)),
        Ok(Some(bal)) => PaymentGate::Unauthorized(
            format!("insufficient balance: have {bal} micro-USDC, need {cost}")
        ),
        Ok(None) => PaymentGate::Unauthorized("api key not found".into()),
        Err(e) => PaymentGate::Unauthorized(format!("balance lookup failed: {e}")),
    }
}

// ── x402 path ────────────────────────────────────────────────────────────────

async fn check_x402(
    headers: &HeaderMap,
    solana: &SolanaClient,
    store: &std::sync::Mutex<SqliteStore>,
    treasury: &str,
    usdc_mint: &str,
    cost: i64,
) -> PaymentGate {
    let proof = match extract_x402_proof(headers) {
        Some(p) => p,
        None => {
            // No payment header — return 402 payment required
            return PaymentGate::NeedPayment(x402_required(treasury, usdc_mint, cost,
                "mnemonic_sign_memory attestation fee"));
        }
    };

    // Verify the Solana USDC transfer
    match verify_usdc_transfer(solana, &proof.tx_sig, treasury, usdc_mint, cost as u64).await {
        Ok(Some(_)) => {}
        Ok(None) => return PaymentGate::Unauthorized(
            format!("x402 payment not valid: tx {} does not transfer >= {cost} micro-USDC to treasury", proof.tx_sig)
        ),
        Err(e) => return PaymentGate::Unauthorized(format!("x402 verification error: {e}")),
    }

    // Mark nonce to prevent replay
    {
        let store = store.lock().unwrap();
        if let Err(e) = db::mark_x402_nonce(&store, &proof.tx_sig) {
            return PaymentGate::Unauthorized(e.to_string());
        }
    }

    PaymentGate::Proceed(None)
}

// ── Builder ──────────────────────────────────────────────────────────────────

fn x402_required(treasury: &str, usdc_mint: &str, cost: i64, description: &str) -> X402Response {
    X402Response {
        x402_version: 1,
        accepts: vec![PaymentOption {
            scheme: "exact".into(),
            network: "solana-mainnet".into(),
            max_amount_required: cost.to_string(),
            asset: usdc_mint.to_string(),
            pay_to: treasury.to_string(),
            description: description.to_string(),
        }],
    }
}

// ── USDC transfer verification ───────────────────────────────────────────────

/// Verify that `tx_sig` transfers at least `min_amount` micro-USDC of `usdc_mint`
/// to `recipient`.  Returns the actual amount transferred (>= min_amount) on
/// success, or `Ok(None)` if the transfer is absent / insufficient.
///
/// This is a payment concern and lives here (not in `mnemonic_core::solana`):
/// core knows chain primitives; the `USDC vs recipient` policy is mcp's.
pub async fn verify_usdc_transfer(
    client: &SolanaClient,
    tx_sig: &str,
    recipient: &str,
    usdc_mint: &str,
    min_amount: u64,
) -> anyhow::Result<Option<u64>> {
    let result = client.rpc("getTransaction", serde_json::json!([
        tx_sig,
        {"encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}
    ])).await?;

    if result.is_null() {
        return Ok(None);
    }

    // Reject failed transactions
    if !result["meta"]["err"].is_null() {
        return Ok(None);
    }

    // Walk postTokenBalances looking for recipient + mint with increased balance
    let pre = result["meta"]["preTokenBalances"].as_array();
    let post = result["meta"]["postTokenBalances"].as_array();

    if let (Some(pre_balances), Some(post_balances)) = (pre, post) {
        for post_entry in post_balances {
            let owner = post_entry["owner"].as_str().unwrap_or("");
            let mint  = post_entry["mint"].as_str().unwrap_or("");
            if owner != recipient || mint != usdc_mint {
                continue;
            }
            let post_amount: u64 = post_entry["uiTokenAmount"]["amount"]
                .as_str().unwrap_or("0").parse().unwrap_or(0);

            let account_index = post_entry["accountIndex"].as_u64().unwrap_or(u64::MAX);
            let pre_amount: u64 = pre_balances.iter()
                .find(|e| e["accountIndex"].as_u64() == Some(account_index))
                .and_then(|e| e["uiTokenAmount"]["amount"].as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let delta = post_amount.saturating_sub(pre_amount);
            if delta >= min_amount {
                return Ok(Some(delta));
            }
        }
    }

    Ok(None)
}
