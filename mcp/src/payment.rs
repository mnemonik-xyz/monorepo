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

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        Ok(Some(bal)) => PaymentGate::Unauthorized(format!(
            "insufficient balance: have {bal} micro-USDC, need {cost}"
        )),
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
            return PaymentGate::NeedPayment(x402_required(
                treasury,
                usdc_mint,
                cost,
                "mnemonic_sign_memory attestation fee",
            ));
        }
    };

    // T3 round-2 — replay-detect WITHOUT consuming. If this nonce has
    // already been consumed (by a successful delivery on an earlier
    // request), reject. The actual `INSERT INTO x402_nonces` happens
    // AFTER `confirm_delivery_or_demote` succeeds (see
    // `consume_x402_nonce_after_success` below) so a delivery failure
    // leaves the nonce reusable — the caller's USDC payment is not
    // forfeit when the operator's anchor isn't proved retrievable.
    {
        let store = store.lock().unwrap();
        if x402_nonce_already_consumed(&store, &proof.tx_sig).unwrap_or(false) {
            return PaymentGate::Unauthorized(format!(
                "x402 payment already used: {}",
                proof.tx_sig
            ));
        }
    }

    // Verify the Solana USDC transfer
    match verify_usdc_transfer(solana, &proof.tx_sig, treasury, usdc_mint, cost as u64).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return PaymentGate::Unauthorized(format!(
                "x402 payment not valid: tx {} does not transfer >= {cost} micro-USDC to treasury",
                proof.tx_sig
            ))
        }
        Err(e) => return PaymentGate::Unauthorized(format!("x402 verification error: {e}")),
    }

    // Do NOT mark the nonce here. The nonce is consumed only after a
    // successful delivery confirmation (or, in the
    // legacy `payment_mode == "none"` path, never). See
    // `consume_x402_nonce_after_success`.
    PaymentGate::Proceed(None)
}

/// Read-only replay check for an x402 nonce. Returns `Ok(true)` if a row
/// already exists in `x402_nonces`, `Ok(false)` otherwise.
///
/// Used by `check_x402` to fail-fast on replay BEFORE the more expensive
/// `verify_usdc_transfer` Solana RPC. Note: a race window exists between
/// this read and the eventual `consume_x402_nonce_after_success` INSERT
/// — the loser gets `mark_x402_nonce` ConstraintViolation, which is the
/// correct outcome (one of the two concurrent requests wins).
pub fn x402_nonce_already_consumed(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<bool> {
    let exists: bool = store
        .conn()
        .query_row(
            "SELECT 1 FROM x402_nonces WHERE tx_sig = ? LIMIT 1",
            params![tx_sig],
            |_| Ok(true),
        )
        .unwrap_or(false);
    Ok(exists)
}

/// Consume an x402 nonce by inserting it into `x402_nonces`. Called by
/// the caller AFTER the delivery confirmation passes (T3 round-2
/// deferral). Returns `Err` on ConstraintViolation if the same nonce was
/// concurrently consumed by another request.
///
/// Round-2 split: the original `check_x402` consumed the nonce at gate
/// time, which made delivery failures permanently spend the caller's
/// USDC. With the nonce deferred to here, a delivery failure leaves the
/// nonce reusable and the caller can retry with the same `X-Payment`
/// header — they pay Arweave/Solana fees again on the retry (operator
/// bleed), but the DoS quota guard caps how many such retries cost the
/// operator.
pub fn consume_x402_nonce_after_success(store: &SqliteStore, tx_sig: &str) -> anyhow::Result<()> {
    mark_x402_nonce(store, tx_sig)
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
            let mint = post_entry["mint"].as_str().unwrap_or("");
            if owner != recipient || mint != usdc_mint {
                continue;
            }
            let post_amount: u64 = post_entry["uiTokenAmount"]["amount"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);

            let account_index = post_entry["accountIndex"].as_u64().unwrap_or(u64::MAX);
            let pre_amount: u64 = pre_balances
                .iter()
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
    let key = format!("mnm_{}", hex::encode(random_bytes::<24>()?));
    let now = chrono::Utc::now().to_rfc3339();
    store.conn().execute(
        "INSERT INTO api_keys (api_key, owner_pubkey, balance_micro_usdc, created_at) VALUES (?,?,0,?)",
        params![key, owner_pubkey, now],
    )?;
    Ok(key)
}

/// Get the owner pubkey for an API key. Returns None if key not found.
pub fn get_owner_pubkey(store: &SqliteStore, api_key: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = store
        .conn()
        .prepare("SELECT owner_pubkey FROM api_keys WHERE api_key = ?")?;
    let mut rows = stmt.query(params![api_key])?;
    Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
}

/// Get balance in micro-USDC for an API key. Returns None if key not found.
pub fn get_balance(store: &SqliteStore, api_key: &str) -> anyhow::Result<Option<i64>> {
    let mut stmt = store
        .conn()
        .prepare("SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?")?;
    let mut rows = stmt.query(params![api_key])?;
    Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
}

/// Deduct `amount` from balance. Returns Err if insufficient funds or key not found.
///
/// The balance UPDATE and the `payment_events` charge INSERT are wrapped in
/// `BEGIN IMMEDIATE` / `COMMIT` so the balance decrement and audit-trail row
/// commit together or not at all. If the INSERT fails (e.g. disk full or an
/// `event_id` UUID collision), the transaction rolls back and the balance is
/// left untouched — matching the atomicity discipline of `credit_deposit` and
/// `refund_balance`.
///
/// The conditional UPDATE `WHERE api_key = ? AND balance_micro_usdc >= ?` is
/// the idempotency gate: `conn.changes() == 0` means either the key is
/// unknown or the balance is too low. In that case we ROLLBACK and use a
/// read-only `get_balance` call to produce the precise user-visible error.
/// Two concurrent deducts on the same key cannot both pass the guard because
/// SQLite serializes writes to the same table under the IMMEDIATE write lock.
pub fn deduct_balance(
    store: &SqliteStore,
    api_key: &str,
    amount: i64,
    description: &str,
) -> anyhow::Result<()> {
    let conn = store.conn();
    let now = chrono::Utc::now().to_rfc3339();

    // Acquire the write lock at transaction open so the UPDATE + INSERT
    // commit atomically.
    conn.execute("BEGIN IMMEDIATE", [])?;

    // Single-statement atomic check + decrement.
    let update_res = conn.execute(
        "UPDATE api_keys
             SET balance_micro_usdc = balance_micro_usdc - ?1,
                 last_used_at = ?2
           WHERE api_key = ?3
             AND balance_micro_usdc >= ?1",
        params![amount, now, api_key],
    );
    let changed = match update_res {
        Ok(n) => n,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
    };

    if changed == 0 {
        // Either the key does not exist or the balance is too low. We reuse
        // get_balance to produce the precise error message the client expects
        // (it's read-only, so there is no race here — worst case the message
        // is slightly stale, which is fine for a rejection path).
        let _ = conn.execute("ROLLBACK", []);
        match get_balance(store, api_key)? {
            None => anyhow::bail!("api key not found"),
            Some(balance) => {
                anyhow::bail!("insufficient balance: have {balance} micro-USDC, need {amount}")
            }
        }
    }

    let insert_res = conn.execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, description, created_at) VALUES (?,?,?,'charge',?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, description, now],
    );
    if let Err(e) = insert_res {
        // Audit-trail INSERT failed — roll back the balance decrement so the
        // ledger and balance stay consistent. Without this, a disk-full or
        // constraint error on the event row would leave the balance debited
        // with no charge record.
        let _ = conn.execute("ROLLBACK", []);
        return Err(e.into());
    }

    conn.execute("COMMIT", [])?;
    Ok(())
}

