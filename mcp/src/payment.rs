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
///
/// The read + decrement is atomic: we issue a single conditional `UPDATE ...
/// WHERE api_key = ? AND balance_micro_usdc >= ?`, then look at
/// `conn.changes()` to distinguish "no such key" from "insufficient funds".
/// Two concurrent calls on the same key can never both pass the guard
/// because SQLite serializes writes to the same table.
pub fn deduct_balance(store: &SqliteStore, api_key: &str, amount: i64, description: &str) -> anyhow::Result<()> {
    let conn = store.conn();
    let now = chrono::Utc::now().to_rfc3339();

    // Single-statement atomic check + decrement.
    let changed = conn.execute(
        "UPDATE api_keys
             SET balance_micro_usdc = balance_micro_usdc - ?1,
                 last_used_at = ?2
           WHERE api_key = ?3
             AND balance_micro_usdc >= ?1",
        params![amount, now, api_key],
    )?;

    if changed == 0 {
        // Either the key does not exist or the balance is too low. We reuse
        // get_balance to produce the precise error message the client expects
        // (it's read-only, so there is no race here — worst case the message
        // is slightly stale, which is fine for a rejection path).
        match get_balance(store, api_key)? {
            None => anyhow::bail!("api key not found"),
            Some(balance) => anyhow::bail!(
                "insufficient balance: have {balance} micro-USDC, need {amount}"
            ),
        }
    }

    conn.execute(
        "INSERT INTO payment_events (event_id, api_key, amount_micro_usdc, event_type, description, created_at) VALUES (?,?,?,'charge',?,?)",
        params![uuid::Uuid::new_v4().to_string(), api_key, amount, description, now],
    )?;
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
pub fn credit_deposit(store: &SqliteStore, api_key: &str, amount: i64, tx_sig: &str) -> anyhow::Result<i64> {
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
        params![api_key], |r| r.get(0),
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
        params![api_key], |r| r.get(0),
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
        assert_eq!(after_2, 1_200, "second identical refund ALSO credits balance");

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
        assert_eq!(successes, 1, "exactly one credit must succeed: {r1:?} {r2:?}");
        assert_eq!(failures, 1, "the other must fail with duplicate: {r1:?} {r2:?}");

        let err_msg = [&r1, &r2].iter().find(|r| r.is_err()).unwrap().as_ref().err().unwrap();
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
        assert!(err.to_string().contains("insufficient balance"), "err = {err}");
        assert_eq!(get_balance(&store, &key).unwrap(), Some(50));
    }

    #[test]
    fn deduct_balance_unknown_key_reports_not_found() {
        let store = fresh_store();
        let err = deduct_balance(&store, "mnm_nonexistent", 10, "whoops").unwrap_err();
        assert!(err.to_string().contains("api key not found"), "err = {err}");
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
        assert!(err.to_string().contains("deposit tx already applied"), "err = {err}");
        assert_eq!(get_balance(&store, &key).unwrap(), Some(500));
    }
}
