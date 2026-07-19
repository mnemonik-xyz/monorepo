use std::path::PathBuf;
use std::str::FromStr;

use mnemonic_core::arweave::IrysNetwork;

/// Selects where a full-storage MCP writes its Irys bundle items and Solana
/// memo anchors. `devnet` is intentionally strict: it is the non-billable
/// staging mode and must never accept a production endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchoringNetwork {
    Mainnet,
    Devnet,
}

impl AnchoringNetwork {
    fn as_irys_network(self) -> IrysNetwork {
        match self {
            Self::Mainnet => IrysNetwork::Mainnet,
            Self::Devnet => IrysNetwork::Devnet,
        }
    }
}

impl FromStr for AnchoringNetwork {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mainnet" => Ok(Self::Mainnet),
            "devnet" => Ok(Self::Devnet),
            _ => Err(format!(
                "ANCHORING_NETWORK must be 'mainnet' or 'devnet', got: {value}"
            )),
        }
    }
}

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
    /// Irys read gateway. `IRYS_GATEWAY_URL` is the preferred name;
    /// `ARWEAVE_URL` remains a backwards-compatible fallback.
    pub arweave_url: String,
    /// Raw environment value, parsed and validated at startup so invalid
    /// values fail closed rather than silently falling back to mainnet.
    pub anchoring_network: String,
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
    /// Payment mode: "none" | "x402". Wave 4 (non-custodial) removed the
    /// custodial "balance"/"both" modes; `check_payment` fail-closes on any
    /// other value.
    pub payment_mode: String,
    /// Solana pubkey that receives USDC payments
    pub treasury_pubkey: String,
    /// USDC SPL mint address (mainnet: EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)
    pub usdc_mint: String,
    /// Admin bearer token gating operator-only endpoints (P&L on /admin/stats).
    /// Empty (default) = the P&L endpoint is disabled (fail-closed); public
    /// onboarding stats remain available on /stats.
    pub admin_token: String,
    /// EVM x402 settlement (Wave 1). All three must be set to enable the EVM
    /// rail; any empty = EVM x402 disabled (Solana-only). No external wallet —
    /// the client self-signs the ERC-20 transfer (see noncustodial design §19).
    pub evm_rpc_url: String,
    pub evm_usdc_token: String,
    pub evm_treasury: String,
    /// Minimum / initial cost of mnemonic_sign_memory in micro-USDC (floor price)
    pub sign_memory_cost_micro_usdc: i64,

    // ── Universal Paywall integration (exact x402 rail) ───────────────────────
    /// Optional URL of the Universal Paywall synchronous session API.
    /// When set, the x402 gate is routed through Universal Paywall instead of
    /// being verified directly against a Solana/EVM RPC.
    pub universal_paywall_url: String,
    /// API key for the Universal Paywall service.
    pub universal_paywall_api_key: String,
    /// Network identifier for the operation binding, e.g. "eip155:31337".
    pub universal_paywall_network: String,
    /// USDC token contract address on the target EVM chain.
    pub universal_paywall_asset: String,
    /// EVM address that receives settled USDC.
    pub universal_paywall_pay_to: String,
    /// EVM address of the payer whose wallet signs the EIP-3009 authorization.
    pub universal_paywall_payer_wallet: String,
    /// Base URL the browser approval page is served from.
    pub universal_paywall_approval_url_base: String,
    /// EIP-712 domain name for the USDC token contract (default "USD Coin").
    pub universal_paywall_eip712_name: String,
    /// EIP-712 domain version for the USDC token contract (default "2").
    pub universal_paywall_eip712_version: String,

    // ── Embedded approval page (production browser UI) ─────────────────────────
    /// Absolute path to the built @universal-paywall/approval-ui dist directory.
    /// When empty, the /approve route and static assets are NOT mounted.
    pub approval_ui_dist: PathBuf,
    /// Test-only hex private key that enables /api/mock-sign. Empty = disabled.
    pub approval_mock_signer: String,
    /// RPC URL advertised to the browser for wallet_addEthereumChain.
    pub approval_chain_rpc_url: String,
    /// Human-readable chain name advertised to the browser.
    pub approval_chain_name: String,
    /// Native currency symbol for the advertised chain.
    pub approval_chain_currency_symbol: String,
    /// Native currency decimals for the advertised chain.
    pub approval_chain_currency_decimals: u8,

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

    // ── Chain-backed traction stats (recover-traction-from-chain) ───────────
    /// Comma-separated base58 Solana pubkeys of every wallet that ever signed
    /// anchored uploads (the funded server keypair(s), current and historic).
    /// Non-empty enables the Arweave-GraphQL snapshot that lets `/stats` and
    /// `/analytics/attestations` survive a total DB loss. Empty (default) =
    /// disabled, DB-only behaviour. Env: `CHAIN_STATS_WALLETS`.
    pub chain_stats_wallets: Vec<String>,
    /// Gateway GraphQL endpoint used to enumerate anchored items.
    /// Env: `CHAIN_STATS_GRAPHQL_URL`.
    pub chain_stats_graphql_url: String,
    /// Gateway base URL for fetching item payloads (legacy producer
    /// backfill). Defaults to the Irys gateway: this node uploads via
    /// Irys, and arweave.net serves an HTML placeholder page (HTTP 200!)
    /// for Irys-bundled items it never indexed — verified live 2026-07-09.
    /// Env: `CHAIN_STATS_GATEWAY_URL`.
    pub chain_stats_gateway_url: String,
    /// Snapshot refresh interval in seconds. Default 3600.
    /// Env: `CHAIN_STATS_REFRESH_SECS`.
    pub chain_stats_refresh_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let home = dirs_home();
        Self {
            transport: env_or("MCP_TRANSPORT", "http"),
            http_host: env_or("MCP_HTTP_HOST", "0.0.0.0"),
            http_port: env_or("MCP_HTTP_PORT", "3000").parse().unwrap_or(3000),
            solana_rpc_url: env_or("SOLANA_RPC_URL", "http://localhost:8899"),
            arweave_url: env_or_fallback(
                "IRYS_GATEWAY_URL",
                "ARWEAVE_URL",
                "http://localhost:1984",
            ),
            anchoring_network: env_or("ANCHORING_NETWORK", "mainnet"),
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
            admin_token: env_or("ADMIN_TOKEN", ""),
            evm_rpc_url: env_or("EVM_RPC_URL", ""),
            evm_usdc_token: env_or("EVM_USDC_TOKEN", ""),
            evm_treasury: env_or("EVM_TREASURY", ""),
            sign_memory_cost_micro_usdc: env_or("SIGN_MEMORY_COST_MICRO_USDC", "1000")
                .parse()
                .unwrap_or(1000),
            universal_paywall_url: env_or("UNIVERSAL_PAYWALL_URL", ""),
            universal_paywall_api_key: env_or("UNIVERSAL_PAYWALL_API_KEY", ""),
            universal_paywall_network: env_or("UNIVERSAL_PAYWALL_NETWORK", ""),
            universal_paywall_asset: env_or("UNIVERSAL_PAYWALL_ASSET", ""),
            universal_paywall_pay_to: env_or("UNIVERSAL_PAYWALL_PAY_TO", ""),
            universal_paywall_payer_wallet: env_or("UNIVERSAL_PAYWALL_PAYER_WALLET", ""),
            universal_paywall_approval_url_base: env_or("UNIVERSAL_PAYWALL_APPROVAL_URL_BASE", ""),
            universal_paywall_eip712_name: env_or("UNIVERSAL_PAYWALL_EIP712_NAME", "USD Coin"),
            universal_paywall_eip712_version: env_or("UNIVERSAL_PAYWALL_EIP712_VERSION", "2"),
            approval_ui_dist: expand_path(&env_or("MNEMONIC_APPROVAL_UI_DIST", "")),
            approval_mock_signer: env_or("MNEMONIC_APPROVAL_MOCK_SIGNER", ""),
            approval_chain_rpc_url: env_or("UNIVERSAL_PAYWALL_CHAIN_RPC_URL", ""),
            approval_chain_name: env_or("UNIVERSAL_PAYWALL_CHAIN_NAME", ""),
            approval_chain_currency_symbol: env_or(
                "UNIVERSAL_PAYWALL_CHAIN_CURRENCY_SYMBOL",
                "ETH",
            ),
            approval_chain_currency_decimals: env_or(
                "UNIVERSAL_PAYWALL_CHAIN_CURRENCY_DECIMALS",
                "18",
            )
            .parse()
            .unwrap_or(18),
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
            chain_stats_wallets: env_or("CHAIN_STATS_WALLETS", "")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            chain_stats_graphql_url: env_or(
                "CHAIN_STATS_GRAPHQL_URL",
                "https://arweave.net/graphql",
            ),
            chain_stats_gateway_url: env_or("CHAIN_STATS_GATEWAY_URL", "https://gateway.irys.xyz"),
            chain_stats_refresh_secs: env_or("CHAIN_STATS_REFRESH_SECS", "3600")
                .parse()
                .unwrap_or(3600),
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

    /// Resolve the requested anchoring network. Kept separate from
    /// `from_env` so startup can emit a clear fatal configuration error.
    pub fn resolved_anchoring_network(&self) -> Result<AnchoringNetwork, String> {
        self.anchoring_network.parse()
    }

    /// In Devnet mode, every external anchoring endpoint is fixed and
    /// non-production. This is deliberately stricter than the legacy mainnet
    /// configuration, which supports custom gateways and local test rigs.
    pub fn validate_anchoring_config(&self) -> Result<AnchoringNetwork, String> {
        let network = self.resolved_anchoring_network()?;
        if network == AnchoringNetwork::Devnet {
            validate_exact_https_origin(
                "SOLANA_RPC_URL",
                &self.solana_rpc_url,
                "https://api.devnet.solana.com",
            )?;
            validate_exact_https_origin(
                "IRYS_GATEWAY_URL",
                &self.arweave_url,
                "https://devnet.irys.xyz",
            )?;
        }
        Ok(network)
    }

    pub fn irys_network(&self) -> Result<IrysNetwork, String> {
        Ok(self.resolved_anchoring_network()?.as_irys_network())
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

fn env_or_fallback(preferred_key: &str, legacy_key: &str, default: &str) -> String {
    std::env::var(preferred_key)
        .or_else(|_| std::env::var(legacy_key))
        .unwrap_or_else(|_| default.to_string())
}

fn validate_exact_https_origin(key: &str, value: &str, expected: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value)
        .map_err(|error| format!("{key} is not a valid URL ({value}): {error}"))?;
    let normalized = parsed.origin().ascii_serialization();
    if parsed.scheme() != "https"
        || normalized != expected
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(format!(
            "ANCHORING_NETWORK=devnet requires {key}={expected}, got: {value}"
        ));
    }
    Ok(())
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

    #[test]
    fn resolves_supported_anchoring_networks() {
        assert_eq!(
            "mainnet".parse::<AnchoringNetwork>(),
            Ok(AnchoringNetwork::Mainnet)
        );
        assert_eq!(
            "devnet".parse::<AnchoringNetwork>(),
            Ok(AnchoringNetwork::Devnet)
        );
        assert!("testnet".parse::<AnchoringNetwork>().is_err());
    }

    #[test]
    fn devnet_rejects_production_and_custom_endpoints() {
        let mut cfg = Config::from_env();
        cfg.anchoring_network = "devnet".to_string();
        cfg.solana_rpc_url = "https://api.mainnet-beta.solana.com".to_string();
        cfg.arweave_url = "https://gateway.irys.xyz".to_string();
        assert!(cfg.validate_anchoring_config().is_err());

        cfg.solana_rpc_url = "https://api.devnet.solana.com".to_string();
        cfg.arweave_url = "https://devnet.irys.xyz".to_string();
        assert_eq!(
            cfg.validate_anchoring_config(),
            Ok(AnchoringNetwork::Devnet)
        );

        // A trailing slash is semantically the same origin, but a path must
        // not be accepted: it could point uploads/reads at an unexpected API.
        cfg.arweave_url = "https://devnet.irys.xyz/".to_string();
        assert_eq!(
            cfg.validate_anchoring_config(),
            Ok(AnchoringNetwork::Devnet)
        );
        cfg.arweave_url = "https://devnet.irys.xyz/custom".to_string();
        assert!(cfg.validate_anchoring_config().is_err());
    }
}
