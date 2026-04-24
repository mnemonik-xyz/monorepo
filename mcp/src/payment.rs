//! Payment gating + payment-related database helpers.
//!
//! This file owns all payment concerns for the MCP server:
//!   - Payment-mode gating (`check_payment` and path selectors).
//!   - API key & balance lifecycle in SQLite (`create_api_key`,
//!     `get_balance`, `deduct_balance`, `credit_deposit`, `get_owner_pubkey`).
//!   - x402 nonce replay protection (`mark_x402_nonce`).
//!   - P&L cost accounting (`record_attestation_cost`, `get_pnl_stats`).
//!   - Standalone `verify_usdc_transfer` over `&SolanaClient` (moved here in
//!     Task 8; the USDC-vs-recipient policy is payment-layer, not chain-layer).
//!
//! Two payment paths:
//!   - balance — human users top up an API key; Cursor/Claude Desktop send
//!     `Authorization: Bearer mnm_<key>` on every MCP request.
//!   - x402 — autonomous agents pay per-call via a USDC Solana transfer and
//!     present the tx sig in `X-Payment: <json>` on the retry request.
//!   - both — balance checked first; x402 accepted as fallback.
//!   - none — open access (development / self-hosted).

use axum::http::HeaderMap;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::SqliteStore;

// ── x402 wire types ──────────────────────────────────────────────────────────

/// Payload sent in the `X-Payment` header by the agent.
#[derive(Debug, Deserialize)]
pub struct X402PaymentProof {
    pub tx_sig: String,
    /// "solana-mainnet" | "solana-devnet" — deserialized for protocol
    /// compliance. Currently not inspected; verification is network-agnostic
    /// via the configured Solana RPC URL.
    #[allow(dead_code)]
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
                if let PaymentGate::Proceed(k) = check_balance(headers, store, cost) {
                    return PaymentGate::Proceed(k);
                }
                // fall through to x402 on failure
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
    match get_balance(&store, &key) {
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
        if let Err(e) = mark_x402_nonce(&store, &proof.tx_sig) {
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

// ── Payment DB helpers (operate on SqliteStore from mnemonic-core) ───────────
//
// These are payment concerns (API key management, balance, P&L) and
// intentionally live in the MCP server, not in core. They operate on the
// public `conn()` accessor of `mnemonic_core::storage::SqliteStore`.

/// Aggregated profit-and-loss statistics.
#[derive(Debug, serde::Serialize)]
pub struct PnlStats {
    pub period_days: u64,
    pub attestations: i64,
    pub earned_micro_usdc: i64,
    pub cost_sol_lamports: i64,
    pub cost_micro_usdc_equiv: i64,
    pub net_micro_usdc: i64,
    pub margin_pct: f64,
    pub avg_sol_price_usdc: f64,
}

/// Create a new API key with zero balance. Returns the key.
pub fn create_api_key(store: &SqliteStore, owner_pubkey: &str) -> anyhow::Result<String> {
    let key = format!("mnm_{}", hex::encode(random_bytes::<24>()));
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "INSERT INTO api_keys (api_key, owner_pubkey, balance_micro_usdc, created_at) VALUES (?,?,0,?)",
        params![key, owner_pubkey, now],
    )?;
    Ok(key)
}

/// Get the owner pubkey for an API key. Returns None if key not found.
pub fn get_owner_pubkey(store: &SqliteStore, api_key: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = store.conn().prepare(
        "SELECT owner_pubkey FROM api_keys WHERE api_key = ?"
    )?;
    let mut rows = stmt.query(params![api_key])?;
    Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
}

/// Get balance in micro-USDC for an API key. Returns None if key not found.
pub fn get_balance(store: &SqliteStore, api_key: &str) -> anyhow::Result<Option<i64>> {
    let mut stmt = store.conn().prepare(
        "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?"
    )?;
    let mut rows = stmt.query(params![api_key])?;
    Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
}

/// Deduct `amount` from balance. Returns Err if insufficient funds or key not found.
pub fn deduct_balance(store: &SqliteStore, api_key: &str, amount: i64, description: &str) -> anyhow::Result<()> {
    let balance = get_balance(store, api_key)?
        .ok_or_else(|| anyhow::anyhow!("api key not found"))?;
    if balance < amount {
        anyhow::bail!("insufficient balance: have {balance} micro-USDC, need {amount}");
    }
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "UPDATE api_keys SET balance_micro_usdc = balance_micro_usdc - ?, last_used_at = ? WHERE api_key = ?",
        params![amount, now, api_key],
    )?;
    store.conn().execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, description, created_at) VALUES (?,?,?,'charge',?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, description, now],
    )?;
    Ok(())
}

