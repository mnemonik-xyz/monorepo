mod api;
mod chat;
mod config;
mod cors_policy;
mod llm;
mod mcp;
mod oauth;
mod payment;
mod pending;
mod pricing;
mod seed;
mod tools;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use mnemonic_core::{arweave, compress, embed, identity, solana, storage::SqliteStore};
use serde::Deserialize;
use solana_sdk::signer::Signer;
use std::sync::Arc;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "mnemonic-mcp",
    about = "Mnemonic MCP server — verifiable memory attestation"
)]
struct Cli {
    /// Transport: "stdio" or "http"
    #[arg(long, default_value = "http")]
    transport: String,

    /// HTTP port (when transport=http)
    #[arg(long, default_value = "3000")]
    port: u16,

    /// HTTP host (when transport=http)
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
}

// ── HTTP request/response types ───────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateKeyRequest {
    owner_pubkey: Option<String>,
}

#[derive(Deserialize)]
struct BalanceQuery {
    api_key: String,
}

#[derive(Deserialize)]
struct DepositRequest {
    api_key: String,
    tx_sig: String,
}

// ── Axum handlers ─────────────────────────────────────────────────────────────
//
// The `/mcp` JSON-RPC dispatcher (streamable HTTP per Decision 1) lives in
// `mcp::mcp_handler` so it can be unit-tested directly from `mcp.rs::tests`.
// The other endpoints below (api keys, balance, deposit, admin) are plain
// JSON request/response — they do not need the streaming envelope.

