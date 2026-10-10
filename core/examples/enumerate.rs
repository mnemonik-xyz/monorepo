//! Chain-agnostic gate: compare Arweave gateway enumeration vs Solana memo
//! enumeration for one server wallet.
//!
//! Usage:
//!   WALLET=<base58-pubkey> SOLANA_RPC_URL=<rpc> cargo run --example enumerate
//!   # Gateway-only (no Solana RPC needed):
//!   WALLET=<base58-pubkey> GATEWAY_ONLY=1 cargo run --example enumerate
//!
//! Exit codes:
//!   0 — the gateway index is a superset of memos (gate PASSES)
//!   1 — the gateway index is missing items that memos have (gate FAILS)
//!   2 — gateway-only mode (no Solana comparison performed)
//!   3 — error or inconclusive (no historical memo sample)

use mnemonic_core::{
    arweave::graphql::{solana_pubkey_to_arweave_address, GraphQlClient},
    solana::SolanaClient,
};
use std::collections::HashSet;

const DEFAULT_GRAPHQL: &str = "https://arweave.net/graphql";
const DEFAULT_SOLANA_RPC: &str = "https://api.mainnet-beta.solana.com";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wallet = std::env::var("WALLET").unwrap_or_else(|_| {
        eprintln!("ERROR: WALLET env var required (base58 Solana pubkey of the server wallet)");
        std::process::exit(3);
    });

    let graphql_url = std::env::var("GRAPHQL_URL").unwrap_or_else(|_| DEFAULT_GRAPHQL.to_string());
    let gateway_only = std::env::var("GATEWAY_ONLY").is_ok();

    // Gateways index ANS-104 items by the SHA-256-derived Arweave address.
    let address = solana_pubkey_to_arweave_address(&wallet).unwrap_or_else(|e| {
        eprintln!("ERROR: invalid WALLET: {e}");
        std::process::exit(3);
    });

    println!("Server wallet  : {wallet}");
    println!("Arweave address: {address}");
    println!("Gateway GraphQL: {graphql_url}");
    println!();

    // ── Enumerate via the Arweave gateway index ───────────────────────────
    println!("Enumerating via the gateway index...");
    let gateway_client = GraphQlClient::new(&graphql_url);
    let gateway_items = gateway_client
        .list_anchored(std::slice::from_ref(&address))
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR fetching from the gateway index: {e}");
            std::process::exit(3);
        });

    let gateway_txs: HashSet<String> = gateway_items.iter().map(|i| i.arweave_tx.clone()).collect();
    println!("  Gateway items  : {}", gateway_txs.len());

    if gateway_only {
        println!();
        println!("GATEWAY-ONLY mode — listing first 20 item IDs:");
        for tx in gateway_items.iter().take(20) {
            println!("  {} (t={})", tx.arweave_tx, tx.block_time.unwrap_or(0));
        }
        println!();
        println!("Run without GATEWAY_ONLY=1 and with SOLANA_RPC_URL to do the full parity check.");
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
    let missing: Vec<&String> = memo_txs.difference(&gateway_txs).collect();
    let gateway_only_items: Vec<&String> = gateway_txs.difference(&memo_txs).collect();

    println!("Results:");
    println!("  In memos, not in gateway : {}", missing.len());
    println!("  In gateway, not in memos : {}", gateway_only_items.len());
    println!(
        "  In both                : {}",
        memo_txs.intersection(&gateway_txs).count()
    );
    println!();

    if memo_txs.is_empty() {
        println!("GATE: INCONCLUSIVE — no historical memo anchors were found for this wallet.");
        println!("An empty comparison cannot establish replacement-index parity.");
        std::process::exit(3);
    } else if missing.is_empty() {
        println!("GATE: PASS ✓ — gateway enumeration is a superset of memo enumeration.");
        println!("Stage 2 (stop writing memos) is safe to proceed.");
        std::process::exit(0);
    } else {
        println!(
            "GATE: FAIL ✗ — the gateway index is missing {} items that memos have:",
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
