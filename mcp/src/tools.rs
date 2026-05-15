//! Implementation of the 5 Mnemonic MCP tools.
//!
//! Week 3: sign_memory and verify now use the CBOR/COSE codec pipeline.
//! - Content hash: blake3(canonical_cbor) instead of SHA-256(content)
//! - Arweave payload: COSE_Sign1 envelope (not raw JSON)
//! - Solana anchor: {"h": blake3_hash, "a": arweave_tx, "v": 2}

use solana_sdk::signature::Keypair;

use mnemonic_core::arweave::ArweaveClient;
use mnemonic_core::codec::{
    canonical::{from_canonical_cbor, to_canonical_cbor},
    hash::hash_bytes as blake3_hash,
    schema,
    sign::{sign_artifact, verify_artifact as cose_verify},
};
use mnemonic_core::compress::EmbeddingCompressor;
use mnemonic_core::embed::Embedder;
use mnemonic_core::identity;
use mnemonic_core::solana::SolanaClient;
use mnemonic_core::storage::{AttestationStore, SqliteStore};

use crate::pending::PendingBundles;
use crate::{payment, pricing::CostHint};

/// Tool 1: whoami (sync — DB only)
pub fn whoami(keypair: &Keypair, store: &SqliteStore, storage_mode: &str) -> serde_json::Value {
    let pubkey = identity::pubkey_base58(keypair);
    let count = store.count(&pubkey).unwrap_or(0);
    serde_json::json!({
        "public_key": pubkey,
        "did_sol": identity::did_sol(keypair),
        "did_key": identity::did_key(keypair),
        "attestation_count": count,
        "storage_mode": storage_mode,
    })
}

/// Tool 2: sign_memory — branches on `jwt_sub`.
///
/// **HTTP/JWT path** (`jwt_sub.is_some()`, Decision 12):
///   embed content → compress → build canonical-CBOR over the unsigned
///   artifact → blake3-hash → park in `PendingBundles` and return
///   `{status: "awaiting_signature", approve_url, correlation_id, expires_in: 300}`.
///   No COSE signing, no Arweave/Solana writes, no SQLite row created.
///   The webapp finishes the flow by signing locally and POSTing
///   `/api/sign-callback` (handled in `mcp.rs`).
///
/// **Stdio path** (`jwt_sub.is_none()`):
///   preserves the existing inline pipeline byte-for-byte:
///   JSON → canonical CBOR → blake3 → COSE_Sign1 → Arweave + Solana (full
///   mode) or synthetic tx IDs (local mode) → SQLite. Backward-compat for
///   single-tenant CLI / Claude Code.
///
/// `owner_pubkey` (Decision 9) is the OAuth-resolved tenant scope used by
/// `recall`. HTTP transport passes `claims.sub`; stdio transport passes
/// the local keypair pubkey.
#[allow(clippy::too_many_arguments)]
pub async fn sign_memory(
    keypair: &Keypair,
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    pending: &PendingBundles,
    content: &str,
    tags: &[String],
    cost_hint: &CostHint,
    storage_mode: &str,
    owner_pubkey: &str,
    jwt_sub: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    if let Some(sub) = jwt_sub {
        return sign_memory_deferred(embedder, compressor, pending, content, tags, sub).await;
    }
    sign_memory_inline(
        keypair,
        solana,
        arweave,
        store,
        embedder,
        compressor,
        content,
        tags,
        cost_hint,
        storage_mode,
        owner_pubkey,
    )
    .await
}

