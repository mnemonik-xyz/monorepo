//! MCP protocol handler — JSON-RPC 2.0 dispatcher for both stdio and HTTP.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_sdk::signature::Keypair;

use std::sync::Arc;
use crate::{arweave::ArweaveClient, compress::EmbeddingCompressor, db::AttestationStore, embed::Embedder, pricing::PricingEngine, solana::SolanaClient, tools};

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// Shared state for the MCP server.
/// AttestationStore uses rusqlite (not Sync), so we wrap in std::sync::Mutex
/// and never hold the lock across await points.
pub struct McpState {
    pub keypair: Keypair,
    pub solana: SolanaClient,
    pub arweave: ArweaveClient,
    pub store: std::sync::Mutex<AttestationStore>,
    pub embedder: Box<dyn Embedder>,
    pub compressor: EmbeddingCompressor,

    // Payment config
    /// "none" | "balance" | "x402" | "both"
    pub payment_mode: String,
    pub treasury_pubkey: String,
    pub usdc_mint: String,
    pub sign_memory_cost_micro_usdc: i64,

    // Dynamic pricing
    pub pricing: Arc<PricingEngine>,
    /// Solana memo tx fee in lamports (passed to CostHint).
    pub sol_tx_fee_lamports: u64,

    // Storage mode
    /// "local" (default, free, SQLite only) or "full" (Arweave + Solana + SQLite)
    pub storage_mode: String,
}

// Safety: We only access store through std::sync::Mutex (short critical sections, no await)
// Keypair is just bytes, SolanaClient/ArweaveClient are reqwest::Client (Send+Sync)
unsafe impl Send for McpState {}
unsafe impl Sync for McpState {}

fn tool_definitions() -> Value {
    serde_json::json!([
        {
            "name": "mnemonic_whoami",
            "description": "Returns this agent's cryptographic identity: Solana public key, did:sol, did:key, attestation count",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "mnemonic_sign_memory",
            "description": "Creates a verifiable memory attestation: embeds content, SHA-256 hash, stores on Arweave, anchors on Solana via SPL Memo",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "Content to attest"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Optional tags"},
                },
                "required": ["content"],
            },
        },
        {
            "name": "mnemonic_verify",
            "description": "Verifies a memory attestation by recomputing hash and comparing against on-chain record",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "solana_tx": {"type": "string", "description": "Solana TX signature"},
                    "arweave_tx": {"type": "string", "description": "Arweave TX ID"},
                },
            },
        },
        {
            "name": "mnemonic_prove_identity",
            "description": "Signs a challenge with Ed25519 key, proving identity without on-chain transaction",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "challenge": {"type": "string", "description": "Challenge to sign"},
                },
                "required": ["challenge"],
            },
        },
        {
            "name": "mnemonic_recall",
            "description": "Searches attested memory history using semantic similarity",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "limit": {"type": "integer", "description": "Max results", "default": 5},
                },
                "required": ["query"],
            },
        },
    ])
}

pub async fn handle_request(req: &JsonRpcRequest, state: &McpState) -> JsonRpcResponse {
    let result = match req.method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mnemonic", "version": "0.1.0"},
        })),
        "tools/list" => Ok(serde_json::json!({"tools": tool_definitions()})),
        "tools/call" => {
            let name = req.params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or_default();
            handle_tool_call(name, &args, state).await
        }
        "notifications/initialized" | "ping" => Ok(serde_json::json!({})),
        _ => Err(format!("unknown method: {}", req.method)),
    };

    match result {
        Ok(val) => JsonRpcResponse {
            jsonrpc: "2.0".into(), id: req.id.clone(), result: Some(val), error: None,
        },
        Err(msg) => JsonRpcResponse {
            jsonrpc: "2.0".into(), id: req.id.clone(), result: None,
            error: Some(JsonRpcError { code: -32603, message: msg }),
        },
    }
}

async fn handle_tool_call(name: &str, args: &Value, state: &McpState) -> Result<Value, String> {
    let result = match name {
        "mnemonic_whoami" => {
            // DB-only: lock, query, release before returning
            let store = state.store.lock().unwrap();
            tools::whoami(&state.keypair, &store, &state.storage_mode)
        }
        "mnemonic_sign_memory" => {
            let content = args["content"].as_str().ok_or("content required")?.to_string();
            let tags: Vec<String> = args.get("tags")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            let cost_hint = state.pricing.cost_hint(state.sol_tx_fee_lamports);
            tools::sign_memory(
                &state.keypair, &state.solana, &state.arweave, &state.store,
                state.embedder.as_ref(), &state.compressor,
                &content, &tags, &cost_hint, &state.storage_mode,
            ).await.map_err(|e| e.to_string())?
        }
        "mnemonic_verify" => {
            let sol = args.get("solana_tx").and_then(|v| v.as_str());
            let ar = args.get("arweave_tx").and_then(|v| v.as_str());
            tools::verify(&state.solana, &state.arweave, &state.store, sol, ar, &state.storage_mode)
                .await.map_err(|e| e.to_string())?
        }
        "mnemonic_prove_identity" => {
            // Pure crypto, no DB or network
            tools::prove_identity(&state.keypair, args["challenge"].as_str().ok_or("challenge required")?)
        }
        "mnemonic_recall" => {
            let query = args["query"].as_str().ok_or("query required")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            // DB-only: lock, query, release
            let store = state.store.lock().unwrap();
            tools::recall(&state.keypair, &store, state.embedder.as_ref(), query, limit)
        }
        _ => return Err(format!("unknown tool: {name}")),
    };

    Ok(serde_json::json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default()}]
    }))
}
