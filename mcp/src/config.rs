use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    pub transport: String,
    /// Populated from MCP_HTTP_HOST. The CLI `--host` flag overrides this at
    /// runtime; kept in `Config` for symmetry with the HTTP port and so that
    /// external consumers (tests, future handlers) can read the env value
    /// without re-parsing. See `main.rs::run_http` for the live binding.
    #[allow(dead_code)]
    pub http_host: String,
    /// See `http_host` — mirrors MCP_HTTP_PORT for env-level inspection.
    #[allow(dead_code)]
    pub http_port: u16,
    pub solana_rpc_url: String,
    pub arweave_url: String,
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

    // ── Ollama / RAG ─────────────────────────────────────────────────────────
    /// Ollama API base URL. Must match http://localhost:* or http://ollama:*
    /// (SSRF prevention, Decision 8).
    pub ollama_url: String,
    /// Ollama model name for chat inference.
    pub ollama_model: String,
    /// Directory where RAG artifacts (chunked knowledge .zip) are written.
    pub rag_chunk_dir: PathBuf,

    // ── LLM provider (universal chat inference) ─────────────────────────────
    /// Provider name: ollama, groq, openrouter, together, cerebras, openai, anthropic
    pub llm_provider: String,
    /// API key for the LLM provider (not needed for ollama).
    pub llm_api_key: String,
    /// Model override. If empty, uses the provider's default model.
    pub llm_model: String,
    /// API URL override. If empty, uses the provider's default base URL.
    pub llm_api_url: String,
    /// Maximum tokens for LLM responses.
    pub llm_max_tokens: u32,

    // ── Google OAuth (chrome-extension T14, Decision 5) ─────────────────────
    // `main.rs::run_http` reads these fields directly when constructing
    // `GoogleOAuthState`. They are the single source of truth for the
    // Google-OAuth env wiring — no `std::env::var` re-reads downstream
    // (round-1 code-reviewer finding #2).
    /// Google OAuth public client id. When empty, the Google OAuth router is
    /// not wired in `main.rs` and the corresponding endpoints return 404.
    pub google_oauth_client_id: String,
    /// Google OAuth client secret. Server-side only — never sent to the
    /// extension. Used for the `https://oauth2.googleapis.com/token` exchange.
    pub google_oauth_client_secret: String,
    /// Google OAuth redirect URI configured in Google Cloud Console. Must be
    /// HTTPS in production; defaults to `https://mcp.mnemonik.xyz/oauth/google/callback`.
    pub google_oauth_redirect_uri: String,

    // ── Extension key escrow (chrome-extension T15, Decision 9) ─────────────
    /// Max GET fetches against `/api/key-escrow` per rolling 24h per
    /// `google_sub`. Bounds online brute-force on the encrypted blob; the
    /// Argon2id KDF bounds the offline brute-force on stolen ciphertext.
    /// `main.rs::run_http` reads this value when constructing the escrow
    /// router state — there is no `std::env::var` re-read downstream
    /// (round-1 code-reviewer finding #2). Default 5.
    pub key_escrow_rate_limit: u32,

    // ── Delivery guarantee (modes-user-choice T3) ───────────────────────────
    //
    // Wall-clock budget + outcome-based DoS guard for the participate
    // delivery confirmation (Arweave re-fetch → verify_cose → in-process
    // recall). All four knobs are operator-tunable env vars so the
    // production sweet-spot can be found empirically without a redeploy of
    // behaviour. See `work/modes-user-choice/tech-spec.md` §"Risk &
    // mitigations / DoS amplification" for the rationale on each default.
    /// Wall-clock budget (seconds) for the post-anchor Arweave re-fetch.
    /// Retries with exponential backoff (200ms start, 2.0 factor, 2s cap)
    /// until either the bytes return OR this budget elapses. Sized against
    /// Arweave's documented eventual-consistency window. Default 15s.
    /// Env: `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS`.
    pub delivery_refetch_timeout_secs: u64,
    /// Quota threshold for the outcome-based DoS guard: when an
    /// `api_key_hash` accumulates this many delivery-failure demotions
    /// within `delivery_quota_window_secs`, subsequent `participate`
    /// requests from the same subject short-circuit with `-32011
    /// DeliveryQuotaExceeded` BEFORE any Arweave/Solana write. Keyed on
    /// `api_key_hash`, not `owner_pubkey` — Ed25519 keys rotate for free
    /// but billable subjects don't. Default 5.
    /// Env: `MNEMONIC_DELIVERY_QUOTA_THRESHOLD`.
    pub delivery_quota_threshold: u32,
    /// Sliding-window length (seconds) for the demotion counter above.
    /// Default 60s.
    /// Env: `MNEMONIC_DELIVERY_QUOTA_WINDOW_SECS`.
    pub delivery_quota_window_secs: u64,
    /// Eviction-loop interval (seconds) for the bounded-size guard.
    /// Entries with no timestamps in the last `2 * window_secs` are dropped
    /// so the map size tracks *active* spenders, not lifetime cardinality.
    /// Default 30s.
    /// Env: `MNEMONIC_DELIVERY_QUOTA_EVICT_SECS`.
    pub delivery_quota_evict_interval_secs: u64,
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
                .parse()
                .unwrap_or(1000),
            price_refresh_secs: env_or("PRICE_REFRESH_SECS", "1800").parse().unwrap_or(1800),
            pricing_margin_bps: env_or("PRICING_MARGIN_BPS", "2000").parse().unwrap_or(2000),
            typical_payload_bytes: env_or("TYPICAL_PAYLOAD_BYTES", "2048")
                .parse()
                .unwrap_or(2048),
            sol_tx_fee_lamports: env_or("SOL_TX_FEE_LAMPORTS", "5000")
                .parse()
                .unwrap_or(5000),
            ollama_url: env_or("OLLAMA_URL", "http://localhost:11434"),
            ollama_model: env_or("OLLAMA_MODEL", "qwen2.5:3b"),
            rag_chunk_dir: expand_path(&env_or("RAG_CHUNK_DIR", "./rag_chunks")),
            llm_provider: env_or("LLM_PROVIDER", "ollama"),
            llm_api_key: env_or("LLM_API_KEY", ""),
            llm_model: env_or("LLM_MODEL", ""),
            llm_api_url: env_or("LLM_API_URL", ""),
            llm_max_tokens: env_or("LLM_MAX_TOKENS", "512").parse().unwrap_or(512),
            google_oauth_client_id: env_or("GOOGLE_OAUTH_CLIENT_ID", ""),
            google_oauth_client_secret: env_or("GOOGLE_OAUTH_CLIENT_SECRET", ""),
            google_oauth_redirect_uri: env_or(
                "GOOGLE_OAUTH_REDIRECT_URI",
                "https://mcp.mnemonik.xyz/oauth/google/callback",
            ),
            key_escrow_rate_limit: env_or("KEY_ESCROW_RATE_LIMIT", "5").parse().unwrap_or(5),
            delivery_refetch_timeout_secs: env_or("MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS", "15")
                .parse()
                .unwrap_or(15),
            delivery_quota_threshold: env_or("MNEMONIC_DELIVERY_QUOTA_THRESHOLD", "5")
                .parse()
                .unwrap_or(5),
            delivery_quota_window_secs: env_or("MNEMONIC_DELIVERY_QUOTA_WINDOW_SECS", "60")
                .parse()
                .unwrap_or(60),
            delivery_quota_evict_interval_secs: env_or("MNEMONIC_DELIVERY_QUOTA_EVICT_SECS", "30")
                .parse()
                .unwrap_or(30),
        }
    }

    /// Validate `ollama_url` against the allowed whitelist.
    ///
    /// Only `http://localhost:<port>` and `http://ollama:<port>` are accepted.
    /// Any other URL is an SSRF risk (Decision 8). Returns `Err` with a
    /// human-readable message on failure -- caller should treat this as fatal.
    pub fn validate_ollama_url(&self) -> Result<(), String> {
        validate_ollama_url(&self.ollama_url)
    }
}