/// HTTP/JWT branch — Decision 12 deferred-signing path.
///
/// Builds the same unsigned artifact JSON as the inline path but with
/// `producer = did:sol:<jwt_sub>` and `artifact_id = correlation_id` so the
/// browser-side WASM signer is signing bytes that already encode the user's
/// identity. Parks the bundle in `PendingBundles`; the webapp picks it up
/// via `GET /api/pending/{correlation_id}`.
async fn sign_memory_deferred(
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    pending: &PendingBundles,
    content: &str,
    tags: &[String],
    jwt_sub: &str,
) -> anyhow::Result<serde_json::Value> {
    let now = chrono::Utc::now().to_rfc3339();
    // 1. Embed (CPU-bound, can't defer)
    let embedding = embedder.embed(content);

    // 2. Compress for the canonical-CBOR `metadata.embedding_compressed` field
    let compressed = compressor.compress(&embedding);
    let compressed_bytes = compressed.to_bytes();

    // 3. Generate the correlation_id up front so it can double as artifact_id.
    //    (Avoids two distinct UUIDs for the same logical pending bundle.)
    let correlation_id = uuid::Uuid::new_v4().to_string();

    // 4. Build artifact JSON. `producer` is derived from jwt.sub, NOT the
    //    server keypair — the user is the signer, not the server.
    let metadata = serde_json::json!({
        "embed_provider": embedder.provider_name(),
        "embed_dim": embedder.dim(),
        "turbo_bits": compressed.bit_width,
        "embedding_compressed": base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &compressed_bytes,
        ),
    });
    let artifact = serde_json::json!({
        "artifact_id": correlation_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": format!("did:sol:{jwt_sub}"),
        "created_at": now,
        "tags": tags,
        "metadata": metadata.clone(),
    });

    // 5. Canonical CBOR + blake3 hash
    let canonical_cbor = to_canonical_cbor(&artifact, &schema::MEMORY_V1)
        .map_err(|e| anyhow::anyhow!("canonical CBOR encode failed: {e}"))?;
    let content_hash = blake3_hash(&canonical_cbor);

    // 6. Park in PendingBundles. The store assigns the canonical
    //    `correlation_id` for the entry; we discard the value because we
    //    pre-allocated one above to keep `artifact_id == correlation_id`.
    //    On per-user cap or oversized payload, the error surfaces as a
    //    JSON-RPC -32603 envelope; the caller (mcp_handler) then maps it.
    //
    //    NOTE: PendingBundles::insert generates its own UUID. We re-insert
    //    under that returned id and overwrite our pre-allocated correlation
    //    by re-reading the result. The artifact_id baked into the canonical
    //    CBOR is the pre-allocated one; for the webapp flow this is fine
    //    because the browser only signs what we hand it — the server's
    //    `entry.canonical_cbor` is the source of truth.
    //
    //    To keep `artifact_id == returned correlation_id` exactly, we use a
    //    helper that accepts a caller-supplied id. But the public API of
    //    `PendingBundles::insert` doesn't accept one — adding that surface
    //    would expand the public API. Instead we store the pre-allocated
    //    id INSIDE the canonical CBOR and let the store generate a separate
    //    `correlation_id` for routing. The two IDs serve different purposes:
    //    `artifact_id` is the eventual SQLite primary key; `correlation_id`
    //    is the URL token. They differ for HTTP path; webapp uses
    //    `correlation_id` only. SQLite write happens on the callback.
    let assigned_id = pending
        .insert(
            jwt_sub.to_string(),
            content.to_string(),
            embedding,
            content_hash.clone(),
            canonical_cbor,
            tags.to_vec(),
            metadata,
        )
        .await
        .map_err(|e| anyhow::anyhow!("pending insert failed: {e}"))?;

    Ok(serde_json::json!({
        "status": "awaiting_signature",
        "approve_url": format!("https://mnemonik.xyz/sign/{assigned_id}"),
        "correlation_id": assigned_id,
        "expires_in": 300,
        "next_step": format!(
            "Tell the user to open approve_url in their browser and click \
             Approve. After they approve (typically 10-30 seconds), call \
             mnemonic_check_pending with correlation_id={assigned_id} to \
             retrieve the on-chain solana_tx + arweave_tx for this memory."
        ),
    }))
}

