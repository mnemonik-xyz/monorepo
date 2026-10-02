//! Chain-agnostic gate: compare Irys enumeration vs Solana memo enumeration
//! for a given server wallet.
//!
//! Usage:
//!   # Full parity check (requires Solana RPC):
//!   WALLET=<base58-pubkey> SOLANA_RPC_URL=<url> cargo run --example enumerate
//!
//!   # Irys-only (no Solana RPC needed):
//!   WALLET=<base58-pubkey> IRYS_ONLY=1 cargo run --example enumerate
//!
//! Exit codes:
//!   0 — Irys is a superset of memos (gate PASSES)
//!   1 — Irys is missing items that memos have (gate FAILS)
//!   2 — Irys-only mode (no Solana comparison performed)
//!   3 — error or inconclusive (no historical memo sample)

use mnemonic_core::{
    arweave::graphql::{solana_pubkey_to_arweave_address, GatewayFlavour, GraphQlClient},
    solana::SolanaClient,
};
use std::collections::HashSet;

const DEFAULT_IRYS_GRAPHQL: &str = "https://uploader.irys.xyz/graphql";
const DEFAULT_SOLANA_RPC: &str = "https://api.mainnet-beta.solana.com";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wallet = std::env::var("WALLET").unwrap_or_else(|_| {
        eprintln!("ERROR: WALLET env var required (base58 Solana pubkey of the server wallet)");
        std::process::exit(3);
    });

    let irys_url =
        std::env::var("IRYS_GRAPHQL_URL").unwrap_or_else(|_| DEFAULT_IRYS_GRAPHQL.to_string());
    let irys_only = std::env::var("IRYS_ONLY").is_ok();

    // Validate the pubkey decodes as valid base58.
    solana_pubkey_to_arweave_address(&wallet).unwrap_or_else(|e| {
        eprintln!("ERROR: invalid WALLET: {e}");
        std::process::exit(3);
    });

    println!("Server wallet  : {wallet}");
    println!("Irys GraphQL   : {irys_url}");
    println!();
    println!("Note: Irys indexes by raw base58 pubkey (not SHA256-derived Arweave address).");

    // ── Enumerate via Irys ────────────────────────────────────────────────
    // Pass the raw base58 pubkey — Irys uses that as its owner index.
    println!("Enumerating via Irys...");
    let irys_client = GraphQlClient::new_with_flavour(&irys_url, GatewayFlavour::Irys);
    let irys_items = irys_client
        .list_anchored(&[wallet.clone()])
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR fetching from Irys: {e}");
            std::process::exit(3);
        });

    let irys_txs: HashSet<String> = irys_items.iter().map(|i| i.arweave_tx.clone()).collect();
    println!("  Irys items     : {}", irys_txs.len());

    if irys_only {
        println!();
        println!("IRYS-ONLY mode — listing first 20 item IDs:");
        for tx in irys_items.iter().take(20) {
            println!("  {} (t={})", tx.arweave_tx, tx.block_time.unwrap_or(0));
        }
        println!();
        println!("Run without IRYS_ONLY=1 and with SOLANA_RPC_URL to do the full parity check.");
        std::process::exit(2);
    }

    // ── Enumerate via Solana memos ─────────────────────────────────────────
    let rpc_url =
        std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| DEFAULT_SOLANA_RPC.to_string());
    println!("Enumerating via Solana memos ({rpc_url})...");

    let solana_client = SolanaClient::new(&rpc_url);
    let memo_anchors = solana_client
        .list_memo_anchors(&wallet)
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR fetching from Solana: {e}");
            eprintln!("Tip: set SOLANA_RPC_URL to a private RPC to avoid rate limits.");
            std::process::exit(3);
        });

    let memo_txs: HashSet<String> = memo_anchors.iter().map(|m| m.arweave_tx.clone()).collect();
    println!("  Memo anchors   : {}", memo_txs.len());
    println!();

    // ── Parity check ──────────────────────────────────────────────────────
    let missing: Vec<&String> = memo_txs.difference(&irys_txs).collect();
    let irys_only_items: Vec<&String> = irys_txs.difference(&memo_txs).collect();

    println!("Results:");
    println!("  In memos, not in Irys  : {}", missing.len());
    println!("  In Irys, not in memos  : {}", irys_only_items.len());
    println!(
        "  In both                : {}",
        memo_txs.intersection(&irys_txs).count()
    );
    println!();

    if memo_txs.is_empty() {
        println!("GATE: INCONCLUSIVE — no historical memo anchors were found for this wallet.");
        println!("An empty comparison cannot establish replacement-index parity.");
        std::process::exit(3);
    } else if missing.is_empty() {
        println!("GATE: PASS ✓ — Irys enumeration is a superset of memo enumeration.");
        println!("Stage 2 (stop writing memos) is safe to proceed.");
        std::process::exit(0);
    } else {
        println!(
            "GATE: FAIL ✗ — Irys is missing {} items that memos have:",
            missing.len()
        );
        for tx in missing.iter().take(20) {
            println!("  MISSING: {tx}");
        }
        if missing.len() > 20 {
            println!("  ... and {} more", missing.len() - 20);
        }
        println!();
        println!("Do NOT proceed to stage 2 until this resolves.");
        std::process::exit(1);
    }
}
