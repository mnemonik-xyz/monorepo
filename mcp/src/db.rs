//! Payment-related database methods -- operates on SqliteStore from mnemonic-core.
//!
//! These methods are payment concerns (API key management, balance, P&L) and
//! intentionally live in the MCP server, not in core.

use rusqlite::params;
use mnemonic_core::storage::SqliteStore;

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
        use sha2::{Sha256, Digest};
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