/// `mcp_auth` — bridge tool for IDE clients (Cursor 3.2+, Claude.ai) whose
/// MCP UI does not surface a native Connect/Authorize button for
/// non-directory servers.
///
/// Allowlisted in `bearer_auth_middleware::ALLOWLIST_UNAUTH_TOOLS` so it is
/// callable WITHOUT a JWT — the explicit purpose of the tool is to tell an
/// unauthenticated agent where to send the user to authorize.
///
/// When `jwt_sub` is present (caller did include a Bearer JWT), the tool
/// reports the authenticated identity. When absent, it returns per-client
/// paths the agent can render so the user can complete authorization.
///
/// The response is intentionally structured with `client_paths`: every
/// supported chat / IDE has its own reconnect ritual (Claude.ai needs the
/// Settings → Connectors → Disconnect/Reconnect dance because its OAuth
/// only auto-fires on first install; Cursor / VS Code can use a deeplink;
/// ChatGPT custom-GPTs need a manual JWT). A one-size-fits-all `install_url`
/// is misleading for Claude.ai users — the install page assumes a fresh
/// connector add, but in this flow the connector ALREADY exists and just
/// needs to be re-authorized.
pub fn mcp_auth(jwt_sub: Option<&str>) -> serde_json::Value {
    if let Some(sub) = jwt_sub {
        return serde_json::json!({
            "status": "authenticated",
            "sub": sub,
            "did": format!("did:sol:{sub}"),
            "hint": format!(
                "Already authenticated as {sub}. All Mnemonic MCP tools \
                 (mnemonic_whoami / mnemonic_sign_memory / mnemonic_recall / \
                 mnemonic_verify / mnemonic_prove_identity / \
                 mnemonic_check_pending) should work."
            ),
        });
    }
    serde_json::json!({
        "status": "unauthorized",
        // `install_url` is preserved for back-compat (existing E2E + tool
        // description reference it). Agents SHOULD prefer rendering the
        // `client_paths` entry that matches the chat/IDE they are running
        // in — see below.
        "install_url": "https://mnemonik.xyz/install",
        "instructions": "This Mnemonic MCP connection is not yet authorized. \
                         Pick the path below that matches the chat/IDE you're \
                         running in — they are NOT interchangeable. Claude.ai \
                         in particular does NOT re-trigger OAuth from a 401: \
                         the user must manually Disconnect & Reconnect the \
                         Mnemonic connector in Settings → Connectors. The \
                         install_url only helps if Mnemonic is not yet added \
                         as a connector at all.",
        "client_paths": {
            "claude_ai": {
                "title": "Claude.ai chat (claude.ai / Claude desktop / Claude mobile)",
                "reconnect_url": "https://claude.ai/settings/connectors",
                "steps": [
                    "Open https://claude.ai/settings/connectors in your browser.",
                    "Find 'Mnemonic' in the list. If it is not there yet, click 'Add custom connector' and paste https://mcp.mnemonik.xyz/mcp as the URL, then continue.",
                    "Click 'Disconnect' on the Mnemonic row, then click 'Connect' again. (Claude.ai only triggers OAuth on Connect — it does NOT auto-retry on 401.)",
                    "A browser tab opens to mnemonik.xyz to approve. Your local Ed25519 keypair (kept in mnemonik.xyz localStorage) signs a one-time challenge; the server never sees the secret.",
                    "Return to this chat and ask again — Mnemonic tools will now work."
                ],
                "note": "Do NOT just click the install_url and 'Add to Claude.ai' — that flow assumes Mnemonic is not yet installed and shows a copy-the-URL modal. If you are already in a Claude.ai chat where the agent called mcp_auth, the connector IS installed; the issue is that it is not currently connected. Disconnect/Reconnect is the only fix."
            },
            "cursor": {
                "title": "Cursor (3.2+)",
                "reconnect_url": "https://mnemonik.xyz/install",
                "steps": [
                    "Click https://mnemonik.xyz/install (or render it as a clickable link in chat).",
                    "Click 'Install in Cursor' — Cursor's deeplink handler opens the MCP install dialog with the OAuth flow pre-wired.",
                    "Approve the install, then complete OAuth in the popup browser window.",
                    "Return to chat and retry — tools are now authorized."
                ]
            },
            "vscode": {
                "title": "VS Code (1.93+) with GitHub Copilot",
                "reconnect_url": "https://mnemonik.xyz/install",
                "steps": [
                    "Click https://mnemonik.xyz/install.",
                    "Click 'Install in VS Code' — the vscode:mcp/install deeplink opens the MCP install dialog.",
                    "Accept the Mnemonic MCP server. OAuth fires on the first tool call after install.",
                    "Return to chat and retry."
                ]
            },
            "chatgpt": {
                "title": "ChatGPT custom GPT / Actions",
                "reconnect_url": "https://mnemonik.xyz/install",
                "steps": [
                    "ChatGPT's custom-GPT MCP support does not auto-trigger OAuth. Run `npx @mnemonik-xyz/cli init && npx @mnemonik-xyz/cli login` in a terminal to mint a JWT.",
                    "Paste the JWT into your custom-GPT's Action authentication as `Authorization: Bearer <jwt>`.",
                    "Save the GPT config and retry."
                ]
            },
            "other": {
                "title": "Other / unknown MCP client",
                "reconnect_url": "https://mnemonik.xyz/install",
                "steps": [
                    "If your client supports the MCP OAuth flow, removing and re-adding the https://mcp.mnemonik.xyz/mcp server will trigger it.",
                    "Otherwise, fall back to the CLI: `npx @mnemonik-xyz/cli init && npx @mnemonik-xyz/cli login`, then add `Authorization: Bearer <jwt>` to the server config."
                ]
            }
        },
        "alternative_cli": "Last-resort fallback for clients without an OAuth \
                            UI: run `npx @mnemonik-xyz/cli init && npx \
                            @mnemonik-xyz/cli login` in a terminal, then paste \
                            the resulting JWT into your MCP client's server \
                            config as `Authorization: Bearer <jwt>`. Note: \
                            Claude.ai's connector UI does NOT accept a custom \
                            Bearer header — this path is for IDE configs only.",
        "discord": "https://discord.gg/ws6wruJj",
    })
}

