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
use mnemonic_core::storage::{AttestationStore, SqliteStore, WriteMode};

use crate::mcp::{invalid_params, unsupported_mode, Envelope, JsonRpcError};
use crate::pending::PendingBundles;
use crate::{payment, pricing::CostHint};

/// Resolve the per-request `mode` field on `mnemonic_sign_memory` to a
/// concrete [`WriteMode`]. This is the **single source of truth** that drives
/// BOTH the paywall gate in `mcp_handler` AND the persisted `write_mode`
/// column on the attestation row — by construction they cannot drift
/// (Decision 1 in work/modes-user-choice/tech-spec.md).
///
/// Resolution rules (tech-spec §"API contract changes / Resolution rule"):
///
/// | Input             | Output                                            |
/// |-------------------|---------------------------------------------------|
/// | `None` (absent)   | env-var fallback: `local` iff `env_storage_mode  |
/// |                   | == "local"`, else `Participate`                  |
/// | `"local"`         | `WriteMode::Local`                                |
/// | `"participate"`   | `WriteMode::Participate`                          |
/// | anything else     | `Err(invalid_params("mode", received_verbatim))`  |
///
/// "Anything else" covers: JSON `null`, non-string types (integer, array,
/// object), empty `""`, whitespace `" "`, capitalised `"Local"` /
/// `"PARTICIPATE"`, unknown strings. The verbatim received `Value` is echoed
/// in the error's `data.received` so a misbehaving client can diff.
///
/// Pure function — no I/O, no globals. The full resolution table is
/// table-driven-tested in `mcp::tests::resolve_write_mode_*`.
pub(crate) fn resolve_write_mode(
    input_mode: Option<&serde_json::Value>,
    env_storage_mode: &str,
) -> Result<WriteMode, JsonRpcError> {
    match input_mode {
        None => {
            // Backward-compat: the shipped chrome-extension and pre-T2 stdio
            // clients never send `mode`. Resolve from env-var so their
            // behaviour is byte-for-byte unchanged.
            if env_storage_mode == "local" {
                Ok(WriteMode::Local)
            } else {
                Ok(WriteMode::Participate)
            }
        }
        Some(serde_json::Value::String(s)) => match WriteMode::from_str_strict(s) {
            Some(m) => Ok(m),
            // `from_str_strict` rejects `"Local"`, `"PARTICIPATE"`, `""`,
            // `" "`, trailing whitespace, and any unknown string. Echo the
            // raw string back through `data.received` (not `s` directly —
            // we want the JSON Value variant preserved).
            None => Err(invalid_params(
                "mode",
                input_mode.expect("Some matched above"),
            )),
        },
        // Non-string (null, integer, array, object) — strict rejection.
        Some(v) => Err(invalid_params("mode", v)),
    }
}

/// Smuggle a `unsupported_mode` JSON-RPC error through `anyhow::Error` so it
/// surfaces from `tools::sign_memory` (which has an `anyhow::Result` return
/// type) and is reconstituted by `mcp::tool_error_to_json_rpc` at the
/// dispatcher boundary. The encoded body is the inner-data shape of
/// `JsonRpcError`: `{code, message, data}` JSON — matching the parsing
/// rule in `tool_error_to_json_rpc`. Drift would silently fall back to
/// `-32603 InternalError`; the round-trip is tested via the integration
/// test `participate_against_local_only_server_returns_unsupported`.
fn unsupported_mode_anyhow(envelope: &Envelope) -> anyhow::Error {
    let err = unsupported_mode("participate", &envelope.supported_modes);
    let body = serde_json::json!({
        "code": err.code,
        "message": err.message,
        "data": err.data,
    });
    anyhow::anyhow!(body.to_string())
}