/// POST /api-keys — create a pre-funded API key (zero initial balance).
async fn create_api_key(
    State(state): State<Arc<mcp::McpState>>,
    Json(body): Json<CreateKeyRequest>,
) -> Response {
    let owner = body.owner_pubkey.as_deref().unwrap_or("");
    let store = state.store.lock().unwrap();
    match payment::create_api_key(&store, owner) {
        Ok(key) => Json(serde_json::json!({
            "api_key": key,
            "balance_micro_usdc": 0,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /balance?api_key=<key> — query balance.
async fn get_balance(
    State(state): State<Arc<mcp::McpState>>,
    Query(q): Query<BalanceQuery>,
) -> Response {
    let store = state.store.lock().unwrap();
    match payment::get_balance(&store, &q.api_key) {
        Ok(Some(bal)) => Json(serde_json::json!({
            "api_key": q.api_key,
            "balance_micro_usdc": bal,
            "balance_usdc": bal as f64 / 1_000_000.0,
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "api key not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /deposit — credit a confirmed on-chain USDC transfer to an API key balance.
///
/// Flow: caller sends USDC to treasury on Solana, then POSTs the tx_sig here.
/// Server verifies the on-chain transfer and credits the key.
async fn deposit(
    State(state): State<Arc<mcp::McpState>>,
    Json(body): Json<DepositRequest>,
) -> Response {
    // Verify the on-chain USDC transfer and get the amount
    let amount = match payment::verify_usdc_transfer(
        &state.solana,
        &body.tx_sig,
        &state.treasury_pubkey,
        &state.usdc_mint,
        1, // at least 1 micro-USDC
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "transaction does not transfer USDC to treasury"
                })),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("solana rpc error: {e}")})),
            )
                .into_response()
        }
    };

    // Look up the API key's owner_pubkey (short lock scope, no await)
    let owner_pubkey = {
        let store = state.store.lock().unwrap();
        match payment::get_owner_pubkey(&store, &body.api_key) {
            Ok(Some(pk)) if !pk.is_empty() => pk,
            Ok(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "api key has no owner_pubkey — cannot verify deposit sender"
                    })),
                )
                    .into_response()
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response()
            }
        }
    }; // lock released here

    // Verify that the API key's owner_pubkey is a signer of the deposit transaction.
    // This prevents front-running: only the wallet that signed the tx can credit the key.
    let signers = match state.solana.get_tx_signers(&body.tx_sig).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("failed to fetch tx signers: {e}")})),
            )
                .into_response()
        }
    };

    if !signers.iter().any(|s| s == &owner_pubkey) {
        // Log full identifiers server-side for operator debugging, but do
        // NOT leak any pubkey/tx_sig prefix to the client. The caller already
        // knows their own pubkey + tx_sig; adding partial values to the
        // response body only narrows key space for third-party observers.
        tracing::warn!(
            owner_pubkey = %owner_pubkey,
            tx_sig = %body.tx_sig,
            signers = ?signers,
            "deposit rejected: API key owner is not a signer of this transaction"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "deposit rejected: API key owner is not a signer of this transaction"
            })),
        )
            .into_response();
    }

    let store = state.store.lock().unwrap();
    match payment::credit_deposit(&store, &body.api_key, amount as i64, &body.tx_sig) {
        Ok(new_balance) => Json(serde_json::json!({
            "api_key": body.api_key,
            "deposited_micro_usdc": amount,
            "new_balance_micro_usdc": new_balance,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /admin/stats?days=<N> — P&L summary for the last N days (default 7).
async fn admin_stats(
    State(state): State<Arc<mcp::McpState>>,
    Query(q): Query<StatsQuery>,
) -> Response {
    let days = q.days.unwrap_or(7);
    let store = state.store.lock().unwrap();
    match payment::get_pnl_stats(&store, days) {
        Ok(stats) => Json(serde_json::json!({
            "period_days": stats.period_days,
            "attestations": stats.attestations,
            "earned_micro_usdc": stats.earned_micro_usdc,
            "earned_usdc": stats.earned_micro_usdc as f64 / 1_000_000.0,
            "cost_sol_lamports": stats.cost_sol_lamports,
            "cost_micro_usdc_equiv": stats.cost_micro_usdc_equiv,
            "cost_usdc_equiv": stats.cost_micro_usdc_equiv as f64 / 1_000_000.0,
            "net_micro_usdc": stats.net_micro_usdc,
            "net_usdc": stats.net_micro_usdc as f64 / 1_000_000.0,
            "margin_pct": (stats.margin_pct * 10.0).round() / 10.0,
            "avg_sol_price_usdc": stats.avg_sol_price_usdc,
            "pricing": {
                "current_price_micro_usdc": state.pricing.current_price(),
                "current_sol_price_usdc": state.pricing.current_sol_price(),
                "current_irys_lamports": state.pricing.current_irys_lamports(),
            },
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct StatsQuery {
    days: Option<u64>,
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let cfg = config::Config::from_env();

    let transport = if std::env::var("MCP_TRANSPORT").is_ok() {
        cfg.transport.clone()
    } else {
        cli.transport.clone()
    };

    let keypair = identity::load_or_create_keypair(&cfg.keypair_path)?;
    tracing::info!("Identity: {}", keypair.pubkey());
    tracing::info!("did:sol: {}", identity::did_sol(&keypair));

    let embedder = embed::build_embedder(
        &cfg.embed_provider,
        &cfg.openai_api_key,
        &cfg.openai_embed_model,
    )
    .unwrap_or_else(|e| {
        tracing::error!("FATAL: {e}");
        std::process::exit(1);
    });
    let dim = embedder.dim();
    tracing::info!(
        "Embedder: {} ({}-dim, model={}, verifiable={})",
        embedder.provider_name(),
        dim,
        embedder.model_id(),
        embedder.is_open_weights(),
    );

    let compressor = compress::EmbeddingCompressor::new(dim, cfg.turbo_bits, 42);
    tracing::info!(
        "Compressor: TurboQuant {}-bit ({:.1}x ratio)",
        cfg.turbo_bits,
        compressor.compression_ratio()
    );

    tracing::info!(
        "Storage mode: {} ({})",
        cfg.storage_mode,
        if cfg.storage_mode == "local" {
            "free, SQLite only"
        } else {
            "Arweave + Solana + SQLite"
        }
    );
    tracing::info!("Payment mode: {}", cfg.payment_mode);

    // ── Ollama URL validation (SSRF prevention, Decision 8) ──────────────────
    if let Err(msg) = cfg.validate_ollama_url() {
        tracing::error!("FATAL: {msg}");
        std::process::exit(1);
    }
    tracing::info!(
        "Ollama URL: {} (model: {})",
        cfg.ollama_url,
        cfg.ollama_model
    );
    tracing::info!("RAG chunk dir: {}", cfg.rag_chunk_dir.display());

    // ── Pricing engine ────────────────────────────────────────────────────────
    let pricing_cfg = pricing::PricingConfig {
        margin_bps: cfg.pricing_margin_bps,
        min_price_micro_usdc: cfg.sign_memory_cost_micro_usdc,
        typical_payload_bytes: cfg.typical_payload_bytes,
        sol_tx_fee_lamports: cfg.sol_tx_fee_lamports,
    };
    let pricing = pricing::PricingEngine::new(cfg.sign_memory_cost_micro_usdc);

    // Attempt an initial price fetch (non-fatal — falls back to floor price)
    if let Err(e) = pricing.refresh(&pricing_cfg).await {
        tracing::warn!("initial pricing refresh failed (using floor price): {e}");
    }
    tracing::info!(
        price_micro_usdc = pricing.current_price(),
        sol_usdc = pricing.current_sol_price(),
        "pricing engine ready"
    );

    // Spawn background refresh loop
    {
        let pricing = pricing.clone();
        let refresh_secs = cfg.price_refresh_secs;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(refresh_secs)).await;
                if let Err(e) = pricing.refresh(&pricing_cfg).await {
                    tracing::warn!("pricing refresh failed: {e}");
                }
            }
        });
    }

    let store = SqliteStore::open(&cfg.database_path)?;
    // ── T14: Google OAuth identity-link table (idempotent migration) ─────────
    // Lives in `mcp/` per Decision 9 (`core/` reserved for the cross-client
    // attestation schema). No-op when the table already exists.
    if let Err(e) = oauth::google::migrate_google_identity_links(store.conn()) {
        tracing::error!("FATAL: google_identity_links migration failed: {e}");
        std::process::exit(1);
    }
    // Chat rate limiter: 10 requests per 60 seconds per IP
    let chat_limiter = {
        use governor::Quota;
        use std::num::NonZeroU32;
        let quota = Quota::per_minute(NonZeroU32::new(10).unwrap());
        governor::RateLimiter::keyed(quota)
    };

    // ── LLM provider abstraction ────────────────────────────────────────────
    let llm_client = llm::LlmClient::new(
        &cfg.llm_provider,
        &cfg.llm_api_key,
        &cfg.llm_model,
        &cfg.llm_api_url,
        cfg.llm_max_tokens,
    )
    .unwrap_or_else(|e| {
        tracing::error!("FATAL: {e}");
        std::process::exit(1);
    });
    tracing::info!(
        "LLM provider: {} (model: {}, url: {}, max_tokens: {})",
        cfg.llm_provider,
        llm_client.model,
        llm_client.base_url,
        llm_client.max_tokens,
    );

    // Shared reqwest client for Ollama calls -- redirect(Policy::none()) for SSRF prevention (Decision 8).
    let ollama_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build reqwest client for Ollama");

    // Browser-mediated signing buffer — Decision 12. Production caps:
    // 10k LRU entries, 300s TTL, 50 pending bundles per jwt.sub.
    let pending = Arc::new(pending::PendingBundles::with_defaults());

    // CLI bootstrap-ticket store (mnemonic-cli tech-spec Decision 7).
    // Production caps: 100 LRU entries, 600s TTL, 3 active tickets per
    // jwt.sub. Webapp issues tickets via /api/cli-bootstrap/issue (Bearer
    // JWT'd), CLI redeems via /api/cli-bootstrap/redeem/:ticket (UUID is
    // the capability — no Authorization header required).
    let bootstrap_tickets = Arc::new(api::BootstrapTickets::with_defaults());

    let state = Arc::new(mcp::McpState {
        keypair,
        solana: solana::SolanaClient::new(&cfg.solana_rpc_url),
        arweave: arweave::ArweaveClient::new(&cfg.arweave_url),
        store: std::sync::Mutex::new(store),
        embedder,
        compressor,
        payment_mode: cfg.payment_mode.clone(),
        treasury_pubkey: cfg.treasury_pubkey.clone(),
        usdc_mint: cfg.usdc_mint.clone(),
        sign_memory_cost_micro_usdc: cfg.sign_memory_cost_micro_usdc,
        pricing,
        sol_tx_fee_lamports: cfg.sol_tx_fee_lamports,
        storage_mode: cfg.storage_mode.clone(),
        ollama_url: cfg.ollama_url.clone(),
        ollama_model: cfg.ollama_model.clone(),
        rag_chunk_dir: cfg.rag_chunk_dir.clone(),
        llm_client,
        artifact_zip_path: std::sync::Mutex::new(None),
        ollama_client,
        chat_limiter,
        pending,
        bootstrap_tickets,
    });

    // ── RAG seeding (whitepaper chunking + artifact generation) ──────────
    if let Err(e) = seed::run(&state).await {
        tracing::error!("RAG seeding failed: {e}");
        // Non-fatal: server can still run without pre-seeded knowledge.
        // Chat endpoint will have empty recall results until manually seeded.
    }

    match transport.as_str() {
        "stdio" => run_stdio(state).await,
        "http" => run_http(state, &cli.host, cli.port).await,
        other => anyhow::bail!("unknown transport: {other} (use 'stdio' or 'http')"),
    }
}

// ── stdio transport ───────────────────────────────────────────────────────────
// stdio clients (Claude Code) run locally and are trusted — payment is skipped.

async fn run_stdio(state: Arc<mcp::McpState>) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    tracing::info!("MCP server running on stdio");
    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let req: mcp::JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}
                });
                stdout
                    .write_all(serde_json::to_string(&err_resp)?.as_bytes())
                    .await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };

        // Stdio path: no JWT, single-tenant CLI mode. Use the local keypair
        // pubkey as owner scope so attestations land under a stable owner
        // and `recall` returns the local user's rows. `jwt_sub = None`
        // routes `sign_memory` through the inline (server-signing) branch
        // rather than the deferred (PendingBundles) one — Decision 12.
        let owner_pubkey = state.keypair.pubkey().to_string();
        let resp = mcp::handle_request(&req, &state, &owner_pubkey, None).await;
        stdout
            .write_all(serde_json::to_string(&resp)?.as_bytes())
            .await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

// ── HTTP transport ────────────────────────────────────────────────────────────

// CORS allow-origin policy moved to `mcp/src/cors_policy.rs` so integration
// tests under `mcp/tests/cors.rs` can reach it via the library facade.

/// Load and decode `MCP_JWT_SECRET` from env. Per Decision 11 the secret must
/// be a base64-encoded value that decodes to >= 32 bytes. Aborts startup if
/// the env var is missing or shorter — silent fallback to a default secret
/// would be the kind of misconfiguration that leaks live JWTs.
fn load_jwt_secret() -> anyhow::Result<Vec<u8>> {
    let raw = std::env::var("MCP_JWT_SECRET")
        .map_err(|_| anyhow::anyhow!("MCP_JWT_SECRET env var is required (32-byte base64)"))?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw.trim())
        .map_err(|e| anyhow::anyhow!("MCP_JWT_SECRET is not valid base64: {e}"))?;
    if bytes.len() < 32 {
        anyhow::bail!(
            "MCP_JWT_SECRET decoded to {} bytes — must be >= 32",
            bytes.len()
        );
    }
    Ok(bytes)
}