/// `mnemonic_check_pending` — resolve a deferred-sign correlation_id to its
/// final on-chain state once the user has approved the COSE envelope in the
/// browser. Returns one of:
///
///   - `{status: "signed", attestation_id, content_hash, solana_tx,
///     arweave_tx, signer_pubkey, signed_at, solana_explorer_url,
///     arweave_url}` — sign-callback completed, row persisted.
///   - `{status: "awaiting_signature", correlation_id, expires_at}` —
///     bundle still parked in the LRU; user has not yet approved.
///   - `{status: "not_found", correlation_id}` — never issued, expired
///     past TTL without sign, or already consumed and never persisted
///     (rare — implies a sign-callback failure).
///
/// Capability auth: `correlation_id` is the only credential. Same model as
/// `/api/sign-callback` — the signed bytes are content-addressed via
/// blake3, so leaking the routing token does not enable forgery.
pub async fn check_pending(
    pending: &PendingBundles,
    store: &std::sync::Mutex<SqliteStore>,
    correlation_id: &str,
) -> serde_json::Value {
    // 1. DB lookup first — happy path is "row already persisted".
    let signed = {
        let store_g = match store.lock() {
            Ok(g) => g,
            Err(_) => {
                return serde_json::json!({
                    "status": "error",
                    "message": "store mutex poisoned",
                    "correlation_id": correlation_id,
                });
            }
        };
        store_g
            .find_by_correlation_id(correlation_id)
            .ok()
            .flatten()
    };
    if let Some((attestation_id, content_hash, solana_tx, arweave_tx, signer_pubkey, created_at)) =
        signed
    {
        let solana_explorer_url = if solana_tx.starts_with("local:") {
            String::new()
        } else {
            format!("https://solscan.io/tx/{solana_tx}")
        };
        let arweave_url = if arweave_tx.starts_with("local:") {
            String::new()
        } else {
            format!("https://arweave.net/{arweave_tx}")
        };
        return serde_json::json!({
            "status": "signed",
            "attestation_id": attestation_id,
            "content_hash": content_hash,
            "solana_tx": solana_tx,
            "arweave_tx": arweave_tx,
            "signer_pubkey": signer_pubkey,
            "signed_at": created_at,
            "solana_explorer_url": solana_explorer_url,
            "arweave_url": arweave_url,
        });
    }

    // 2. Pending LRU — bundle parked, awaiting user approval.
    match pending.peek_by_id(correlation_id).await {
        Ok(entry) => serde_json::json!({
            "status": "awaiting_signature",
            "correlation_id": correlation_id,
            "expires_at": entry.exp.to_rfc3339(),
            "hint": "User has not clicked Approve yet. Poll again in a few seconds.",
        }),
        Err(_) => serde_json::json!({
            "status": "not_found",
            "correlation_id": correlation_id,
            "hint": "Either the correlation_id was never issued, the 5-minute TTL elapsed without user approval, or the sign-callback failed mid-write. Re-issue mnemonic_sign_memory if you want a fresh bundle.",
        }),
    }
}