/// Credit a deposit. Returns new balance.
///
/// Idempotency is enforced at the SQL level via the UNIQUE index on
/// `payment_events.tx_sig` (see `core/src/storage/sqlite.rs`). Two concurrent
/// calls with the same `tx_sig` cannot both succeed: the second one's INSERT
/// fails with `ConstraintViolation`, which we convert to a user-visible error.
/// The INSERT + UPDATE are wrapped in an `IMMEDIATE` transaction so the
/// balance is only credited when the idempotency row is actually inserted.
pub fn credit_deposit(
    store: &SqliteStore,
    api_key: &str,
    amount: i64,
    tx_sig: &str,
) -> anyhow::Result<i64> {
    let conn = store.conn();
    let now = chrono::Utc::now().to_rfc3339();

    // BEGIN IMMEDIATE: acquire the write lock at transaction start so the
    // UNIQUE-constraint check races cleanly against concurrent writers.
    conn.execute("BEGIN IMMEDIATE", [])?;

    // The idempotency-gating INSERT. If tx_sig already exists, this returns
    // SqliteFailure(ConstraintViolation) — we translate that into a typed
    // bail so the caller can tell apart "duplicate" from "other DB error".
    let insert_res = conn.execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, tx_sig, description, created_at) VALUES (?,?,?,'deposit',?,?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, tx_sig, "USDC deposit", now],
    );

    if let Err(rusqlite::Error::SqliteFailure(e, _)) = &insert_res {
        if e.code == rusqlite::ErrorCode::ConstraintViolation {
            let _ = conn.execute("ROLLBACK", []);
            anyhow::bail!("deposit tx already applied: {tx_sig}");
        }
    }
    if let Err(e) = insert_res {
        let _ = conn.execute("ROLLBACK", []);
        return Err(e.into());
    }

    // Credit the balance.
    let update_res = conn.execute(
        "UPDATE api_keys SET balance_micro_usdc = balance_micro_usdc + ? WHERE api_key = ?",
        params![amount, api_key],
    );
    let changed = match update_res {
        Ok(n) => n,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
    };
    if changed == 0 {
        let _ = conn.execute("ROLLBACK", []);
        anyhow::bail!("api key not found: {api_key}");
    }

    let new_balance: i64 = match conn.query_row(
        "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?",
        params![api_key],
        |r| r.get(0),
    ) {
        Ok(b) => b,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
    };

    conn.execute("COMMIT", [])?;
    Ok(new_balance)
}