/// Validate that `url` matches the OLLAMA_URL whitelist.
///
/// Allowed patterns: `http://localhost:<port>[/...]` or `http://ollama:<port>[/...]`.
/// Rejects HTTPS (Ollama is always plain HTTP on a local/Docker network),
/// other hostnames, and malformed URLs.
fn validate_ollama_url(url: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(url).map_err(|e| format!("OLLAMA_URL is not a valid URL ({url}): {e}"))?;

    if parsed.scheme() != "http" {
        return Err(format!("OLLAMA_URL must use http:// scheme, got: {url}"));
    }

    match parsed.host_str() {
        Some("localhost") | Some("ollama") => Ok(()),
        Some(host) => Err(format!(
            "OLLAMA_URL host must be 'localhost' or 'ollama', got: {host}"
        )),
        None => Err(format!("OLLAMA_URL has no host: {url}")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_localhost_with_port() {
        assert!(validate_ollama_url("http://localhost:11434").is_ok());
    }

    #[test]
    fn accepts_localhost_with_path() {
        assert!(validate_ollama_url("http://localhost:11434/api/generate").is_ok());
    }

    #[test]
    fn accepts_ollama_host() {
        assert!(validate_ollama_url("http://ollama:11434").is_ok());
    }

    #[test]
    fn rejects_https() {
        let result = validate_ollama_url("https://localhost:11434");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("http://"));
    }

    #[test]
    fn rejects_external_host() {
        let result = validate_ollama_url("http://evil.com:11434");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("evil.com"));
    }

    #[test]
    fn rejects_ip_address() {
        let result = validate_ollama_url("http://10.0.0.1:11434");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_malformed_url() {
        let result = validate_ollama_url("not-a-url");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_string() {
        let result = validate_ollama_url("");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_loopback_ip() {
        // 127.0.0.1 is semantically localhost but must use the hostname form
        let result = validate_ollama_url("http://127.0.0.1:11434");
        assert!(result.is_err());
    }
}