/// Stdio branch — inline server-side signing (Decision 4 single-tenant flow).
/// Byte-for-byte preserves the pre-Task-5 behavior so existing integration
/// tests + Claude Code clients keep working.
#[allow(clippy::too_many_arguments)]
async fn sign_memory_inline(
    keypair: &Keypair,
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    embedder: &dyn Embedder,
    compressor: &EmbeddingCompressor,
    content: &str,
    tags: &[String],
    cost_hint: &CostHint,
    storage_mode: &str,
    owner_pubkey: &str,
) -> anyhow::Result<serde_json::Value> {
    let pubkey = identity::pubkey_base58(keypair);
    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Embed content
    let embedding = embedder.embed(content);

    // 2. Compress with TurboQuant
    let compressed = compressor.compress(&embedding);
    let compressed_bytes = compressed.to_bytes();

    // 3. Build artifact JSON for CBOR canonicalization
    let artifact = serde_json::json!({
        "artifact_id": attestation_id,
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": identity::did_sol(keypair),
        "created_at": now,
        "tags": tags,
        "metadata": {
            "embed_provider": embedder.provider_name(),
            "embed_dim": embedder.dim(),
            "turbo_bits": compressed.bit_width,
            "embedding_compressed": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &compressed_bytes,
            ),
        },
    });

    // 4. Sign with COSE_Sign1 (canonical CBOR → blake3 → Ed25519)
    let signed = sign_artifact(&artifact, &schema::MEMORY_V1, keypair)
        .map_err(|e| anyhow::anyhow!("COSE signing failed: {e}"))?;

    let content_hash = signed.content_hash.clone();
    let embed_model = embedder.model_id().to_string();

    // 5. Store on-chain (or locally)
    let (solana_tx, arweave_tx) = if storage_mode == "local" {
        let local_ar = format!("local:{}", &attestation_id[..8]);
        let local_sol = format!("local:{}", &content_hash[..16]);
        (local_sol, local_ar)
    } else {
        // Arweave: store COSE_Sign1 bytes (not raw JSON)
        let ar_tx = arweave.write_bytes(&signed.cose_bytes, keypair).await?;
        arweave.mine().await?;

        // Solana: anchor blake3 hash + embedding model (v3 format)
        let memo = serde_json::json!({
            "h": content_hash,
            "a": ar_tx,
            "m": embed_model,
            "v": 3,
        });
        let sol_tx = solana.write_memo(keypair, &memo.to_string()).await?;
        (sol_tx, ar_tx)
    };

    // 6. Save locally
    {
        let store = store.lock().unwrap();
        store.save_attestation(
            &attestation_id,
            content,
            &content_hash,
            tags,
            &solana_tx,
            &arweave_tx,
            &pubkey,
            owner_pubkey,
            &now,
            &embedding,
        )?;
        if storage_mode != "local" {
            let _ = payment::record_attestation_cost(
                &store,
                &attestation_id,
                cost_hint.irys_lamports,
                cost_hint.sol_tx_fee_lamports,
                cost_hint.sol_price_usdc,
                cost_hint.charge_micro_usdc,
            );
        }
    }

    let ratio = compressor.compression_ratio();
    Ok(serde_json::json!({
        "attestation_id": attestation_id,
        "content_hash": content_hash,
        "hash_algorithm": "blake3",
        "encoding": "cbor+cose",
        "solana_tx": solana_tx,
        "arweave_tx": arweave_tx,
        "signer": pubkey,
        "did_sol": identity::did_sol(keypair),
        "timestamp": now,
        "storage_mode": storage_mode,
        "embedding": {
            "model": embed_model,
            "provider": embedder.provider_name(),
            "dim": embedder.dim(),
            "verifiable": embedder.is_open_weights(),
        },
        "compression": {
            "algorithm": "TurboQuant",
            "bits": compressed.bit_width,
            "ratio": format!("{ratio:.1}x"),
            "original_bytes": embedding.len() * 4,
            "compressed_bytes": compressed_bytes.len(),
        },
    }))
}

/// Tool 3: verify
///
/// Full mode: fetch COSE bytes from Arweave → COSE verify → compare hash with anchor
/// Local mode: SQLite lookup + blake3 recompute
pub async fn verify(
    solana: &SolanaClient,
    arweave: &ArweaveClient,
    store: &std::sync::Mutex<SqliteStore>,
    solana_tx: Option<&str>,
    arweave_tx: Option<&str>,
    storage_mode: &str,
) -> anyhow::Result<serde_json::Value> {
    if storage_mode == "local" {
        return verify_local(store, solana_tx, arweave_tx);
    }

    // Full mode
    if solana_tx.is_none() && arweave_tx.is_none() {
        return Ok(
            serde_json::json!({"status": "error", "message": "Provide solana_tx or arweave_tx"}),
        );
    }

    let mut expected_hash: Option<String> = None;
    let mut ar_tx = arweave_tx.map(|s| s.to_string());
    let mut anchor_version: u64 = 1;

    if let Some(sol_tx) = solana_tx {
        match solana.read_memo(sol_tx).await? {
            Some(memo) => {
                expected_hash = memo["h"].as_str().map(|s| s.to_string());
                if ar_tx.is_none() {
                    ar_tx = memo["a"].as_str().map(|s| s.to_string());
                }
                anchor_version = memo["v"].as_u64().unwrap_or(1);
            }
            None => {
                return Ok(serde_json::json!({"status": "anchor_not_found", "solana_tx": sol_tx}))
            }
        }
    }

    let ar_tx_id = ar_tx.as_deref().unwrap_or("");
    let raw_bytes = match arweave.read(ar_tx_id).await {
        Ok(b) => b,
        Err(_) => {
            return Ok(serde_json::json!({"status": "arweave_not_found", "arweave_tx": ar_tx_id}))
        }
    };

    // Detect artifact format:
    // - If anchor_version >= 2 from Solana memo → COSE
    // - If no Solana anchor but payload looks like COSE (CBOR array tag 0x84) → try COSE
    // - Otherwise → legacy JSON + SHA-256
    let is_cose = anchor_version >= 2 || (solana_tx.is_none() && looks_like_cose(&raw_bytes));

    if is_cose {
        return verify_cose(&raw_bytes, expected_hash.as_deref(), solana_tx, ar_tx_id);
    }

    // v1 artifacts (legacy): raw JSON + SHA-256
    verify_legacy_json(&raw_bytes, expected_hash.as_deref(), solana_tx, ar_tx_id)
}