/// Refund `amount` to `api_key`'s balance after a failed tool call.
///
/// This is the reverse of `deduct_balance` and intentionally does NOT use
/// `credit_deposit`: deposits are gated by the UNIQUE(tx_sig) idempotency
/// index, but a tool can legitimately fail the same way multiple times for
/// the same key and each refund must apply. Refund rows are written with
/// `tx_sig = NULL` (exempt from the partial index) and `event_type='refund'`.
/// The read + update is wrapped in an IMMEDIATE transaction.
pub fn refund_balance(
    store: &SqliteStore,
    api_key: &str,
    amount: i64,
    reason: &str,
) -> anyhow::Result<i64> {
    let conn = store.conn();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute("BEGIN IMMEDIATE", [])?;

    let update_res = conn.execute(
        "UPDATE api_keys SET balance_micro_usdc = balance_micro_usdc + ? WHERE api_key = ?",
        params![amount, api_key],
    );
    let changed = match update_res {
        Ok(n) => n,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
    };
    if changed == 0 {
        let _ = conn.execute("ROLLBACK", []);
        anyhow::bail!("api key not found: {api_key}");
    }

    let description = format!("refund: {reason}");
    let insert_res = conn.execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, tx_sig, description, created_at) VALUES (?,?,?,'refund',NULL,?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, description, now],
    );
    if let Err(e) = insert_res {
        let _ = conn.execute("ROLLBACK", []);
        return Err(e.into());
    }

    let new_balance: i64 = match conn.query_row(
        "SELECT balance_micro_usdc FROM api_keys WHERE api_key = ?",
        params![api_key],
        |r| r.get(0),
    ) {
        Ok(b) => b,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
    };

    conn.execute("COMMIT", [])?;
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
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
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

/// Cryptographically secure random bytes from `/dev/urandom`.
///
/// Fails loudly if OS entropy is unavailable. API keys derive directly from
/// this output, so silently substituting a weak PRNG (time+PID+counter) on
/// entropy-source failure would let an attacker narrow the key search space.
/// Callers propagate the error; the caller of `create_api_key` surfaces it
/// as a 500 to the client, which is the correct behavior.
fn random_bytes<const N: usize>() -> anyhow::Result<[u8; N]> {
    use std::io::Read;
    let mut out = [0u8; N];
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| anyhow::anyhow!("entropy source /dev/urandom unavailable: {e}"))?;
    f.read_exact(&mut out)
        .map_err(|e| anyhow::anyhow!("reading from /dev/urandom failed: {e}"))?;
    Ok(out)
}

// ── Delivery DoS guard (modes-user-choice T3) ───────────────────────────────
//
// Outcome-based per-`api_key_hash` sliding-window counter consulted at the
// *entry* of the participate path in `mcp_handler` BEFORE any Arweave or
// Solana write. Increments on every delivery-not-confirmed demotion. When a
// caller crosses the threshold within the window the next `participate`
// request short-circuits with `-32011 DeliveryQuotaExceeded` so a
// systematically-failing client cannot bleed operator margin by triggering
// chain spend that is always refunded.
//
// Keying is on the bearer api_key's blake3 digest (`api_key_hash`), NOT on
// `owner_pubkey`. Ed25519 keys can be rotated for free; billable subjects
// can't. Keying on the wrong identifier would let an attacker bypass the
// quota by minting a fresh identity per request — same threat model as
// e-mail-based rate-limits not keyed on e-mail-aliases.
//
// `DashMap` shard-level lock discipline (extends Decision 8 of the
// tech-spec): every method below holds a shard guard for the duration of a
// single `record` / `count` / `is_empty_for` call and drops it before
// returning. No `.await` between guard acquisition and drop. The
// background eviction task respects the same rule per shard.

/// Compute the blake3 digest of an api_key, hex-encoded. Used both as the
/// quota-counter subject AND as the credential-at-rest identifier for the
/// `payment_events` audit row written by [`record_refund_failed`]. Centralised
/// here so call-sites cannot accidentally substitute the raw key (CWE-312
/// hygiene).
pub fn hash_api_key(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}

/// Sliding-window timestamp counter — push, prune-on-read, count. Not
/// thread-safe by itself; protection comes from the `DashMap` shard the
/// counter sits inside. Methods are `&mut self` so the type-system enforces
/// the shard-guard exclusivity at the call-site.
#[derive(Debug, Default, Clone)]
pub struct SlidingWindowCounter {
    timestamps: Vec<Instant>,
}

impl SlidingWindowCounter {
    /// Push `now` and drop timestamps older than `window`. Bounded by
    /// the threshold check that fronts every increment site so the vec
    /// never grows past `O(threshold)` in practice.
    pub fn record(&mut self, now: Instant, window: Duration) {
        let cutoff = now.checked_sub(window);
        if let Some(cutoff) = cutoff {
            self.timestamps.retain(|t| *t >= cutoff);
        }
        self.timestamps.push(now);
    }

    /// Count timestamps that fall inside `window` looking back from `now`.
    /// Does NOT mutate (callers might want to inspect without pruning).
    pub fn count(&self, now: Instant, window: Duration) -> u32 {
        let cutoff = match now.checked_sub(window) {
            Some(c) => c,
            None => return self.timestamps.len() as u32,
        };
        self.timestamps.iter().filter(|t| **t >= cutoff).count() as u32
    }

    /// `true` if the counter has had no timestamps in the last `since`
    /// duration — used by the eviction loop to decide whether a subject
    /// has gone dormant.
    pub fn is_empty_for(&self, now: Instant, since: Duration) -> bool {
        let Some(cutoff) = now.checked_sub(since) else {
            return false;
        };
        !self.timestamps.iter().any(|t| *t >= cutoff)
    }
}

/// Per-subject sliding-window demotion counter. See module-level comment.
///
/// Single `DashMap` instance shared across the whole process via
/// `McpState.refunds_by_subject`. Bounded by the background eviction task
/// spawned in `main.rs::run_http`.
pub struct RefundsBySubject {
    inner: Arc<DashMap<String, SlidingWindowCounter>>,
    window: Duration,
    threshold: u32,
}

