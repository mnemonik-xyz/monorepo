use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    pub transport: String,
    pub http_host: String,
    pub http_port: u16,
    pub solana_rpc_url: String,
    pub arweave_url: String,
    pub keypair_path: PathBuf,
    pub database_path: PathBuf,
    /// "hash" (default, offline) or "openai" (requires OPENAI_API_KEY)
    pub embed_provider: String,
    pub openai_api_key: String,
    pub openai_embed_model: String,
    /// TurboQuant bit width for compression (2, 3, or 4)
    pub turbo_bits: usize,

    // ── Storage mode ─────────────────────────────────────────────────────────
    /// "full" (default): Arweave + Solana + SQLite
    /// "local": SQLite only — no blockchain writes, free, instant, offline.
    ///          Perfect for testing the MCP flow without paying for on-chain ops.
    pub storage_mode: String,

    // ── Payment ──────────────────────────────────────────────────────────────
    /// Payment mode: "none" | "balance" | "x402" | "both"
    pub payment_mode: String,
    /// Solana pubkey that receives USDC payments
    pub treasury_pubkey: String,
    /// USDC SPL mint address (mainnet: EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)
    pub usdc_mint: String,
    /// Minimum / initial cost of mnemonic_sign_memory in micro-USDC (floor price)
    pub sign_memory_cost_micro_usdc: i64,

    // ── Dynamic pricing ───────────────────────────────────────────────────────
    /// How often to refresh Irys + SOL prices (seconds). Default 1800 (30 min).
    pub price_refresh_secs: u64,
    /// Profit margin above break-even in basis points (2000 = 20 %).
    pub pricing_margin_bps: u64,
    /// Typical mnemonic_sign_memory payload size used for Irys price quotes (bytes).
    pub typical_payload_bytes: usize,
    /// Solana memo tx fee in lamports (~5 000 on mainnet).
    pub sol_tx_fee_lamports: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let home = dirs_home();
        Self {
            transport: env_or("MCP_TRANSPORT", "http"),
            http_host: env_or("MCP_HTTP_HOST", "0.0.0.0"),
            http_port: env_or("MCP_HTTP_PORT", "3000").parse().unwrap_or(3000),
            solana_rpc_url: env_or("SOLANA_RPC_URL", "http://localhost:8899"),
            arweave_url: env_or("ARWEAVE_URL", "http://localhost:1984"),
            keypair_path: expand_path(&env_or(
                "MNEMONIC_KEYPAIR_PATH",
                &format!("{}/.mnemonic/id.json", home),
            )),
            database_path: expand_path(&env_or(
                "DATABASE_PATH",
                &format!("{}/.mnemonic/attestations.db", home),
            )),
            embed_provider: env_or("EMBED_PROVIDER", "fastembed"),
            openai_api_key: env_or("OPENAI_API_KEY", ""),
            openai_embed_model: env_or("OPENAI_EMBED_MODEL", "text-embedding-3-small"),
            turbo_bits: env_or("TURBO_BITS", "4").parse().unwrap_or(4),
            storage_mode: env_or("STORAGE_MODE", "local"),
            payment_mode: env_or("PAYMENT_MODE", "none"),
            treasury_pubkey: env_or("TREASURY_PUBKEY", ""),
            usdc_mint: env_or("USDC_MINT", "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            sign_memory_cost_micro_usdc: env_or("SIGN_MEMORY_COST_MICRO_USDC", "1000")
                .parse().unwrap_or(1000),
            price_refresh_secs: env_or("PRICE_REFRESH_SECS", "1800").parse().unwrap_or(1800),
            pricing_margin_bps: env_or("PRICING_MARGIN_BPS", "2000").parse().unwrap_or(2000),
            typical_payload_bytes: env_or("TYPICAL_PAYLOAD_BYTES", "2048").parse().unwrap_or(2048),
            sol_tx_fee_lamports: env_or("SOL_TX_FEE_LAMPORTS", "5000").parse().unwrap_or(5000),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn expand_path(p: &str) -> PathBuf {
    if p.starts_with('~') {
        PathBuf::from(p.replacen('~', &dirs_home(), 1))
    } else {
        PathBuf::from(p)
    }
}

fn dirs_home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
}
