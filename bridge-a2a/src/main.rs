//! `bridge-a2a` binary entry point.
//!
//! All logic lives in the library (`src/lib.rs`); main only handles CLI
//! parsing, wiring, and the axum server loop.

use std::net::SocketAddr;

use axum::{routing::any, Router};
use bridge_a2a::{config::{Cli, StorageMode}, middleware, state::AppState};
use clap::Parser;
use mnemonic_core::storage::SqliteStore;
use solana_sdk::signature::Keypair;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let keypair = load_or_generate_keypair(cli.keypair.as_deref())?;
    info!(
        pubkey = %solana_sdk::signature::Signer::pubkey(&keypair),
        "bridge-a2a starting"
    );

    if cli.storage == StorageMode::Full {
        tracing::warn!(
            "storage=full is not yet implemented; falling back to local SQLite"
        );
    }
    let store = SqliteStore::open(std::path::Path::new(&cli.db_path))?;

    let state = AppState::new(
        keypair,
        store,
        cli.upstream.clone(),
        cli.context_strategy.clone(),
        cli.failure_mode.clone(),
    );

    let addr: SocketAddr = cli.listen.parse()?;
    info!(listen = %addr, upstream = %cli.upstream, "proxy ready");

    let app = Router::new()
        .route("/{*path}", any(middleware::proxy_handler))
        .route("/", any(middleware::proxy_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_or_generate_keypair(path: Option<&str>) -> anyhow::Result<Keypair> {
    use anyhow::Context as _;

    match path {
        None => {
            let kp = Keypair::new();
            tracing::warn!(
                "no --keypair path provided; using ephemeral keypair \
                 (attestations will not survive restart)"
            );
            Ok(kp)
        }
        Some(p) => {
            let raw = std::fs::read(p)
                .with_context(|| format!("failed to read keypair file: {p}"))?;
            let bytes: Vec<u8> = serde_json::from_slice(&raw)
                .with_context(|| format!("keypair file is not a JSON byte array: {p}"))?;
            Keypair::try_from(bytes.as_slice())
                .with_context(|| format!("invalid keypair bytes in {p}"))
        }
    }
}
