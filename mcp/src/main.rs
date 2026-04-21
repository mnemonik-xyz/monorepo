mod arweave;
mod codec;
mod compress;
mod config;
mod db;
mod embed;
mod identity;
mod lineage;
mod mcp;
mod payment;
mod pricing;
mod solana;
mod tools;

use std::sync::Arc;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use serde::Deserialize;
use solana_sdk::signer::Signer;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "mnemonic-mcp", about = "Mnemonic MCP server — verifiable memory attestation")]
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

/// /mcp — payment-aware MCP JSON-RPC dispatcher.
async fn mcp_handler(
    State(state): State<Arc<mcp::McpState>>,
    headers: HeaderMap,
    Json(req): Json<mcp::JsonRpcRequest>,
) -> Response {
    let is_sign_memory = req.method == "tools/call"
        && req.params.get("name").and_then(|n| n.as_str()) == Some("mnemonic_sign_memory");

    if is_sign_memory && state.payment_mode != "none" && state.storage_mode != "local" {
        // Use live price from pricing engine (refreshed in background)
        let current_cost = state.pricing.current_price();
        let gate = payment::check_payment(
            &headers,
            &state.payment_mode,
            &state.store,
            &state.solana,
            &state.treasury_pubkey,
            &state.usdc_mint,
            current_cost,
        )
        .await;

        match gate {
            payment::PaymentGate::Proceed(api_key) => {
                // Deduct balance BEFORE executing the tool (reserve funds)
                if let Some(ref key) = api_key {
                    let store = state.store.lock().unwrap();
                    if let Err(e) = store.deduct_balance(
                        key,
                        state.sign_memory_cost_micro_usdc,
                        "mnemonic_sign_memory",
                    ) {
                        let err_body = serde_json::json!({
                            "jsonrpc": "2.0", "id": req.id,
                            "error": {"code": -32600, "message": format!("payment failed: {e}")}
                        });
                        return (StatusCode::PAYMENT_REQUIRED, Json(err_body)).into_response();
                    }
                }

                let resp = mcp::handle_request(&req, &state).await;

                // Refund on tool failure
                if resp.error.is_some() {
                    if let Some(ref key) = api_key {
                        let store = state.store.lock().unwrap();
                        let _ = store.credit_deposit(
                            key,
                            current_cost,
                            &format!("refund:{}",
                                resp.error.as_ref().map(|e| e.message.as_str()).unwrap_or("error")),
                        );
                    }
                }

                Json(resp).into_response()
            }
            payment::PaymentGate::NeedPayment(x402) => {
                (StatusCode::PAYMENT_REQUIRED, Json(x402)).into_response()
            }
            payment::PaymentGate::Unauthorized(msg) => {
                let err_body = serde_json::json!({
                    "jsonrpc": "2.0", "id": req.id,
                    "error": {"code": -32600, "message": msg}
                });
                (StatusCode::UNAUTHORIZED, Json(err_body)).into_response()
            }
        }
    } else {
        Json(mcp::handle_request(&req, &state).await).into_response()
    }
}

/// POST /api-keys — create a pre-funded API key (zero initial balance).
async fn create_api_key(
    State(state): State<Arc<mcp::McpState>>,
    Json(body): Json<CreateKeyRequest>,
) -> Response {
    let owner = body.owner_pubkey.as_deref().unwrap_or("");
    let store = state.store.lock().unwrap();
    match store.create_api_key(owner) {
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
    match store.get_balance(&q.api_key) {
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
    let amount = match state
        .solana
        .verify_usdc_transfer(
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
        match store.get_owner_pubkey(&body.api_key) {
            Ok(Some(pk)) if !pk.is_empty() => pk,
            Ok(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "api key has no owner_pubkey — cannot verify deposit sender"
                    })),
                ).into_response()
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                ).into_response()
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
            ).into_response()
        }
    };

    if !signers.iter().any(|s| s == &owner_pubkey) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": format!(
                    "deposit rejected: API key owner ({}) is not a signer of tx {}. \
                     Only the wallet that signed the deposit can credit this key.",
                    &owner_pubkey[..owner_pubkey.len().min(12)],
                    &body.tx_sig[..body.tx_sig.len().min(16)],
                ),
            })),
        ).into_response();
    }

    let store = state.store.lock().unwrap();
    match store.credit_deposit(&body.api_key, amount as i64, &body.tx_sig) {
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
    match store.get_pnl_stats(days) {
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
    ).unwrap_or_else(|e| {
        tracing::error!("FATAL: {e}");
        std::process::exit(1);
    });
    let dim = embedder.dim();
    tracing::info!(
        "Embedder: {} ({}-dim, model={}, verifiable={})",
        embedder.provider_name(), dim, embedder.model_id(), embedder.is_open_weights(),
    );

    let compressor = compress::EmbeddingCompressor::new(dim, cfg.turbo_bits, 42);
    tracing::info!(
        "Compressor: TurboQuant {}-bit ({:.1}x ratio)",
        cfg.turbo_bits,
        compressor.compression_ratio()
    );

    tracing::info!("Storage mode: {} ({})", cfg.storage_mode,
        if cfg.storage_mode == "local" { "free, SQLite only" } else { "Arweave + Solana + SQLite" });
    tracing::info!("Payment mode: {}", cfg.payment_mode);

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

    let store = db::AttestationStore::open(&cfg.database_path)?;
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
    });

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

        let resp = mcp::handle_request(&req, &state).await;
        stdout
            .write_all(serde_json::to_string(&resp)?.as_bytes())
            .await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

// ── HTTP transport ────────────────────────────────────────────────────────────

async fn run_http(state: Arc<mcp::McpState>, host: &str, port: u16) -> anyhow::Result<()> {
    use tower_http::cors::{Any, CorsLayer};

    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/api-keys", post(create_api_key))
        .route("/balance", get(get_balance))
        .route("/deposit", post(deposit))
        .route("/admin/stats", get(admin_stats))
        .route("/health", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let addr = format!("{host}:{port}");
    tracing::info!("MCP server listening on http://{addr}/mcp");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