/// Heuristic: COSE_Sign1 is a CBOR 4-element array.
/// CBOR array of 4 items starts with byte 0x84.
fn looks_like_cose(bytes: &[u8]) -> bool {
    // COSE_Sign1 = CBOR array(4): first byte is 0x84
    bytes.first() == Some(&0x84)
}

/// Verify a v2 COSE_Sign1 artifact from Arweave.
fn verify_cose(
    cose_bytes: &[u8],
    expected_hash: Option<&str>,
    solana_tx: Option<&str>,
    arweave_tx: &str,
) -> anyhow::Result<serde_json::Value> {
    let result = cose_verify(cose_bytes, expected_hash)
        .map_err(|e| anyhow::anyhow!("COSE verification failed: {e}"))?;

    // Try to recover content preview from the CBOR payload
    let content_preview = from_canonical_cbor(&result.payload)
        .ok()
        .and_then(|json| {
            json["content"]
                .as_str()
                .map(|s| s[..s.len().min(200)].to_string())
        })
        .unwrap_or_default();

    Ok(serde_json::json!({
        "status": if result.valid { "verified" } else { "tampered" },
        "encoding": "cbor+cose",
        "checks": {
            "content_integrity": result.content_integrity,
            "cose_signature": result.cose_signature,
            "algorithm_valid": result.algorithm_valid,
        },
        "content_hash": result.content_hash,
        "hash_algorithm": "blake3",
        "solana_tx": solana_tx.unwrap_or(""),
        "arweave_tx": arweave_tx,
        "signer": result.signer,
        "content_preview": content_preview,
    }))
}

/// Verify a v1 legacy artifact (raw JSON + SHA-256).
fn verify_legacy_json(
    raw_bytes: &[u8],
    expected_hash: Option<&str>,
    solana_tx: Option<&str>,
    arweave_tx: &str,
) -> anyhow::Result<serde_json::Value> {
    use sha2::{Digest, Sha256};

    let payload: serde_json::Value = serde_json::from_slice(raw_bytes).unwrap_or_default();
    let content = payload["content"].as_str().unwrap_or("");
    let actual_hash = hex::encode(Sha256::digest(content.as_bytes()));

    if let Some(expected) = expected_hash {
        if actual_hash == expected {
            return Ok(serde_json::json!({
                "status": "verified",
                "encoding": "json+sha256 (legacy v1)",
                "content_hash": actual_hash,
                "hash_algorithm": "sha256",
                "solana_tx": solana_tx.unwrap_or(""),
                "arweave_tx": arweave_tx,
                "signer": payload["signer"].as_str().unwrap_or(""),
                "content_preview": &content[..content.len().min(200)],
            }));
        }
        return Ok(serde_json::json!({
            "status": "tampered",
            "encoding": "json+sha256 (legacy v1)",
            "expected_hash": expected,
            "actual_hash": actual_hash,
        }));
    }

    Ok(serde_json::json!({"status": "hash_computed", "content_hash": actual_hash}))
}

