//! Dynamic pricing engine — Irys cost + SOL/USDC rate → attestation price.
//!
//! Runs a background refresh loop (configurable interval) that queries:
//!   • Irys price API  — upload cost for a typical payload (in lamports)
//!   • CoinGecko       — SOL/USDC spot price (no API key required)
//!
//! Computes:  price = max(min_price, (irys + tx_fee) × sol_usd × (1 + margin))
//! Stores result atomically so reads are always wait-free.

use std::sync::{
    atomic::{AtomicI64, AtomicU64, Ordering},
    Arc,
};
use anyhow::Context;

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
    client: reqwest::Client,
}

impl PricingEngine {
    pub fn new(initial_price: i64) -> Arc<Self> {
        Arc::new(Self {
            price_micro_usdc: AtomicI64::new(initial_price),
            sol_price_bits: AtomicU64::new(0f64.to_bits()),
            irys_lamports: AtomicI64::new(0),
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
    pub async fn refresh(&self, config: &PricingConfig) -> anyhow::Result<()> {
        let irys_lamports = fetch_irys_price(&self.client, config.typical_payload_bytes)
            .await
            .context("irys price fetch")?;

        let sol_price = fetch_sol_price(&self.client)
            .await
            .context("sol price fetch")?;

        let new_price = compute_price(
            irys_lamports,
            config.sol_tx_fee_lamports,
            sol_price,
            config.margin_bps,
            config.min_price_micro_usdc,
        );

        self.irys_lamports.store(irys_lamports as i64, Ordering::Relaxed);
        self.sol_price_bits.store(sol_price.to_bits(), Ordering::Relaxed);
        self.price_micro_usdc.store(new_price, Ordering::Relaxed);

        tracing::info!(
            irys_lamports,
            sol_price_usdc = sol_price,
            new_price_micro_usdc = new_price,
            "pricing refreshed"
        );
        Ok(())
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