impl RefundsBySubject {
    /// Build a fresh guard with the given window and threshold.
    pub fn new(window: Duration, threshold: u32) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            window,
            threshold,
        }
    }

    /// Configured window duration. Surfaced for the `DeliveryQuotaExceeded`
    /// error envelope so the client knows the SLO knob.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Configured demotion threshold. Surfaced for the typed error envelope.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// True iff the subject has hit or exceeded the threshold inside the
    /// sliding window. Acquires the shard guard for `subject`, takes the
    /// count, releases. No `.await` between acquire and drop (extends
    /// Decision 8 to DashMap).
    pub fn is_over(&self, subject: &str) -> bool {
        let now = Instant::now();
        match self.inner.get(subject) {
            Some(entry) => entry.count(now, self.window) >= self.threshold,
            None => false,
        }
    }

    /// Increment the subject's counter by one. Acquires the shard guard
    /// briefly and releases before returning. Safe to call from the
    /// failure-branch of `sign_memory_inline` after the SQLite mutex has
    /// already been released.
    pub fn record_failure(&self, subject: &str) {
        let now = Instant::now();
        let window = self.window;
        self.inner
            .entry(subject.to_string())
            .or_default()
            .record(now, window);
    }

    /// Bounded eviction: drop any entry whose counter has been empty for the
    /// last `since` duration. Holds each shard guard only for the duration
    /// of its own retain pass; never across `.await`. Returns the number of
    /// entries evicted (useful for instrumentation + tests).
    pub fn evict_idle(&self, since: Duration) -> usize {
        let before = self.inner.len();
        let now = Instant::now();
        // `retain` on DashMap iterates shard-by-shard, holding only one
        // shard guard at a time. Closure runs synchronously; no `.await`
        // crosses the guard boundary.
        self.inner
            .retain(|_, counter| !counter.is_empty_for(now, since));
        before.saturating_sub(self.inner.len())
    }

    /// Number of subjects currently tracked. Useful for tests.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if the map is currently empty. Convenience for tests.
    #[allow(dead_code)] // exercised by unit tests; future eviction-loop introspection.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Process-lifetime counters incremented in the delivery-guarantee flow.
/// Lightweight `AtomicU64` shims that stand in for the eventual Prometheus
/// histogram/counter surface (the rest of the binary hasn't wired
/// `metrics::` crates yet — see commit log; we can swap to `metrics::counter!`
/// later without touching call-sites because they go through
/// [`DeliveryMetrics`]).
///
/// The four counters are:
/// - `delivery_quota_short_circuit` — `-32011 DeliveryQuotaExceeded` returns.
/// - `delivery_not_confirmed_refetch` — demotions due to Arweave re-fetch.
/// - `delivery_not_confirmed_verify` — demotions due to verify_cose mismatch.
/// - `delivery_not_confirmed_recall` — demotions due to recall miss.
///
/// No per-tenant label (`api_key_hash` or `owner_pubkey`) is attached. That
/// would be a high-cardinality anti-pattern for any future Prometheus
/// adapter; per-tenant detail belongs in structured `tracing::warn!` lines
/// (already emitted at every demotion call-site).
pub struct DeliveryMetrics {
    quota_short_circuit: AtomicU64,
    not_confirmed_refetch: AtomicU64,
    not_confirmed_verify: AtomicU64,
    not_confirmed_recall: AtomicU64,
}

impl Default for DeliveryMetrics {
    fn default() -> Self {
        Self {
            quota_short_circuit: AtomicU64::new(0),
            not_confirmed_refetch: AtomicU64::new(0),
            not_confirmed_verify: AtomicU64::new(0),
            not_confirmed_recall: AtomicU64::new(0),
        }
    }
}

impl DeliveryMetrics {
    /// Increment the quota-exceeded short-circuit counter.
    pub fn record_quota_short_circuit(&self) {
        self.quota_short_circuit.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the per-stage `delivery_not_confirmed_total` counter.
    /// `stage` must be one of `"refetch"`, `"verify"`, `"recall"`.
    pub fn record_not_confirmed(&self, stage: &str) {
        match stage {
            "refetch" => self.not_confirmed_refetch.fetch_add(1, Ordering::Relaxed),
            "verify" => self.not_confirmed_verify.fetch_add(1, Ordering::Relaxed),
            "recall" => self.not_confirmed_recall.fetch_add(1, Ordering::Relaxed),
            // Unknown stage — log but do not crash. Reaching here means a
            // call-site sent a stage label the metric type doesn't know.
            other => {
                tracing::warn!(
                    stage = other,
                    "DeliveryMetrics::record_not_confirmed called with unknown stage"
                );
                0
            }
        };
    }

    /// Read the quota-short-circuit counter. For tests + future Prometheus
    /// adapter only.
    #[allow(dead_code)] // exercised by integration tests via test-support feature.
    pub fn quota_short_circuit(&self) -> u64 {
        self.quota_short_circuit.load(Ordering::Relaxed)
    }

    /// Read the per-stage `delivery_not_confirmed_total` counter.
    #[allow(dead_code)] // exercised by integration tests via test-support feature.
    pub fn not_confirmed(&self, stage: &str) -> u64 {
        match stage {
            "refetch" => self.not_confirmed_refetch.load(Ordering::Relaxed),
            "verify" => self.not_confirmed_verify.load(Ordering::Relaxed),
            "recall" => self.not_confirmed_recall.load(Ordering::Relaxed),
            _ => 0,
        }
    }
}

/// Record a `refund_failed` audit row when the refund-itself path fails after
/// a delivery demotion. Best-effort — callers should `.ok()` the result; the
/// row is a forensic crumb, not a correctness barrier.
///
/// **PII allow-list** (tech-spec §"Risk & mitigations / Refund audit-trail
/// correctness"): the row carries ONLY `{api_key_hash, attestation_id,
/// reason, occurred_at}`. NO raw `api_key`, NO `content_preview`, NO
/// embedding bytes, NO COSE payload. The `payment_events.api_key` column
/// receives the `blake3` digest here (column name is legacy; the schema is
/// untouched per the spec — no new migration needed).
///
/// Lives in `mcp/src/payment.rs` per the project's hard architectural rule:
/// all payment methods live in `payment.rs`, never in `core/`. Body uses
/// the existing schema columns; the new `event_type='refund_failed'` value
/// is just a new enumerant.
pub fn record_refund_failed(
    store: &SqliteStore,
    api_key_hash: &str,
    attestation_id: &str,
    reason: &str,
    occurred_at: &str,
) -> anyhow::Result<()> {
    // Description carries the demoted attestation_id + the failure reason in
    // a structured `{id} | {reason}` shape so an operator can grep
    // `payment_events.description LIKE '<attestation_id>%'` for forensics.
    let description = format!("{attestation_id} | {reason}");
    store.conn().execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, tx_sig, description, created_at) VALUES (?,?,?,'refund_failed',NULL,?,?)",
        params![
            uuid::Uuid::new_v4().to_string(),
            api_key_hash,
            0i64,
            description,
            occurred_at,
        ],
    )?;
    Ok(())
}

