//! `sign_pending` — sign a canonical-CBOR pending bundle with a test
//! Ed25519 keypair (Decision 12 webapp WASM signer, native equivalent).
//!
//! Used by `scripts/test-deferred-sign-flow.sh` to drive the
//! `GET /api/pending/<id>` -> `POST /api/sign-callback` round-trip from
//! bash without needing a browser. The native binary mirrors what the
//! webapp's `sign_attestation_bundle` (T2) does in WASM.
//!
//! Usage:
//!     cargo run -p mnemonic-mcp --example sign_pending -- \
//!         --cbor-file <path> --keypair-base58 <secret>
//!
//! Output: stdout is base64 of the COSE_Sign1 envelope. stderr is the
//! recovered base58 pubkey (also written for the bash caller). Exits non-
//! zero on any error.

use clap::Parser;
use mnemonic_core::codec::sign::sign_cose;
use solana_sdk::signature::{Keypair, Signer};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "sign_pending",
    about = "Sign a canonical-CBOR bundle with a test Ed25519 keypair (Decision 12)"
)]
struct Cli {
    /// Path to the canonical-CBOR bytes file (binary, not base64).
    #[arg(long)]
    cbor_file: std::path::PathBuf,
    /// 64-byte keypair (secret + public) as base64 (standard alphabet).
    /// If omitted, generates a fresh keypair and prints it to stderr
    /// (NEVER use in production).
    #[arg(long)]
    secret_base64: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let cbor_bytes = match std::fs::read(&cli.cbor_file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read cbor file {:?}: {e}", cli.cbor_file);
            return ExitCode::from(1);
        }
    };

    let kp = match cli.secret_base64 {
        Some(s) => {
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s.as_bytes()) {
                Ok(bytes) => match Keypair::try_from(bytes.as_slice()) {
                    Ok(k) => k,
                    Err(e) => {
                        eprintln!("failed to parse keypair from secret_base64: {e}");
                        return ExitCode::from(1);
                    }
                },
                Err(e) => {
                    eprintln!("secret_base64 is not valid base64: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        None => {
            let kp = Keypair::new();
            let secret =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, kp.to_bytes());
            eprintln!("WARNING: generated fresh keypair (test only). secret_base64={secret}");
            kp
        }
    };

    eprintln!("signer_pubkey={}", kp.pubkey());

    let cose = match sign_cose(&cbor_bytes, &kp) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("sign_cose failed: {e}");
            return ExitCode::from(2);
        }
    };

    let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cose);
    println!("{cose_b64}");
    ExitCode::SUCCESS
}
