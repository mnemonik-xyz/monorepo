//! Dynamic pricing engine — Irys cost + SOL/USDC rate → attestation price.
//!
//! Runs a background refresh loop (configurable interval) that queries:
//!   • Irys price API  — upload cost for a typical payload (in lamports)
//!   • CoinGecko       — SOL/USDC spot price (no API key required)
//!
//! Computes:  price = max(min_price, (irys + tx_fee) × sol_usd × (1 + margin))
//! Stores result atomically so reads are always wait-free.

use anyhow::Context;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    Arc,
};

// ── Status ────────────────────────────────────────────────────────────────────

/// Where the quoted `participate` price comes from. Surfaced in the
/// `mnemonic_whoami` envelope as `participate_cost.pricing_status` so a
/// client can tell "free" apart from "the price feed is down" (issue #165).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PricingStatus {
    /// The most recent refresh succeeded: the price comes from live Irys +
    /// SOL/USDC quotes.
    Live,
    /// No refresh has succeeded yet, or the most recent refresh failed. The
    /// price is the configured floor or the last good quote.
    Fallback,
    /// The operator does not charge (`PAYMENT_MODE=none`). Set by the
    /// envelope, never by the engine.
    Disabled,
}

// ── Config ────────────────────────────────────────────────────────────────────

pub struct PricingConfig {
    /// Profit margin above break-even, in basis points (2000 = 20 %).
    pub margin_bps: u64,
    /// Hard floor: never quote below this (micro-USDC).
    pub min_price_micro_usdc: i64,
    /// Byte count used when quoting Irys upload price (should approximate a
    /// typical mnemonic_sign_memory payload).
    pub typical_payload_bytes: usize,
    /// Solana memo transaction fee (lamports). Usually ~5 000.
    pub sol_tx_fee_lamports: u64,
}

// ── Cost hint passed to each sign_memory call ─────────────────────────────────

/// Snapshot of pricing data at the moment a sign_memory call is dispatched.
/// Stored in the DB for P&L accounting.
pub struct CostHint {
    /// Estimated Irys upload cost in lamports (from last pricing refresh).
    pub irys_lamports: u64,
    /// Solana memo tx fee in lamports (from config).
    pub sol_tx_fee_lamports: u64,
    /// SOL/USDC rate used for this estimate.
    pub sol_price_usdc: f64,
    /// What we charged the caller (micro-USDC).
    pub charge_micro_usdc: i64,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct PricingEngine {
    /// Current quoted price in micro-USDC.
    price_micro_usdc: AtomicI64,
    /// SOL/USDC spot price stored as f64 bits in an AtomicU64.
    sol_price_bits: AtomicU64,
    /// Irys upload estimate (lamports) for `typical_payload_bytes`.
    irys_lamports: AtomicI64,
    /// True only while the most recent refresh succeeded. Starts false: the
    /// seed price is the configured floor, not a live quote.
    last_refresh_ok: AtomicBool,
    client: reqwest::Client,
}

impl PricingEngine {
    pub fn new(initial_price: i64) -> Arc<Self> {
        Arc::new(Self {
            price_micro_usdc: AtomicI64::new(initial_price),
            sol_price_bits: AtomicU64::new(0f64.to_bits()),
            irys_lamports: AtomicI64::new(0),
            last_refresh_ok: AtomicBool::new(false),
            client: reqwest::Client::new(),
        })
    }

    /// Current quoted price for mnemonic_sign_memory (micro-USDC).
    pub fn current_price(&self) -> i64 {
        self.price_micro_usdc.load(Ordering::Relaxed)
    }

    /// Most recently fetched SOL/USDC rate.
    pub fn current_sol_price(&self) -> f64 {
        f64::from_bits(self.sol_price_bits.load(Ordering::Relaxed))
    }

    /// Most recently fetched Irys quote (lamports).
    pub fn current_irys_lamports(&self) -> i64 {
        self.irys_lamports.load(Ordering::Relaxed)
    }

    /// `Live` while the most recent refresh succeeded, else `Fallback`.
    pub fn status(&self) -> PricingStatus {
        if self.last_refresh_ok.load(Ordering::Relaxed) {
            PricingStatus::Live
        } else {
            PricingStatus::Fallback
        }
    }

    /// Record a failed refresh. The quoted price keeps its previous value
    /// (floor or last good quote); only the status flips to `Fallback`.
    pub fn record_refresh_failure(&self) {
        self.last_refresh_ok.store(false, Ordering::Relaxed);
    }

    /// Build a cost hint snapshot for recording alongside an attestation.
    pub fn cost_hint(&self, sol_tx_fee_lamports: u64) -> CostHint {
        CostHint {
            irys_lamports: self.current_irys_lamports() as u64,
            sol_tx_fee_lamports,
            sol_price_usdc: self.current_sol_price(),
            charge_micro_usdc: self.current_price(),
        }
    }

    /// Fetch fresh prices, recompute quoted price, store atomically.
    ///
    /// On any failure the previous price stays in place and `status()`
    /// reports `Fallback` until the next successful refresh.
    pub async fn refresh(&self, config: &PricingConfig) -> anyhow::Result<()> {
        let result = self.fetch_and_apply(config).await;
        if result.is_err() {
            self.record_refresh_failure();
        }
        result
    }