// ── EVM x402 (Wave 1 — non-custodial Arc/EVM settlement) ─────────────────────
//
// Mirror of the Solana `verify_usdc_transfer` for EVM chains (Arc, Base, …): a
// client signs an ERC-20 USDC `transfer(treasury, amount)` with its own derived
// key (no external wallet — see work/noncustodial-paradigm/design.md §19) and
// presents the tx hash in the `X-Payment` header. We confirm it on-chain via
// `eth_getTransactionReceipt` and decode the ERC-20 Transfer log.

/// EVM-side payment settlement config. `None` on `McpState` = EVM x402 disabled.
#[derive(Debug, Clone)]
pub struct EvmPaymentConfig {
    /// EVM JSON-RPC endpoint (e.g. Arc: https://rpc.testnet.arc.network).
    pub rpc_url: String,
    /// ERC-20 USDC token contract address (lowercased `0x…`).
    pub usdc_token: String,
    /// Treasury recipient address (lowercased `0x…`).
    pub treasury: String,
}

/// keccak256("Transfer(address,address,uint256)") — the ERC-20 Transfer topic0.
const ERC20_TRANSFER_TOPIC0: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Lowercase + strip `0x`; right-most 40 hex chars of a 32-byte topic = address.
fn topic_to_address(topic: &str) -> String {
    let t = topic.trim_start_matches("0x").to_lowercase();
    let start = t.len().saturating_sub(40);
    format!("0x{}", &t[start..])
}

/// Pure decoder: given a receipt's `logs` array, return the transferred amount
/// (as u128) iff some log is an ERC-20 `Transfer` from `usdc_token` to
/// `treasury` of at least `min_amount`. Network-free so it is unit-testable.
fn match_erc20_transfer(
    logs: &[serde_json::Value],
    usdc_token: &str,
    treasury: &str,
    min_amount: u128,
) -> Option<u128> {
    let token = usdc_token.to_lowercase();
    let to_want = treasury.to_lowercase();
    for log in logs {
        let addr = log["address"].as_str().unwrap_or("").to_lowercase();
        if addr != token {
            continue;
        }
        let topics = log["topics"].as_array()?;
        if topics.len() < 3 {
            continue;
        }
        if topics[0].as_str().unwrap_or("").to_lowercase() != ERC20_TRANSFER_TOPIC0 {
            continue;
        }
        if topic_to_address(topics[2].as_str().unwrap_or("")) != to_want {
            continue;
        }
        let data = log["data"]
            .as_str()
            .unwrap_or("0x")
            .trim_start_matches("0x");
        let amount = u128::from_str_radix(data, 16).unwrap_or(0);
        if amount >= min_amount {
            return Some(amount);
        }
    }
    None
}

