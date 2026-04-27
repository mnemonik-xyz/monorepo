//! `mint-test-jwt` — small helper binary that issues a valid HS256 JWT
//! against `MCP_JWT_SECRET` for a given subject. Used by the smoke harness
//! (`scripts/test-oauth-flow.sh`) and by Task 8's CI to mint an
//! authenticated `tools/call` request without walking the full OAuth flow.
//!
//! The token's claim set matches `mnemonic_mcp::oauth::Claims` exactly:
//! `iss="mcp.mnemonik.xyz"`, `aud="mcp"`, `sub=<argv>`, `iat=now`,
//! `exp=now+3600`, `jti=Uuid::new_v4()`.
//!
//! Usage:
//!     MCP_JWT_SECRET=$(openssl rand -base64 32) \
//!         cargo run -p mnemonic-mcp --bin mint-test-jwt -- --sub <pubkey>
//!
//! Aborts on missing/short secret with a non-zero exit so CI fails loudly
//! rather than printing a token signed with a dev fallback.

use std::process::ExitCode;

use clap::Parser;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(
    name = "mint-test-jwt",
    about = "Mint a test HS256 JWT for the mnemonic-mcp server"
)]
struct Cli {
    /// JWT subject (base58 user pubkey).
    #[arg(long)]
    sub: String,
    /// JWT TTL in seconds (default 3600 = 1h, matching production).
    #[arg(long, default_value_t = 3600)]
    ttl: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    iat: u64,
    exp: u64,
    jti: String,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let secret_raw = match std::env::var("MCP_JWT_SECRET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("MCP_JWT_SECRET env var is required (32-byte base64)");
            return ExitCode::from(1);
        }
    };
    let secret_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        secret_raw.trim(),
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("MCP_JWT_SECRET is not valid base64: {e}");
            return ExitCode::from(1);
        }
    };
    if secret_bytes.len() < 32 {
        eprintln!(
            "MCP_JWT_SECRET decoded to {} bytes — must be >= 32",
            secret_bytes.len()
        );
        return ExitCode::from(1);
    }

    let now = now_secs();
    let claims = Claims {
        iss: "mcp.mnemonik.xyz".to_string(),
        aud: "mcp".to_string(),
        sub: cli.sub,
        iat: now,
        exp: now + cli.ttl,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let token = match encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&secret_bytes),
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("JWT encode failed: {e}");
            return ExitCode::from(2);
        }
    };
    println!("{token}");
    ExitCode::SUCCESS
}