/// Credit a deposit. Returns new balance.
pub fn credit_deposit(store: &SqliteStore, api_key: &str, amount: i64, tx_sig: &str) -> anyhow::Result<i64> {
    let conn = store.conn();
    let now = chrono::Utc::now().to_rfc3339();
    let existing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM payment_events WHERE tx_sig = ?",
        params![tx_sig], |r| r.get(0),
    )?;
    if existing > 0 {
        anyhow::bail!("deposit tx already applied: {tx_sig}");
    }
    conn.execute(
        "UPDATE api_keys SET balance_micro_usdc = balance_micro_usdc + ? WHERE api_key = ?",
        params![amount, api_key],
    )?;
    if conn.changes() == 0 {
        anyhow::bail!("api key not found: {api_key}");
    }
    conn.execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, tx_sig, description, created_at) VALUES (?,?,?,'deposit',?,?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, tx_sig, "USDC deposit", now],
    )?;
    let new_balance: i64 = conn.query_row(
        "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?",
        params![api_key], |r| r.get(0),
    )?;
    Ok(new_balance)
}

/// Record an x402 tx sig as used (prevents replay). Returns Err if already used.
pub fn mark_x402_nonce(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = store.conn().execute(
        "INSERT INTO x402_nonces (tx_sig, used_at) VALUES (?,?)",
        params![tx_sig, now],
    );
    match result {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
            anyhow::bail!("x402 payment already used: {tx_sig}")
        }
        Err(e) => Err(e.into()),
    }
}

/// Record actual server costs alongside each completed attestation.
pub fn record_attestation_cost(
    store: &SqliteStore,
    attestation_id: &str,
    irys_lamports: u64,
    sol_tx_fee_lamports: u64,
    sol_price_usdc: f64,
    earned_micro_usdc: i64,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "INSERT OR IGNORE INTO attestation_costs
         (attestation_id, irys_cost_lamports, sol_tx_fee_lamports, sol_price_usdc, earned_micro_usdc, created_at)
         VALUES (?,?,?,?,?,?)",
        params![
            attestation_id,
            irys_lamports as i64,
            sol_tx_fee_lamports as i64,
            sol_price_usdc,
            earned_micro_usdc,
            now,
        ],
    )?;
    Ok(())
}

/// Aggregate P&L statistics over the last `days` days.
pub fn get_pnl_stats(store: &SqliteStore, days: u64) -> anyhow::Result<PnlStats> {
    let interval = format!("-{days} days");
    let row = store.conn().query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(earned_micro_usdc), 0),
            COALESCE(SUM(irys_cost_lamports + sol_tx_fee_lamports), 0),
            COALESCE(SUM((irys_cost_lamports + sol_tx_fee_lamports) * sol_price_usdc / 1000.0), 0.0),
            COALESCE(AVG(sol_price_usdc), 0.0)
         FROM attestation_costs
         WHERE created_at > datetime('now', ?1)",
        params![interval],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        },
    )?;

    let (attestations, earned, cost_lamports, cost_usdc_equiv, avg_sol) = row;
    let cost_micro_usdc = cost_usdc_equiv.ceil() as i64;
    let net = earned - cost_micro_usdc;
    let margin_pct = if earned > 0 {
        (net as f64 / earned as f64) * 100.0
    } else {
        0.0
    };

    Ok(PnlStats {
        period_days: days,
        attestations,
        earned_micro_usdc: earned,
        cost_sol_lamports: cost_lamports,
        cost_micro_usdc_equiv: cost_micro_usdc,
        net_micro_usdc: net,
        margin_pct,
        avg_sol_price_usdc: avg_sol,
    })
}

/// Cryptographically secure random bytes using OS entropy.
fn random_bytes<const N: usize>() -> [u8; N] {
    use std::io::Read;
    let mut out = [0u8; N];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut out);
    } else {
        use std::sync::atomic::{AtomicU64, Ordering};
        use sha2::{Digest, Sha256};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let seed = format!(
            "{}:{}:{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
            std::process::id(),
            CTR.fetch_add(1, Ordering::Relaxed),
        );
        let hash = Sha256::digest(seed.as_bytes());
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = hash[i % 32];
        }
    }
    out
}