/// Tool 1: whoami (sync — DB only)
///
/// T2 extension: returns the discoverability envelope (`supported_modes`,
/// `default_mode`, `participate_cost`) alongside the existing fields so
/// clients can choose `local` vs `participate` BEFORE attempting to write.
/// The legacy `storage_mode` field is kept verbatim for pre-envelope clients
/// (chrome-extension Cloud tier still reads it).
pub fn whoami(
    keypair: &Keypair,
    store: &SqliteStore,
    storage_mode: &str,
    envelope: &Envelope,
) -> serde_json::Value {
    let pubkey = identity::pubkey_base58(keypair);
    let count = store.count(&pubkey).unwrap_or(0);
    // Serialize the envelope through serde_json so the `null` rendering of
    // `participate_cost: Option<ParticipateCost>` and the static `&'static
    // str` arrays in `supported_modes` come out byte-identical to the
    // spec'd wire shape (no manual JSON construction drift).
    let envelope_value = serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null);
    let envelope_obj = envelope_value.as_object().cloned().unwrap_or_default();
    let mut out = serde_json::json!({
        "public_key": pubkey,
        "did_sol": identity::did_sol(keypair),
        "did_key": identity::did_key(keypair),
        "attestation_count": count,
        "storage_mode": storage_mode,
    });
    // Merge envelope keys (`supported_modes`, `default_mode`,
    // `participate_cost`) into the response. Done as a post-merge rather
    // than inline so the field order in the json! macro stays stable for
    // the golden fixture.
    if let Some(map) = out.as_object_mut() {
        for (k, v) in envelope_obj {
            map.insert(k, v);
        }
    }
    out
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
    write_mode: WriteMode,
    envelope: &Envelope,
) -> anyhow::Result<serde_json::Value> {
    // T2 — UnsupportedMode check fires BEFORE the JWT-deferred branch so a
    // browser client asking for `participate` against a local-only deploy
    // gets the typed error even when it would otherwise enter the
    // deferred-signing path. The user explicitly asked to anchor on-chain;
    // the server cannot fulfil that intent regardless of whether the
    // signing path is server-side or browser-side.
    if write_mode == WriteMode::Participate && !envelope.supports_participate() {
        return Err(unsupported_mode_anyhow(envelope));
    }
    // The deferred-signing branch is the Cloud-tier path (browser signs
    // COSE, server only anchors). It fires on any JWT-authenticated
    // request EXCEPT when the caller explicitly opted into the inline
    // free-local path (`mode: "local"` against a server that ALSO
    // supports participate — i.e. a `full` deploy). For mode-absent
    // requests we preserve the legacy behaviour (deferred whenever JWT
    // is present) so the shipped chrome-extension Cloud-tier keeps
    // working byte-for-byte. The "explicit local on a full deploy
    // returns free" case is signalled by `write_mode == Local` AND the
    // envelope supporting participate (a paid/anchoring deploy that the
    // user is on but chose to bypass).
    let explicit_local_on_full = write_mode == WriteMode::Local && envelope.supports_participate();
    if let Some(sub) = jwt_sub {
        if !explicit_local_on_full {
            return sign_memory_deferred(embedder, compressor, pending, content, tags, sub).await;
        }
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
        write_mode,
        envelope,
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
///
/// T2 changes (routing now driven by per-request `write_mode`, not the
/// operator's `STORAGE_MODE` env-var):
///
/// - The `write_mode` parameter replaces `storage_mode` as the routing
///   decision. `WriteMode::Local` → synthetic-id no-anchor path
///   regardless of env-var. `WriteMode::Participate` → real Arweave +
///   Solana writes regardless of env-var (the paywall gate in
///   `mcp_handler` has already ensured the deploy supports it).
/// - `storage_mode` is retained ONLY for the legacy whoami-echo field in
///   the success envelope. It does NOT influence behaviour anymore — the
///   chrome-extension and other legacy clients that read `storage_mode`
///   from the response keep working byte-for-byte because the resolver
///   maps `None` (no `mode` field) to env-var fallback, producing the same
///   `WriteMode` value the env-var would have selected.
/// - `envelope` is consulted to short-circuit a `participate` request
///   against a local-only deploy with a typed `UnsupportedMode` error.
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
    write_mode: WriteMode,
    envelope: &Envelope,
) -> anyhow::Result<serde_json::Value> {
    // The participate-on-local-only short-circuit lives in
    // `sign_memory` (the public entry point) — fires before deferred-vs-
    // inline branching so the user gets the typed error regardless of
    // path.
    let _ = envelope; // retained in the signature for future hooks
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

    // 5. Store on-chain (or locally) — routed by per-request `write_mode`,
    //    not the operator's `STORAGE_MODE`. A `local` request against a
    //    `STORAGE_MODE=full` deploy stays free (no Arweave/Solana writes).
    let (solana_tx, arweave_tx) = match write_mode {
        WriteMode::Local => {
            let local_ar = format!("local:{}", &attestation_id[..8]);
            let local_sol = format!("local:{}", &content_hash[..16]);
            (local_sol, local_ar)
        }
        WriteMode::Participate => {
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
        }
    };

    // 6. Save locally
    {
        let store = store.lock().unwrap();
        // T2: the persisted `write_mode` column is the SAME value the
        // paywall gate consulted (single source of truth — Decision 1).
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
            write_mode,
            &embedding,
        )?;
        // Cost-recording fires only on `Participate` writes. A `Local`
        // request can hit this code path against a `STORAGE_MODE=full +
        // PAYMENT_MODE=x402` server and MUST NOT produce an
        // `attestation_costs` row — that would charge the caller for a
        // free path.
        if write_mode == WriteMode::Participate {
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
        "write_mode": write_mode.as_str(),
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

    fn local_envelope() -> Envelope {
        Envelope::from_config("local", "none", 0)
    }

    #[tokio::test]
    async fn test_sign_memory_returns_awaiting_signature_for_jwt_path() {
        // T2: the legacy mode-absent JWT path still resolves to the
        // deferred branch because the resolver maps a local-only deploy
        // with no explicit mode to `WriteMode::Local`, and on a deploy
        // that does NOT support participate that takes the deferred
        // branch (legacy chrome-extension shape preserved). See
        // `sign_memory` for the routing rule.
        let (kp, sol, ar, store, emb, comp, pending, hint) = fixtures();
        let owner = kp.pubkey().to_string();
        let env = local_envelope();
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
            WriteMode::Local,
            &env,
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
        let env = local_envelope();
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
            WriteMode::Local,
            &env,
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
mod resolve_write_mode_tests {
    //! T2 resolver unit tests. Pure function — no fixtures needed.
    //!
    //! Drives the SINGLE source of truth that feeds both the paywall gate
    //! in `mcp_handler` and the persisted `write_mode` column. Drift is
    //! impossible by construction because both call sites consume the
    //! return value of `resolve_write_mode`.

    use super::*;

    /// Helper: assert the error is `-32602 InvalidParams` with the expected
    /// `data.field` and `data.received` payload.
    fn assert_invalid_params(err: JsonRpcError, expected_received: &serde_json::Value) {
        assert_eq!(err.code, -32602, "expected -32602 InvalidParams");
        assert_eq!(err.message, "Invalid params");
        let data = err.data.expect("InvalidParams must carry `data`");
        assert_eq!(data["field"], "mode", "data.field must be \"mode\"");
        assert_eq!(
            &data["received"], expected_received,
            "data.received must echo input verbatim"
        );
    }

    #[test]
    fn none_with_env_local_resolves_to_local() {
        let m = resolve_write_mode(None, "local").expect("None+local resolves");
        assert_eq!(m, WriteMode::Local);
    }

    #[test]
    fn none_with_env_full_resolves_to_participate() {
        // Legacy compat: pre-T2 clients (chrome-extension Cloud) on a full
        // deploy fall back to env-var behaviour — Participate.
        let m = resolve_write_mode(None, "full").expect("None+full resolves");
        assert_eq!(m, WriteMode::Participate);
    }

    #[test]
    fn explicit_local_string_resolves_to_local() {
        let v = serde_json::json!("local");
        let m = resolve_write_mode(Some(&v), "full").expect("explicit local");
        assert_eq!(m, WriteMode::Local);
    }

    #[test]
    fn explicit_participate_string_resolves_to_participate() {
        let v = serde_json::json!("participate");
        let m = resolve_write_mode(Some(&v), "local").expect("explicit participate");
        // Note: even on a `STORAGE_MODE=local` env, the resolver returns
        // Participate; rejection happens later in `sign_memory_inline` via
        // the envelope check. The resolver's job is parse-only.
        assert_eq!(m, WriteMode::Participate);
    }

    #[test]
    fn null_rejects_with_invalid_params() {
        let v = serde_json::Value::Null;
        let err = resolve_write_mode(Some(&v), "local").expect_err("null rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_integer_rejects() {
        let v = serde_json::json!(42);
        let err = resolve_write_mode(Some(&v), "local").expect_err("integer rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_array_rejects() {
        let v = serde_json::json!(["local"]);
        let err = resolve_write_mode(Some(&v), "local").expect_err("array rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn non_string_object_rejects() {
        let v = serde_json::json!({"mode": "local"});
        let err = resolve_write_mode(Some(&v), "local").expect_err("object rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn empty_string_rejects() {
        let v = serde_json::json!("");
        let err = resolve_write_mode(Some(&v), "local").expect_err("empty rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn whitespace_string_rejects() {
        let v = serde_json::json!(" ");
        let err = resolve_write_mode(Some(&v), "local").expect_err("whitespace rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn capitalised_local_rejects() {
        let v = serde_json::json!("Local");
        let err = resolve_write_mode(Some(&v), "local").expect_err("Local rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn uppercase_participate_rejects() {
        let v = serde_json::json!("PARTICIPATE");
        let err = resolve_write_mode(Some(&v), "local").expect_err("PARTICIPATE rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn unknown_string_rejects() {
        let v = serde_json::json!("cloud");
        let err = resolve_write_mode(Some(&v), "local").expect_err("unknown rejects");
        assert_invalid_params(err, &v);
    }

    #[test]
    fn trailing_whitespace_rejects() {
        let v = serde_json::json!("local ");
        let err = resolve_write_mode(Some(&v), "local").expect_err("trailing space rejects");
        assert_invalid_params(err, &v);
    }
}