async fn run_http(state: Arc<mcp::McpState>, host: &str, port: u16) -> anyhow::Result<()> {
    use axum::http::{header, Method};
    use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
    use tower_http::cors::{AllowOrigin, CorsLayer};

    // ── OAuth state + JWT secret (Decisions 9 + 11) ──────────────────────────
    let secret = load_jwt_secret()?;
    let oauth_state = Arc::new(oauth::OAuthState::new(&secret));
    tracing::info!(
        "OAuth state initialized (LRU cap {})",
        oauth::OAUTH_STATE_CAPACITY
    );

    // ── Per-IP rate limiters (Decision 9, tech-spec AC line 357) ─────────────
    // /mcp aggregates sign_memory + recall traffic. tower_governor caps by IP
    // before the bearer-auth check so 429s short-circuit before JWT verify.
    // sign_memory ≤ 5/min/IP, recall ≤ 30/min/IP — we apply the looser of the
    // two (30/min) at the route-level limiter and rely on PendingBundles
    // (Task 5) for the per-method 5/min cap on sign_memory.
    let mcp_governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2) // 30 / 60 = 0.5/s smoothed; per_second uses int
            .burst_size(30)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build /mcp governor config"))?,
    );
    // /oauth/* per-IP rate limit: 5 burst + 1 req/s refill ≈ 5 req/min/IP
    // production cap. Set OAUTH_RATELIMIT_DISABLE=1 to widen for e2e tests
    // (Playwright on a single dev IP runs ~12 /oauth/* calls per test
    // suite; 5/min would short-circuit them with HTTP 429).
    let oauth_disabled = std::env::var("OAUTH_RATELIMIT_DISABLE").ok().as_deref() == Some("1");
    let (oauth_per_sec, oauth_burst) = if oauth_disabled {
        (100u64, 1000u32) // effectively unlimited for test runs
    } else {
        (1u64, 5u32)
    };
    let oauth_governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(oauth_per_sec)
            .burst_size(oauth_burst)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build /oauth/* governor config"))?,
    );
    if oauth_disabled {
        tracing::warn!(
            "OAUTH_RATELIMIT_DISABLE=1 — /oauth/* rate limit relaxed; do NOT use in production"
        );
    }

    // ── Bearer-auth middleware on /mcp (Decision 9) ──────────────────────────
    // /oauth/* and /health are URI-allowlisted INSIDE the middleware; we still
    // attach the layer to the whole MCP-protected subset because the body-peek
    // logic for JSON-RPC method allowlisting (initialize / tools/list) only
    // makes sense on /mcp.
    // Two paths point to the SAME handler:
    //   - `/mcp` — explicit, used by Cursor + VS Code (deeplinks pass
    //     this URL verbatim, MCP_URL = "https://mcp.mnemonik.xyz/mcp")
    //   - `/`    — apex, used by Claude.ai (Anthropic's connector
    //     strips the path from the user-supplied URL and POSTs to root;
    //     we observed `Claude-User` POST/GET hitting `/` after a
    //     successful OAuth flow, returning 404 because we only had
    //     `/mcp` registered)
    //
    // Both routes are equivalent and run under the same bearer-auth +
    // governor stack. The duplication is intentional for client
    // compatibility — pruning either one breaks one of the connectors.
    let mcp_subrouter = Router::new()
        .route("/mcp", post(mcp::mcp_handler))
        .route("/", post(mcp::mcp_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state.clone(),
            oauth::bearer_auth_middleware,
        ))
        .layer(GovernorLayer {
            config: mcp_governor_conf,
        })
        .with_state(state.clone());

    // ── /api/* webapp surface (Decision 12) ──────────────────────────────────
    // GET /api/pending/{id} returns the unsigned canonical-CBOR for the
    // browser to sign; POST /api/sign-callback ingests the COSE_Sign1 and
    // persists the attestation. Both routes sit behind the same Bearer-auth
    // middleware as /mcp; non-JSON-RPC, so the body-peek allowlist
    // (`initialize` / `tools/list`) does not apply — every /api/* request
    // requires a valid JWT.
    let api_subrouter = Router::new()
        .route(
            "/api/pending/{correlation_id}",
            axum::routing::get(api::get_pending_handler),
        )
        .route("/api/sign-callback", post(api::sign_callback_handler))
        // CLI bootstrap-ticket flow — mnemonic-cli tech-spec Decision 7.
        // /issue requires Bearer JWT (enforced by bearer_auth_middleware,
        // which inserts Claims into the request extension before the
        // handler runs). /redeem/:ticket is UUID-as-capability, exempt
        // from the middleware via the URI allowlist above.
        .route(
            "/api/cli-bootstrap/issue",
            post(api::bootstrap_issue_handler),
        )
        .route(
            "/api/cli-bootstrap/redeem/{ticket}",
            axum::routing::get(api::bootstrap_redeem_handler),
        )
        .layer(middleware::from_fn_with_state(
            oauth_state.clone(),
            oauth::bearer_auth_middleware,
        ))
        .with_state(state.clone());

    // ── Google OAuth state (T14, optional) ───────────────────────────────────
    // Initialized unconditionally so `is_disabled()` can return true and the
    // handlers can short-circuit with 404. Routes themselves are only wired
    // when `GOOGLE_OAUTH_CLIENT_ID` is set so disabled deployments don't
    // mount unused handlers.
    let google_oauth_state = Arc::new(oauth::google::GoogleOAuthState::new(
        std::env::var("GOOGLE_OAUTH_CLIENT_ID").unwrap_or_default(),
        std::env::var("GOOGLE_OAUTH_CLIENT_SECRET").unwrap_or_default(),
        std::env::var("GOOGLE_OAUTH_REDIRECT_URI")
            .unwrap_or_else(|_| "https://mc.mnemonik.xyz/oauth/google/callback".to_string()),
    ));
    let google_enabled = !google_oauth_state.is_disabled();
    tracing::info!(
        "Google OAuth: {}",
        if google_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );

    // ── OAuth routes (Decision 10) ────────────────────────────────────────────
    // GET /oauth/authorize — bootstrap (per-method dispatch in axum).
    //   Accepts standard OAuth 2.1 + PKCE query params; validates S256;
    //   inserts a pending challenge under `state`; returns either JSON
    //   (programmatic clients) or 302 to the webapp consent page (browsers).
    // POST /oauth/authorize — challenge-signed callback (existing behavior).
    let oauth_routes = Router::new()
        .route(
            "/oauth/authorize",
            get(oauth::authorize_init_handler).post(oauth::authorize_handler),
        )
        .route("/oauth/token", post(oauth::token_handler))
        // RFC 7591 Dynamic Client Registration. Open registration: any client
        // POSTs its redirect_uris and gets back a client_id. Required for VS
        // Code / Claude.ai connector flows that abort with "DCR not supported"
        // when registration_endpoint is missing from the metadata.
        .route("/oauth/register", post(oauth::oauth_register_handler))
        .layer(GovernorLayer {
            config: oauth_governor_conf.clone(),
        })
        .with_state(oauth_state.clone());

    // ── CORS (Decision 9 + hotfix: widen for MCP-client origins) ─────────────
    // Anthropic / Cursor / ChatGPT connectors run inside the user's browser
    // and originate requests from `https://claude.ai`, `https://*.claude.ai`,
    // `https://cursor.sh`, `https://chatgpt.com` etc. The previous narrow
    // single-origin policy returned ACAO=mnemonik.xyz for ALL preflights,
    // breaking connector reachability with "Couldn't reach the MCP server".
    //
    // Predicate-based allow-origin echoes back the request origin if it
    // matches a trusted MCP-client root domain — see `is_allowed_cors_origin`.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            cors_policy::is_allowed_cors_origin(origin.as_bytes())
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    // ── /.well-known discovery (RFC 8414 + MCP spec) ─────────────────────────
    // Anthropic's MCP connector probes these BEFORE attempting OAuth. Public
    // metadata, no auth, no state — a separate sub-router avoids dragging
    // bearer-auth or governor onto a 200-byte JSON response.
    let well_known_routes = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::oauth_authorization_server_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::oauth_protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth::oauth_protected_resource_metadata_mcp),
        );

    // ── Google OAuth routes (T14, conditional) ───────────────────────────────
    // Two-layer mount: public start/callback don't go through bearer-auth;
    // lookup/link sit behind bearer-auth so they can read JWT claims.
    let google_public_routes = if google_enabled {
        Router::new()
            .route(
                "/oauth/google/start",
                get(oauth::google::google_start_handler),
            )
            .route(
                "/oauth/google/callback",
                get(oauth::google::google_callback_handler),
            )
            .layer(GovernorLayer {
                config: oauth_governor_conf.clone(),
            })
            .with_state((
                state.clone(),
                oauth_state.clone(),
                google_oauth_state.clone(),
            ))
    } else {
        Router::new()
    };
    // Lookup + link auth is enforced inline (the global bearer-auth middleware
    // URI-allowlists `/oauth/*` and would short-circuit before checking JWTs
    // on these routes). Same governor rate limit as the rest of /oauth/*.
    let google_authed_lookup = if google_enabled {
        Router::new()
            .route(
                "/oauth/google/lookup",
                post(oauth::google::google_lookup_handler),
            )
            .layer(GovernorLayer {
                config: oauth_governor_conf.clone(),
            })
            .with_state((
                state.clone(),
                oauth_state.clone(),
                google_oauth_state.clone(),
            ))
    } else {
        Router::new()
    };
    let google_authed_link = if google_enabled {
        Router::new()
            .route(
                "/oauth/google/link",
                post(oauth::google::google_link_handler),
            )
            .layer(GovernorLayer {
                config: oauth_governor_conf.clone(),
            })
            .with_state((
                state.clone(),
                oauth_state.clone(),
                google_oauth_state.clone(),
            ))
    } else {
        Router::new()
    };

    let app = Router::new()
        .route("/chat", post(chat::chat_handler))
        .route("/api-keys", post(create_api_key))
        .route("/balance", get(get_balance))
        .route("/deposit", post(deposit))
        .route("/admin/stats", get(admin_stats))
        .route("/download-knowledge", get(chat::download_knowledge_handler))
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .with_state(state)
        .merge(mcp_subrouter)
        .merge(api_subrouter)
        .merge(oauth_routes)
        .merge(google_public_routes)
        .merge(google_authed_lookup)
        .merge(google_authed_link)
        .merge(well_known_routes)
        .layer(cors);

    let addr = format!("{host}:{port}");
    tracing::info!("MCP server listening on http://{addr}/mcp");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