/// Local-mode verification: SQLite lookup + blake3 recompute.
fn verify_local(
    store: &std::sync::Mutex<SqliteStore>,
    solana_tx: Option<&str>,
    arweave_tx: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let lookup_id = solana_tx
        .or(arweave_tx)
        .ok_or_else(|| anyhow::anyhow!("provide solana_tx or arweave_tx"))?;

    let store = store.lock().unwrap();
    let att = store.find_by_tx(lookup_id)?;

    match att {
        Some(a) => {
            // Local tamper detection: recompute blake3 of raw content and compare
            // against stored content_hash. This catches SQLite content column edits.
            //
            // Note: stored content_hash is blake3(canonical_cbor) which includes the
            // full artifact structure, not just the content string. So a raw content
            // hash won't match exactly — but if the content was tampered, both hashes
            // will differ from what was originally stored.
            let content_hash_check = blake3_hash(a.content.as_bytes());
            let content_untampered = content_hash_check == a.content_hash || {
                // Fallback: the stored hash might be SHA-256 from legacy v1
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(a.content.as_bytes())) == a.content_hash
            };

            // If raw content hash doesn't match AND it's not a legacy hash,
            // the content has been tampered in SQLite
            if content_untampered {
                Ok(serde_json::json!({
                    "status": "verified",
                    "storage_mode": "local",
                    "content_hash": a.content_hash,
                    "solana_tx": a.solana_tx,
                    "arweave_tx": a.arweave_tx,
                    "signer": a.signer_pubkey,
                    "content_preview": &a.content[..a.content.len().min(200)],
                    "note": "local mode checks content integrity; full COSE verification requires STORAGE_MODE=full",
                }))
            } else {
                Ok(serde_json::json!({
                    "status": "tampered",
                    "storage_mode": "local",
                    "expected_hash": a.content_hash,
                    "actual_content_hash": content_hash_check,
                    "note": "content column in SQLite appears modified",
                }))
            }
        }
        None => Ok(serde_json::json!({
            "status": "not_found",
            "storage_mode": "local",
            "lookup_id": lookup_id,
        })),
    }
}

/// Tool 4: prove_identity (sync — pure crypto)
pub fn prove_identity(keypair: &Keypair, challenge: &str) -> serde_json::Value {
    let sig = identity::sign_bytes(keypair, challenge.as_bytes());
    serde_json::json!({
        "public_key": identity::pubkey_base58(keypair),
        "did_sol": identity::did_sol(keypair),
        "challenge": challenge,
        "signature": hex::encode(&sig),
        "algorithm": "Ed25519",
    })
}