    async fn fetch_and_apply(&self, config: &PricingConfig) -> anyhow::Result<()> {
        let irys_lamports = fetch_irys_price(&self.client, config.typical_payload_bytes)
            .await
            .context("irys price fetch")?;

        let sol_price = fetch_sol_price(&self.client)
            .await
            .context("sol price fetch")?;

        let new_price = self.apply_quote(irys_lamports, sol_price, config)?;

        tracing::info!(
            irys_lamports,
            sol_price_usdc = sol_price,
            new_price_micro_usdc = new_price,
            "pricing refreshed"
        );
        Ok(())
    }

    /// Recompute the quoted price from fetched quotes, store it, and mark the
    /// engine `Live`. Returns the new price (micro-USDC).
    ///
    /// A SOL/USDC rate that is not a finite positive number is a failed
    /// fetch, not a free price: it is rejected and nothing is stored.
    pub fn apply_quote(
        &self,
        irys_lamports: u64,
        sol_price: f64,
        config: &PricingConfig,
    ) -> anyhow::Result<i64> {
        if !sol_price.is_finite() || sol_price <= 0.0 {
            anyhow::bail!("invalid SOL/USDC rate: {sol_price}");
        }
        let new_price = compute_price(
            irys_lamports,
            config.sol_tx_fee_lamports,
            sol_price,
            config.margin_bps,
            config.min_price_micro_usdc,
        );

        self.irys_lamports
            .store(irys_lamports as i64, Ordering::Relaxed);
        self.sol_price_bits
            .store(sol_price.to_bits(), Ordering::Relaxed);
        self.price_micro_usdc.store(new_price, Ordering::Relaxed);
        self.last_refresh_ok.store(true, Ordering::Relaxed);
        Ok(new_price)
    }
}

// ── Price computation ─────────────────────────────────────────────────────────

/// Break-even + margin, floored at `min_price`.
///
/// cost_micro_usdc = (irys_lam + tx_lam) × sol_price_usdc / 1_000  (lamports → micro-USDC)
/// quoted          = ceil(cost × (1 + margin_bps / 10_000))
pub fn compute_price(
    irys_lamports: u64,
    sol_tx_lamports: u64,
    sol_price_usdc: f64,
    margin_bps: u64,
    min_price: i64,
) -> i64 {
    let total_lamports = irys_lamports + sol_tx_lamports;
    // 1 lamport = 1e-9 SOL, 1 USDC = 1e6 micro-USDC
    // micro_usdc = lamports × sol_price / 1e9 × 1e6 = lamports × sol_price / 1_000
    let cost_micro_usdc = (total_lamports as f64) * sol_price_usdc / 1_000.0;
    let margin_factor = 1.0 + (margin_bps as f64) / 10_000.0;
    let quoted = (cost_micro_usdc * margin_factor).ceil() as i64;
    quoted.max(min_price)
}

// ── External price fetchers ───────────────────────────────────────────────────

/// `GET https://uploader.irys.xyz/price/solana/<bytes>` → lamports (u64).
async fn fetch_irys_price(client: &reqwest::Client, bytes: usize) -> anyhow::Result<u64> {
    let url = format!("https://uploader.irys.xyz/price/solana/{bytes}");
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let body = resp.text().await?;
    // Response is a plain integer (e.g. "4200") or JSON number
    let lamports: u64 = body
        .trim()
        .trim_matches('"')
        .parse()
        .with_context(|| format!("irys price response not a number: {body}"))?;
    Ok(lamports)
}

/// CoinGecko free API — SOL/USD spot price, no auth required.
/// `GET https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd`
async fn fetch_sol_price(client: &reqwest::Client) -> anyhow::Result<f64> {
    let resp = client
        .get("https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    json["solana"]["usd"]
        .as_f64()
        .context("CoinGecko: missing solana.usd field")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PricingConfig {
        PricingConfig {
            margin_bps: 2000,
            min_price_micro_usdc: 1000,
            typical_payload_bytes: 2048,
            sol_tx_fee_lamports: 5000,
        }
    }

    #[test]
    fn new_engine_reports_fallback_at_floor() {
        let engine = PricingEngine::new(1000);
        assert_eq!(engine.current_price(), 1000);
        assert_eq!(engine.status(), PricingStatus::Fallback);
    }

    #[test]
    fn apply_quote_marks_live_and_failure_keeps_last_price() {
        let engine = PricingEngine::new(1000);
        // (1_000_000 + 5_000) lamports at $150/SOL = 150_750 µUSDC, +20 % margin.
        let price = engine.apply_quote(1_000_000, 150.0, &cfg()).unwrap();
        assert_eq!(price, 180_900);
        assert_eq!(engine.current_price(), 180_900);
        assert_eq!(engine.status(), PricingStatus::Live);

        engine.record_refresh_failure();
        assert_eq!(engine.status(), PricingStatus::Fallback);
        assert_eq!(engine.current_price(), 180_900, "last good quote is kept");
    }

    #[test]
    fn apply_quote_rejects_zero_or_non_finite_sol_rate() {
        let engine = PricingEngine::new(1000);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(engine.apply_quote(1_000_000, bad, &cfg()).is_err());
        }
        assert_eq!(engine.status(), PricingStatus::Fallback);
        assert_eq!(engine.current_price(), 1000);
        assert_eq!(engine.current_sol_price(), 0.0);
    }

    #[test]
    fn pricing_status_serializes_lowercase() {
        let v = serde_json::to_value([
            PricingStatus::Live,
            PricingStatus::Fallback,
            PricingStatus::Disabled,
        ])
        .unwrap();
        assert_eq!(v, serde_json::json!(["live", "fallback", "disabled"]));
    }
}