/// Verify an EVM ERC-20 USDC transfer to the treasury via `eth_getTransactionReceipt`.
/// Returns `Ok(Some(amount))` when a matching Transfer of `>= min_amount` is found
/// in a successful (`status == 0x1`) receipt, else `Ok(None)`.
pub async fn verify_evm_usdc_transfer(
    rpc_url: &str,
    tx_hash: &str,
    treasury: &str,
    usdc_token: &str,
    min_amount: u128,
) -> anyhow::Result<Option<u128>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "eth_getTransactionReceipt", "params": [tx_hash],
    });
    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    let receipt = &resp["result"];
    if receipt.is_null() {
        return Ok(None); // unknown / unconfirmed tx
    }
    // Reject reverted transactions (status 0x0).
    if receipt["status"].as_str().unwrap_or("") != "0x1" {
        return Ok(None);
    }
    let logs = receipt["logs"].as_array().cloned().unwrap_or_default();
    Ok(match_erc20_transfer(
        &logs, usdc_token, treasury, min_amount,
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    //! Unit tests for the atomicity + idempotency properties of the payment
    //! DB helpers. Each test opens an in-memory SqliteStore so there is no
    //! filesystem dependency. For "concurrent" assertions we open a second
    //! connection to the same file-backed DB via tempfile::NamedTempFile and
    //! spawn threads — `SqliteStore::in_memory()` gives each caller its own
    //! empty DB, which is the wrong semantic for race tests.
    use super::*;
    use mnemonic_core::storage::SqliteStore;
    use std::sync::Arc;
    use std::thread;

    // ── EVM ERC-20 Transfer decoder (Wave 1) ────────────────────────────────
    fn transfer_log(token: &str, to_topic: &str, data_hex: &str) -> serde_json::Value {
        serde_json::json!({
            "address": token,
            "topics": [
                super::ERC20_TRANSFER_TOPIC0,
                "0x000000000000000000000000aaaa000000000000000000000000000000000001",
                to_topic
            ],
            "data": data_hex,
        })
    }

    #[test]
    fn evm_transfer_matches_recipient_and_amount() {
        let token = "0x3600000000000000000000000000000000000000";
        let treasury = "0x00000000000000000000000000000000000000fe";
        let to_topic = "0x00000000000000000000000000000000000000000000000000000000000000fe";
        // 1_000_000 (1 USDC, 6-dec) = 0xf4240
        let logs = vec![transfer_log(
            token,
            to_topic,
            "0x00000000000000000000000000000000000000000000000000000000000f4240",
        )];
        assert_eq!(
            match_erc20_transfer(&logs, token, treasury, 1_000_000),
            Some(1_000_000)
        );
        // below minimum → no match
        assert_eq!(
            match_erc20_transfer(&logs, token, treasury, 2_000_000),
            None
        );
    }

    #[test]
    fn evm_transfer_rejects_wrong_token_or_recipient() {
        let token = "0x3600000000000000000000000000000000000000";
        let treasury = "0x00000000000000000000000000000000000000fe";
        let to_topic = "0x00000000000000000000000000000000000000000000000000000000000000fe";
        let amt = "0x00000000000000000000000000000000000000000000000000000000000f4240";
        // wrong token contract
        let wrong_token = vec![transfer_log(
            "0xdeadbeef00000000000000000000000000000000",
            to_topic,
            amt,
        )];
        assert_eq!(match_erc20_transfer(&wrong_token, token, treasury, 1), None);
        // wrong recipient
        let other = "0x00000000000000000000000000000000000000000000000000000000000000ab";
        let wrong_to = vec![transfer_log(token, other, amt)];
        assert_eq!(match_erc20_transfer(&wrong_to, token, treasury, 1), None);
    }

    #[test]
    fn topic_to_address_takes_last_20_bytes() {
        assert_eq!(
            topic_to_address("0x00000000000000000000000000000000000000000000000000000000000000fe"),
            "0x00000000000000000000000000000000000000fe"
        );
    }

    fn fresh_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn fresh_file_store() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mcp-test.db");
        // Create the schema by opening + dropping.
        let _ = SqliteStore::open(&path).expect("open store");
        (dir, path)
    }

    #[test]
    fn refund_balance_allows_duplicate_reasons() {
        // Covers: review round1 major finding — refund via credit_deposit
        // silently dropped the second refund when tx_sig collided.
        let store = fresh_store();
        let key = create_api_key(&store, "owner_a").unwrap();
        // Pre-fund the key by crediting two distinct deposits.
        credit_deposit(&store, &key, 1_000, "sig_deposit_1").unwrap();

        // Two refunds with the same reason — both must apply.
        let after_1 = refund_balance(&store, &key, 100, "arweave upload failed").unwrap();
        let after_2 = refund_balance(&store, &key, 100, "arweave upload failed").unwrap();

        assert_eq!(after_1, 1_100, "first refund credits balance");
        assert_eq!(
            after_2, 1_200,
            "second identical refund ALSO credits balance"
        );

        // Both rows must exist in payment_events with event_type='refund'.
        let refund_count: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM payment_events WHERE api_key = ? AND event_type = 'refund'",
                params![key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(refund_count, 2);
    }

    #[test]
    fn credit_deposit_concurrent_same_tx_sig_applies_once() {
        // Covers: security-auditor major finding — TOCTOU on credit_deposit.
        // Two threads attempt to credit the SAME tx_sig at the same time.
        // Exactly one must succeed; balance must increase by exactly one
        // credit amount (not two).
        let (_dir, path) = fresh_file_store();

        // Pre-create the api_key using a short-lived connection.
        let key = {
            let s = SqliteStore::open(&path).unwrap();
            create_api_key(&s, "owner_concurrent").unwrap()
        };

        let tx_sig = "same_tx_abc_123".to_string();
        let amount = 5_000;
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let p1 = path.clone();
        let k1 = key.clone();
        let s1 = tx_sig.clone();
        let b1 = barrier.clone();
        let t1 = thread::spawn(move || -> Result<i64, String> {
            let s = SqliteStore::open(&p1).map_err(|e| e.to_string())?;
            b1.wait();
            credit_deposit(&s, &k1, amount, &s1).map_err(|e| e.to_string())
        });

        let p2 = path.clone();
        let k2 = key.clone();
        let s2 = tx_sig.clone();
        let b2 = barrier.clone();
        let t2 = thread::spawn(move || -> Result<i64, String> {
            let s = SqliteStore::open(&p2).map_err(|e| e.to_string())?;
            b2.wait();
            credit_deposit(&s, &k2, amount, &s2).map_err(|e| e.to_string())
        });

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();

        let successes = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
        let failures = [&r1, &r2].iter().filter(|r| r.is_err()).count();
        assert_eq!(
            successes, 1,
            "exactly one credit must succeed: {r1:?} {r2:?}"
        );
        assert_eq!(
            failures, 1,
            "the other must fail with duplicate: {r1:?} {r2:?}"
        );

        let err_msg = [&r1, &r2]
            .iter()
            .find(|r| r.is_err())
            .unwrap()
            .as_ref()
            .err()
            .unwrap();
        assert!(
            err_msg.contains("deposit tx already applied"),
            "duplicate error should surface: {err_msg}"
        );

        // Final balance: exactly one credit amount.
        let final_store = SqliteStore::open(&path).unwrap();
        assert_eq!(get_balance(&final_store, &key).unwrap(), Some(amount));

        // Exactly one deposit row in payment_events (UNIQUE index enforced).
        let deposit_rows: i64 = final_store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM payment_events WHERE tx_sig = ?",
                params![tx_sig],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(deposit_rows, 1);
    }

    #[test]
    fn deduct_balance_concurrent_cannot_overdraw() {
        // Covers: security-auditor major finding — TOCTOU on deduct_balance.
        // Seed balance = 100. Two threads each try to deduct 75. At most
        // ONE must succeed; the final balance must never be negative.
        let (_dir, path) = fresh_file_store();

        // Seed the key with exactly 100 micro-USDC.
        let key = {
            let s = SqliteStore::open(&path).unwrap();
            let k = create_api_key(&s, "owner_overdraft").unwrap();
            credit_deposit(&s, &k, 100, "seed_tx").unwrap();
            k
        };

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let p1 = path.clone();
        let k1 = key.clone();
        let b1 = barrier.clone();
        let t1 = thread::spawn(move || -> Result<(), String> {
            let s = SqliteStore::open(&p1).map_err(|e| e.to_string())?;
            b1.wait();
            deduct_balance(&s, &k1, 75, "t1").map_err(|e| e.to_string())
        });

        let p2 = path.clone();
        let k2 = key.clone();
        let b2 = barrier.clone();
        let t2 = thread::spawn(move || -> Result<(), String> {
            let s = SqliteStore::open(&p2).map_err(|e| e.to_string())?;
            b2.wait();
            deduct_balance(&s, &k2, 75, "t2").map_err(|e| e.to_string())
        });

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();

        let ok_count = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok_count, 1, "exactly one deduct succeeds: {r1:?} {r2:?}");

        let final_store = SqliteStore::open(&path).unwrap();
        let final_bal = get_balance(&final_store, &key).unwrap().unwrap();
        assert_eq!(final_bal, 25, "balance = 100 - 75 = 25, never negative");
        assert!(final_bal >= 0);
    }

    #[test]
    fn deduct_balance_insufficient_leaves_balance_unchanged() {
        let store = fresh_store();
        let key = create_api_key(&store, "owner_b").unwrap();
        credit_deposit(&store, &key, 50, "seed_tx_2").unwrap();

        let err = deduct_balance(&store, &key, 100, "too big").unwrap_err();
        assert!(
            err.to_string().contains("insufficient balance"),
            "err = {err}"
        );
        assert_eq!(get_balance(&store, &key).unwrap(), Some(50));
    }

    #[test]
    fn deduct_balance_unknown_key_reports_not_found() {
        let store = fresh_store();
        let err = deduct_balance(&store, "mnm_nonexistent", 10, "whoops").unwrap_err();
        assert!(err.to_string().contains("api key not found"), "err = {err}");
    }

    #[test]
    fn deduct_balance_audit_insert_failure_rolls_back_balance() {
        // Covers: review round2 major finding — balance decrement and audit-
        // trail INSERT must be atomic. We install a temporary BEFORE INSERT
        // trigger on payment_events that RAISEs ABORT when the charge row's
        // description matches a sentinel. The balance UPDATE in
        // deduct_balance runs first, so without transaction wrapping the
        // balance would be debited with no matching charge row. With the
        // round-3 fix, the failing INSERT triggers a ROLLBACK that undoes
        // the UPDATE.
        let store = fresh_store();
        let key = create_api_key(&store, "owner_rollback").unwrap();
        credit_deposit(&store, &key, 500, "seed_rollback_tx").unwrap();
        assert_eq!(get_balance(&store, &key).unwrap(), Some(500));

        // Trigger that fails only when the INSERT carries our sentinel
        // description. Other charge rows (if any) are unaffected.
        store
            .conn()
            .execute_batch(
                "CREATE TRIGGER force_fail_charge
                 BEFORE INSERT ON payment_events
                 WHEN NEW.event_type = 'charge' AND NEW.description = '__FORCE_FAIL__'
                 BEGIN SELECT RAISE(ABORT, 'forced audit insert failure'); END;",
            )
            .unwrap();

        let err = deduct_balance(&store, &key, 100, "__FORCE_FAIL__").unwrap_err();
        assert!(
            err.to_string().contains("forced audit insert failure")
                || err.to_string().to_lowercase().contains("abort"),
            "expected INSERT-failure error, got: {err}"
        );

        // Balance must be unchanged: 500 minus the FAILED deduction = 500.
        assert_eq!(
            get_balance(&store, &key).unwrap(),
            Some(500),
            "rollback must leave balance untouched"
        );

        // No charge row must have been written for the failed attempt.
        let charge_rows: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM payment_events WHERE api_key = ? AND event_type = 'charge'",
                params![key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            charge_rows, 0,
            "no charge row should exist when the INSERT aborted"
        );

        // Drop the trigger and verify a subsequent successful deduct works
        // normally, confirming the store is not left in a stuck state after
        // the rolled-back transaction.
        store
            .conn()
            .execute("DROP TRIGGER force_fail_charge", [])
            .unwrap();
        deduct_balance(&store, &key, 100, "normal").unwrap();
        assert_eq!(get_balance(&store, &key).unwrap(), Some(400));
    }

    #[test]
    fn credit_deposit_sequential_duplicate_is_rejected() {
        // Baseline idempotency test: sequential (same-thread) second call
        // with the same tx_sig must be rejected, mirroring the concurrent
        // test above but removing thread-scheduling variance.
        let store = fresh_store();
        let key = create_api_key(&store, "owner_c").unwrap();
        credit_deposit(&store, &key, 500, "unique_sig").unwrap();
        let err = credit_deposit(&store, &key, 500, "unique_sig").unwrap_err();
        assert!(
            err.to_string().contains("deposit tx already applied"),
            "err = {err}"
        );
        assert_eq!(get_balance(&store, &key).unwrap(), Some(500));
    }

    // ── T3: RefundsBySubject sliding-window guard ────────────────────────────

    #[test]
    fn hash_api_key_is_deterministic_and_not_raw_key() {
        let key = "mnm_abcdefghijklmnopqrstuvwx";
        let h1 = hash_api_key(key);
        let h2 = hash_api_key(key);
        assert_eq!(h1, h2, "hash must be deterministic");
        assert_ne!(h1, key, "hash must not equal raw key");
        assert!(
            !h1.contains("mnm_"),
            "blake3 hex must not contain the key prefix: {h1}"
        );
        assert_eq!(h1.len(), 64, "blake3 hex is 32 bytes = 64 hex chars");
    }

    #[test]
    fn refunds_by_subject_under_threshold_is_not_over() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 5);
        assert!(!g.is_over("sub_x"));
        g.record_failure("sub_x");
        g.record_failure("sub_x");
        assert!(!g.is_over("sub_x"));
    }

    #[test]
    fn refunds_by_subject_at_threshold_is_over() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 3);
        g.record_failure("sub_y");
        g.record_failure("sub_y");
        g.record_failure("sub_y");
        assert!(g.is_over("sub_y"));
        // Different subject is unaffected.
        assert!(!g.is_over("sub_z"));
    }

    #[test]
    fn refunds_by_subject_expired_entries_drop_out() {
        // 50ms window — easy to wait out in a test.
        let g = RefundsBySubject::new(Duration::from_millis(50), 2);
        g.record_failure("sub_a");
        g.record_failure("sub_a");
        assert!(g.is_over("sub_a"));
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            !g.is_over("sub_a"),
            "after window elapses count must drop below threshold"
        );
    }

    #[test]
    fn evict_idle_drops_silent_subjects() {
        let g = RefundsBySubject::new(Duration::from_millis(20), 5);
        g.record_failure("sub_p");
        assert_eq!(g.len(), 1);
        std::thread::sleep(Duration::from_millis(50));
        let dropped = g.evict_idle(Duration::from_millis(20));
        assert_eq!(dropped, 1);
        assert!(g.is_empty());
    }

    #[test]
    fn evict_idle_keeps_active_subjects() {
        let g = RefundsBySubject::new(Duration::from_secs(60), 5);
        g.record_failure("active");
        // Eviction `since` longer than the time elapsed → keep.
        let dropped = g.evict_idle(Duration::from_secs(60));
        assert_eq!(dropped, 0);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn record_refund_failed_writes_audit_row_with_hash_not_raw_key() {
        let store = fresh_store();
        let raw_key = "mnm_secretkeyabcdefghijklmn";
        let hash = hash_api_key(raw_key);
        let occurred_at = chrono::Utc::now().to_rfc3339();
        record_refund_failed(
            &store,
            &hash,
            "att-id-uuid-0001",
            "refund-itself-failed",
            &occurred_at,
        )
        .unwrap();

        // Row exists with the hash, not the raw key.
        let row: (String, String, i64, String) = store
            .conn()
            .query_row(
                "SELECT api_key, event_type, amount_micro_usdc, description
                   FROM payment_events
                  WHERE event_type = 'refund_failed'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, hash, "api_key column must carry hash, not raw key");
        assert_eq!(row.1, "refund_failed");
        assert_eq!(row.2, 0, "amount must be 0 (refund_failed is not money)");
        assert!(
            row.3.starts_with("att-id-uuid-0001 | "),
            "description must lead with the demoted attestation id: {}",
            row.3
        );
        assert!(
            row.3.contains("refund-itself-failed"),
            "description must include the reason: {}",
            row.3
        );

        // Negative assertion: no raw-key substring anywhere in the row.
        for col in [&row.0, &row.1, &row.3] {
            assert!(
                !col.contains(raw_key),
                "raw api_key must not appear anywhere in the audit row: {col}"
            );
        }
    }

    #[test]
    fn delivery_metrics_increment_and_read() {
        let m = DeliveryMetrics::default();
        assert_eq!(m.quota_short_circuit(), 0);
        m.record_quota_short_circuit();
        m.record_quota_short_circuit();
        assert_eq!(m.quota_short_circuit(), 2);

        m.record_not_confirmed("refetch");
        m.record_not_confirmed("refetch");
        m.record_not_confirmed("verify");
        assert_eq!(m.not_confirmed("refetch"), 2);
        assert_eq!(m.not_confirmed("verify"), 1);
        assert_eq!(m.not_confirmed("recall"), 0);
    }
}