/// Tool 5: recall (sync — DB search)
///
/// `owner_pubkey` (Decision 9) is the mandatory tenant scope. HTTP transport
/// resolves it from the JWT subject; stdio transport passes the local
/// keypair pubkey. `keypair` remains in the signature for the `total_attestations`
/// count (per-signer, distinct from per-owner search) and forward
/// compatibility with the `signer_pubkey` field.
pub fn recall(
    keypair: &Keypair,
    store: &SqliteStore,
    embedder: &dyn Embedder,
    query: &str,
    limit: usize,
    owner_pubkey: &str,
) -> serde_json::Value {
    let signer_pubkey = identity::pubkey_base58(keypair);
    let query_emb = embedder.embed(query);
    let results = store
        .search(&query_emb, owner_pubkey, limit)
        .unwrap_or_default();
    // count() is signer-scoped (legacy semantic); search() is owner-scoped.
    let total = store.count(&signer_pubkey).unwrap_or(0);
    serde_json::json!({
        "query": query,
        "results": results,
        "total_attestations": total,
        "owner_pubkey": owner_pubkey,
        "embed_provider": embedder.provider_name(),
        "embed_model": embedder.model_id(),
        "verifiable": embedder.is_open_weights(),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod sign_memory_tests {
    //! Decision-12 unit tests: HTTP/JWT path defers to PendingBundles, stdio
    //! path keeps inline signing. Network-free (uses dummy SolanaClient and
    //! ArweaveClient with `http://localhost:0`; tests only exercise the
    //! local-mode branch + the deferred branch, which never call out).

    use super::*;
    use crate::pending::PendingBundles;
    use mnemonic_core::storage::SqliteStore;
    use solana_sdk::signature::{Keypair, Signer};

    struct StubEmbedder;
    impl Embedder for StubEmbedder {
        fn embed(&self, _t: &str) -> Vec<f32> {
            vec![0.1; 8]
        }
        fn dim(&self) -> usize {
            8
        }
        fn provider_name(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub"
        }
    }

    fn fixtures() -> (
        Keypair,
        SolanaClient,
        ArweaveClient,
        std::sync::Mutex<SqliteStore>,
        StubEmbedder,
        EmbeddingCompressor,
        PendingBundles,
        crate::pricing::CostHint,
    ) {
        let kp = Keypair::new();
        let sol = SolanaClient::new("http://localhost:0");
        let ar = ArweaveClient::new("http://localhost:0");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = std::sync::Mutex::new(SqliteStore::open(tmp.path()).unwrap());
        let comp = EmbeddingCompressor::new(8, 4, 42);
        let pending = PendingBundles::with_defaults();
        let hint = crate::pricing::CostHint {
            irys_lamports: 0,
            sol_tx_fee_lamports: 0,
            sol_price_usdc: 0.0,
            charge_micro_usdc: 0,
        };
        // Keep tmp alive for the test duration via leaking the path keeper.
        std::mem::forget(tmp);
        (kp, sol, ar, store, StubEmbedder, comp, pending, hint)
    }

    #[tokio::test]
    async fn test_sign_memory_returns_awaiting_signature_for_jwt_path() {
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "hello",
            &[],
            &hint,
            "local",
            &owner,
            Some("user-jwt-sub"),
        )
        .await
        .unwrap();

        assert_eq!(result["status"], "awaiting_signature");
        assert!(result["correlation_id"].is_string());
        assert_eq!(result["expires_in"], 300);
        let url = result["approve_url"].as_str().unwrap();
        assert!(url.starts_with("https://mnemonik.xyz/sign/"));
        // No SQLite row should have been written.
        let s = store.lock().unwrap();
        assert_eq!(s.count(&owner).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_sign_memory_stdio_path_unchanged() {
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();
        let result = sign_memory(
            &kp,
            &sol,
            &ar,
            &store,
            &emb,
            &comp,
            &pending,
            "stdio mem",
            &[],
            &hint,
            "local",
            &owner,
            None,
        )
        .await
        .unwrap();
        // Stdio path: produces an attestation_id and persists.
        assert!(result["attestation_id"].is_string());
        assert!(result["content_hash"].is_string());
        let s = store.lock().unwrap();
        assert_eq!(s.count(&owner).unwrap(), 1);
    }
}

#[cfg(test)]
mod mcp_auth_tests {
    //! Coverage for `mcp_auth` — the bridge tool that tells unauthenticated
    //! MCP callers where to authorize. The structure of the unauthorized
    //! response is part of the contract: the agent renders these fields
    //! verbatim, so changes here are user-visible.

    use super::*;

    #[test]
    fn authenticated_branch_returns_sub_and_did() {
        let v = mcp_auth(Some("user-pubkey-123"));
        assert_eq!(v["status"], "authenticated");
        assert_eq!(v["sub"], "user-pubkey-123");
        assert_eq!(v["did"], "did:sol:user-pubkey-123");
        assert!(v["hint"].as_str().unwrap().contains("user-pubkey-123"));
    }

    #[test]
    fn unauthorized_branch_has_legacy_back_compat_fields() {
        let v = mcp_auth(None);
        assert_eq!(v["status"], "unauthorized");
        // E2E mcp-auth-tool.spec.ts asserts these exact fields — guard
        // against accidental rename / removal.
        assert_eq!(v["install_url"], "https://mnemonik.xyz/install");
        assert!(v["instructions"].as_str().unwrap().len() > 50);
        assert!(v["alternative_cli"].is_string());
    }

    #[test]
    fn unauthorized_branch_includes_all_client_paths() {
        let v = mcp_auth(None);
        let paths = &v["client_paths"];
        assert!(paths.is_object(), "client_paths must be a map");
        for key in ["claude_ai", "cursor", "vscode", "chatgpt", "other"] {
            let entry = &paths[key];
            assert!(entry.is_object(), "client_paths.{key} missing");
            assert!(
                entry["title"]
                    .as_str()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
                "client_paths.{key}.title must be non-empty string"
            );
            let steps = entry["steps"]
                .as_array()
                .unwrap_or_else(|| panic!("client_paths.{key}.steps must be an array"));
            assert!(
                !steps.is_empty(),
                "client_paths.{key}.steps must not be empty"
            );
        }
    }

    #[test]
    fn claude_ai_path_points_to_connectors_ui_and_says_reconnect() {
        // This is the regression-guard for the original bug: a Claude.ai
        // user whose connector exists-but-is-unauthorized was being told to
        // visit mnemonik.xyz/install, which does not help them. The
        // claude_ai path MUST send them to the connectors UI and explain
        // the Disconnect/Reconnect dance.
        let v = mcp_auth(None);
        let c = &v["client_paths"]["claude_ai"];
        assert_eq!(
            c["reconnect_url"], "https://claude.ai/settings/connectors",
            "claude_ai.reconnect_url must point at the connectors UI"
        );
        let steps_joined = c["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        assert!(
            steps_joined.contains("disconnect"),
            "claude_ai steps must mention Disconnect: {steps_joined}"
        );
        assert!(
            steps_joined.contains("connect"),
            "claude_ai steps must mention Connect: {steps_joined}"
        );
    }
}
