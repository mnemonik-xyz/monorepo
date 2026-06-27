//! `/api/pending/*` and `/api/sign-callback` routes — Decision 12 webapp surface.
//!
//! These two HTTP endpoints close the browser-mediated signing loop:
//!
//! - **`GET /api/pending/{correlation_id}`** — webapp fetches the unsigned
//!   canonical-CBOR bytes the user must sign (Content-Type: application/cbor).
//!   Auth required; `jwt.sub` must match the entry's stored `jwt_sub` (403
//!   otherwise). Returns 404 if absent, 410 if expired or already consumed.
//!
//! - **`POST /api/sign-callback`** — webapp delivers the COSE_Sign1 bytes
//!   produced by the WASM signer. Body schema:
//!   `{"correlation_id": String, "cose_signed_bytes": base64, "signer_pubkey": base58}`.
//!   Validation order:
//!   1. lookup pending (410 if missing/expired/consumed)
//!   2. assert `signer_pubkey == jwt.sub` (403 otherwise)
//!   3. verify COSE_Sign1 against `entry.content_hash` (401 otherwise)
//!   4. atomically `consume` (single-use; replay → 410)
//!   5. persist via `AttestationStore::save_attestation(... owner_pubkey = jwt.sub)`
//!   6. return `{"status": "ok", "attestation_id": <uuid>}`
//!
//! Auth model: both endpoints sit behind `oauth::bearer_auth_middleware`;
//! `Claims` is read from request extensions.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use crypto_box::{
    aead::{Aead, AeadCore, OsRng},
    PublicKey as X25519PublicKey, SalsaBox, SecretKey as X25519SecretKey,
};
use lru::LruCache;
use mnemonic_core::codec::{hash::hash_bytes, sign::verify_artifact};
use mnemonic_core::storage::{
    AttestationStore, BlogPost, PublicArtifact, SearchResult, Visibility, WriteMode,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::mcp::McpState;
use crate::oauth::Claims;
use crate::pending::PendingError;
use crate::publish::{PublishError, PublishInput};

/// `GET /api/pending/{correlation_id}` — webapp fetches the unsigned
/// canonical-CBOR bytes for the user to sign.
///
/// Auth: capability-based — `correlation_id` IS the capability. No Bearer
/// JWT required because the webapp on `mnemonik.xyz` does not hold the JWT
/// that the AI-tool client received from `/oauth/token`. The unsigned CBOR
/// leak is bounded (5-min TTL, content the user just typed); the actual
/// signing chain is gated by `consume_by_id` + COSE verification in
/// `sign_callback_handler`, where keypair ownership is the auth.
pub async fn get_pending_handler(
    State(state): State<Arc<McpState>>,
    Path(correlation_id): Path<String>,
) -> Response {
    let entry = match state.pending.peek_by_id(&correlation_id).await {
        Ok(e) => e,
        Err(e) => return e.into_response(),
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/cbor"),
    );
    // Surface the expected hash + correlation id so the webapp can sanity-
    // check before signing without a second round-trip. These are not
    // load-bearing (the COSE_Sign1 envelope already carries the hash); they
    // are convenience headers.
    if let Ok(hv) = HeaderValue::from_str(&entry.content_hash) {
        headers.insert("x-mnemonic-content-hash", hv);
    }
    if let Ok(hv) = HeaderValue::from_str(&correlation_id) {
        headers.insert("x-mnemonic-correlation-id", hv);
    }

    let mut resp = Response::new(Body::from(entry.canonical_cbor));
    *resp.status_mut() = StatusCode::OK;
    *resp.headers_mut() = headers;
    resp
}

/// Body of `POST /api/sign-callback`.
#[derive(Debug, Deserialize)]
pub struct SignCallbackRequest {
    pub correlation_id: String,
    /// base64 (standard alphabet, with padding) of the COSE_Sign1 envelope.
    pub cose_signed_bytes: String,
    /// base58 user pubkey — must equal `jwt.sub` and the COSE kid.
    pub signer_pubkey: String,
}

/// Successful response of `POST /api/sign-callback`.
///
/// Includes the on-chain anchor identifiers so the webapp's success page can
/// render Solscan / Arweave links without an extra round-trip. In `local`
/// storage mode both fields carry synthetic `local:<...>` prefixes; in
/// `full` mode they are real Solana SPL Memo signatures + Arweave tx ids.
#[derive(Debug, Serialize)]
pub struct SignCallbackResponse {
    pub status: &'static str,
    pub attestation_id: String,
    pub content_hash: String,
    pub solana_tx: String,
    pub arweave_tx: String,
    /// Convenience explorer URL — `https://solscan.io/tx/{solana_tx}` for real
    /// txs; empty string for synthetic `local:` ids.
    pub solana_explorer_url: String,
    /// Convenience gateway URL — `https://arweave.net/{arweave_tx}` for real
    /// uploads; empty string for synthetic `local:` ids.
    pub arweave_url: String,
}

/// `POST /api/sign-callback` — webapp delivers the user's COSE_Sign1.
///
/// Auth: capability + cryptographic chain — no Bearer JWT required. The
/// body's `signer_pubkey` MUST equal the pending entry's stored `jwt_sub`
/// (validated atomically by `consume_by_id`), AND the COSE signature MUST
/// verify against that same pubkey. An attacker holding only a guessed
/// `correlation_id` cannot forge this without the user's private key.
pub async fn sign_callback_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<SignCallbackRequest>,
) -> Response {
    // 2. Decode the COSE bytes.
    let cose_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.cose_signed_bytes.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("cose_signed_bytes is not valid base64: {e}"),
            );
        }
    };

    // 3. Atomic consume — pop the entry from the LRU under a single lock.
    //    A concurrent second callback for the same correlation_id observes
    //    `NotFound` after this point. We do NOT verify the COSE before
    //    popping; otherwise two concurrent valid callbacks would both
    //    proceed to step 5 (persistence), inserting two SQLite rows for the
    //    same logical attestation.
    let entry = match state
        .pending
        .consume_by_id(&req.correlation_id, &req.signer_pubkey)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            // PendingError::NotFound → 404. But per Decision 12 a missing
            // entry on the callback is "already consumed or expired", which
            // is semantically 410 Gone. Override here.
            if matches!(e, PendingError::NotFound) {
                return error_resp(
                    StatusCode::GONE,
                    "pending bundle missing or already consumed",
                );
            }
            return e.into_response();
        }
    };

    // 4. Verify COSE against the stored content hash. If verification fails
    //    the entry is already gone from the LRU; the user's bundle is
    //    effectively forfeit. This is the right tradeoff: tampered or
    //    replayed signatures should not allow retries.
    let result = match verify_artifact(&cose_bytes, Some(&entry.content_hash)) {
        Ok(r) => r,
        Err(e) => {
            return error_resp(
                StatusCode::UNAUTHORIZED,
                &format!("COSE verification failed: {e}"),
            );
        }
    };
    if !result.valid
        || !result.cose_signature
        || !result.content_integrity
        || !result.algorithm_valid
    {
        return error_resp(StatusCode::UNAUTHORIZED, "COSE signature invalid");
    }
    // The COSE kid (recovered as `result.signer`) must equal the body's
    // `signer_pubkey`, which `consume_by_id` already validated against the
    // pending entry's stored `jwt_sub`. This closes the chain:
    // "correlation_id capability → entry.jwt_sub → body.signer_pubkey → COSE kid → Ed25519 signature".
    if result.signer != req.signer_pubkey {
        return error_resp(
            StatusCode::UNAUTHORIZED,
            "COSE kid does not match signer_pubkey",
        );
    }
    // Defense in depth: independently recompute the hash of the recovered
    // COSE payload and compare with the stored entry hash. `verify_artifact`
    // already does this internally (via `expected_hash`), but a second
    // explicit check guarantees that even if `result.content_integrity` is
    // ever loosened we still catch a mismatch here.
    let recomputed = hash_bytes(&result.payload);
    if recomputed != entry.content_hash {
        return error_resp(
            StatusCode::UNAUTHORIZED,
            "stored content_hash differs from COSE payload hash",
        );
    }

    // 5. Persist + (optionally) anchor on-chain.
    //
    // Originally Decision 4 forced `local:` ids on the deferred path. That
    // capped the protocol's headline value behind the inline (stdio) path
    // only. We now branch on `state.storage_mode`:
    //
    //   - `local` — synthetic `local:` ids preserved (offline / dev / free
    //     tier). Trust chain still works for in-store recall + verify.
    //   - `full`  — real Arweave upload (server keypair signs the ANS-104
    //     bundle) + Solana SPL Memo (server keypair pays the tx fee).
    //     Memo data binds the user's blake3 content hash to the server's
    //     on-chain anchor — a third party fetching the memo can re-fetch
    //     the COSE bytes from Arweave and verify the user's COSE signature
    //     end-to-end without contacting Mnemonic.
    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Wave 3: a deferred write whose resolved mode is `Local` (a remote user's
    // free local write, now client-signed too) skips on-chain anchoring and
    // gets synthetic ids — exactly like a `local` storage deploy. Either
    // condition routes to the synthetic-id branch.
    let is_local_write = state.storage_mode == "local" || entry.write_mode == WriteMode::Local;
    let (solana_tx, arweave_tx) = if is_local_write {
        let local_ar = format!("local:{}", &attestation_id[..8]);
        let local_sol = format!("local:{}", &entry.content_hash[..16]);
        (local_sol, local_ar)
    } else {
        // Arweave upload of the COSE_Sign1 envelope bytes.
        let ar_tx = match state.arweave.write_bytes(&cose_bytes, &state.keypair).await {
            Ok(t) => t,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("arweave upload failed: {e}"),
                );
            }
        };
        // No-op for production Irys (mine() only writes against arlocal).
        let _ = state.arweave.mine().await;

        // Solana SPL Memo anchor — `v=2` schema (h=hash, a=arweave_tx) so
        // existing verifiers continue to parse without an alg field. The
        // inline path emits v=3 with embed_model; the deferred path's
        // `entry` carries metadata in its CBOR but not as a flat string,
        // so v=2 is the conservative choice to avoid embed-model drift.
        let memo = serde_json::json!({
            "h": entry.content_hash,
            "a": ar_tx,
            "v": 2,
        });
        let sol_tx = match state
            .solana
            .write_memo(&state.keypair, &memo.to_string())
            .await
        {
            Ok(t) => t,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("solana memo write failed: {e}"),
                );
            }
        };
        (sol_tx, ar_tx)
    };

    let persist_res = {
        // Short, await-free critical section.
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        // Wave 3 update: the pending bundle now carries the resolved
        // `write_mode` (added when Wave 3 routed remote-owned writes —
        // including explicit `mode: "local"` — through the client-signing
        // path). Persist with the caller's intended mode rather than
        // assuming `Participate`. A `Local` bundle took the synthetic-id
        // branch above and stays free; a `Participate` bundle was anchored
        // on Arweave + Solana. (Historically this path only ever saw
        // `Participate` because explicit-local fell through to the inline
        // operator-signed path; that custodial fall-through is now removed.)
        //
        // Round-2 (T3 extension): the row is persisted BEFORE the delivery
        // check. The delivery check's primary-key recall stage needs the row
        // to exist (see `tools::perform_delivery_check`). On delivery failure
        // the row is demoted in place via `INSERT OR REPLACE` inside
        // `confirm_delivery_or_demote`.
        // Visibility defaults to `Private` here — the deferred-sign callback
        // path is a Participate write (browser-mediated COSE_Sign1), and
        // until the JSON-input resolver lands (Task 5) every such write is
        // private-by-default (AC13). Public visibility will become an
        // explicit opt-in propagated from the original `sign_memory` call.
        let save_res = store.save_attestation(
            &attestation_id,
            &entry.content,
            &entry.content_hash,
            &entry.tags,
            &solana_tx,
            &arweave_tx,
            &req.signer_pubkey, // signer = pubkey we just verified via COSE
            &req.signer_pubkey, // owner = same pubkey (Decision 9 — webapp flow uses keypair as identity)
            &now,
            entry.write_mode,
            Visibility::Private,
            &entry.embedding,
        );
        // Stamp the correlation_id onto the row so `mnemonic_check_pending`
        // can resolve it later. Best-effort; an UPDATE failure here doesn't
        // invalidate the attestation itself.
        if save_res.is_ok() {
            let _ = store.set_correlation_id(&attestation_id, &req.correlation_id);
        }
        save_res
    };
    if let Err(e) = persist_res {
        // Persistence failed AFTER the LRU consumed the entry AND after any
        // on-chain anchor was written. The user's bundle is gone. Surface a
        // 500 — the failure mode here is operator-visible (DB I/O / disk
        // full) rather than user-fixable.
        return error_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("persist failed: {e}"),
        );
    }

    // T3 (round-2): extend the delivery guarantee to the deferred-signing
    // path. After the chain anchor + DB persist, run the same three-stage
    // check (refetch → verify_cose → primary-key recall) that
    // `sign_memory_inline` runs. On failure: row is demoted in place to
    // `WriteMode::Local` by the shared helper, and we surface a typed
    // error response so the webapp can show the user a "delivery not
    // confirmed" notice rather than a green checkmark.
    //
    // Skip the check for `local`-mode deploys AND for `Local` write-mode
    // bundles (Wave 3) — neither anchored anything on-chain, so there is no
    // real Arweave tx to re-fetch. Both took the synthetic-id branch above.
    // Refund-on-failure for the deferred path is out of scope: the webapp
    // owns its own credit accounting and the inline-path `mcp_handler`
    // doesn't see this code path. The demoted row + structured error
    // signal the webapp to NOT charge the user.
    if !is_local_write {
        let ctx = crate::tools::DeliveryContext {
            arweave: &state.arweave,
            store: &state.store,
            timeout: state.delivery_refetch_timeout,
            attestation_id: &attestation_id,
            content: &entry.content,
            content_hash: &entry.content_hash,
            tags: &entry.tags,
            solana_tx: &solana_tx,
            arweave_tx: &arweave_tx,
            signer_pubkey: &req.signer_pubkey,
            owner_pubkey: &req.signer_pubkey,
            created_at: &now,
            embedding: &entry.embedding,
        };
        match crate::tools::confirm_delivery_or_demote(ctx).await {
            Ok(crate::tools::DeliveryOutcome::Confirmed { .. }) => {
                // Happy path — fall through to the success envelope.
            }
            Ok(crate::tools::DeliveryOutcome::Demoted { stage }) => {
                state.delivery_metrics.record_not_confirmed(stage);
                tracing::warn!(
                    attestation_id = %attestation_id,
                    arweave_tx = %arweave_tx,
                    solana_tx = %solana_tx,
                    stage = %stage,
                    "deferred-path delivery not confirmed — row demoted to local"
                );
                // 200 OK with a typed-error body so the webapp's existing
                // JSON-handling does not break, but the body carries the
                // demotion signal. (HTTP 4xx would be wrong: the anchor
                // DID succeed and the row IS persisted — just demoted.)
                let body = serde_json::json!({
                    "status": "delivery_not_confirmed",
                    "kind": "DeliveryNotConfirmed",
                    "stage": stage,
                    "row_demoted_to": "local",
                    "attestation_id": attestation_id,
                    "arweave_tx": arweave_tx,
                    "solana_tx": solana_tx,
                });
                return (StatusCode::OK, Json(body)).into_response();
            }
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("delivery check internal error: {e}"),
                );
            }
        }
    }

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

    let body = SignCallbackResponse {
        status: "ok",
        attestation_id,
        content_hash: entry.content_hash,
        solana_tx,
        arweave_tx,
        solana_explorer_url,
        arweave_url,
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn error_resp(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"status": "error", "error": msg})),
    )
        .into_response()
}

// ── Bootstrap-ticket flow (mnemonic-cli tech-spec Decision 7) ───────────────
//
// Webapp issues a one-time ticket bound to a JWT subject; CLI redeems the
// ticket exactly once (or until TTL) to fetch the user's keypair so the CLI
// runs under the same identity the webapp already provisioned. Tickets are
// in-memory only — server restart drops every ticket.
//
// Caps:
//   - LRU 100 entries total. The 101st insert evicts the oldest entry.
//   - Per-`jwt_sub` cap 3. The 4th insert by the same user → 429.
//   - TTL 300 seconds (5 minutes). Redeems past TTL → 404 (treated identically
//     to "not found / already consumed" so a probing attacker cannot distinguish).
//
// Atomicity: `consume` removes-and-returns under a single tokio mutex guard.
// Two concurrent redeems of the same ticket race deterministically: exactly
// one returns Some, the other returns None.

/// Maximum total number of pending bootstrap tickets in the LRU.
pub const BOOTSTRAP_LRU_CAPACITY: usize = 100;
/// Maximum tickets per `jwt_sub` (4th insert returns 429).
pub const BOOTSTRAP_PER_USER_CAP: usize = 3;
/// Ticket TTL in seconds (5 minutes).
///
/// Matches tech-spec Decision 12 and the Deviation 2 trust model — the
/// plaintext-reachability window on the server is bounded to 5 minutes
/// so that a transient memory dump after compromise has a small target.
pub const BOOTSTRAP_TTL_SECS: i64 = 300;

/// Generate a short code in `XXXX-XXXX` format using an alphabet that
/// excludes visually confusable characters (0, 1, O, I). 40 bits of entropy,
/// well within the threat model for a 5-minute, single-use token.
fn generate_short_code() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut bytes = [0u8; 8];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut bytes);
    let chars: Vec<char> = bytes
        .iter()
        .map(|&b| ALPHABET[(b as usize) % ALPHABET.len()] as char)
        .collect();
    format!(
        "{}{}{}{}-{}{}{}{}",
        chars[0], chars[1], chars[2], chars[3], chars[4], chars[5], chars[6], chars[7]
    )
}

/// Which side issued the ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TicketOrigin {
    /// Webapp issued the ticket (original Decision-7 flow). The ticket
    /// carries a `keypair_json` blob; no x25519 wrapping is involved.
    Webapp,
    /// CLI issued the ticket (Task-12 symmetric flow). The ticket carries
    /// an x25519-wrapped secret blob (`wrapped_secret` + `eph_pub`).
    Cli,
}

/// One pending bootstrap ticket. Held in memory under the mutex until
/// `consume` removes it or the TTL expires.
#[derive(Debug, Clone)]
pub struct BootstrapTicket {
    /// UUIDv4 — both the LRU key and the redeem-URL capability.
    /// Kept on the struct for log continuity and future audit hooks; the
    /// LRU key is the same value, so the field appears redundant to the
    /// dead-code analyzer but is part of the public Decision-7 schema.
    #[allow(dead_code)]
    pub ticket_id: Uuid,

    /// Which direction this ticket flows.
    pub origin: TicketOrigin,

    // ── Webapp-origin fields (origin == Webapp) ──────────────────────────
    /// Solana CLI keypair JSON: a JSON array of 64 bytes (Ed25519 secret +
    /// public concatenation, the same format the Solana CLI writes to disk).
    /// Stored as-is (string) so the server never deserializes secret material
    /// — the CLI receives the raw JSON and parses on its end.
    /// Empty string for `Cli`-origin tickets.
    pub keypair_json: String,

    // ── CLI-origin fields (origin == Cli) ────────────────────────────────
    /// Issuer's x25519 public key bytes (32 bytes). Stored for the re-wrap
    /// step; not logged or persisted outside the process.
    /// Zero array for `Webapp`-origin tickets.
    pub eph_pub: [u8; 32],
    /// Encrypted secret blob (nonce[24] || ciphertext). Encrypted with the
    /// server's static x25519 key; re-encrypted for the redeemer on consume.
    /// Empty for `Webapp`-origin tickets.
    pub wrapped_secret: Vec<u8>,
    /// Ed25519 pubkey of the CLI issuer, base58-encoded. Relayed to the
    /// redeemer in the redeem response so the webapp can verify identity.
    /// Empty string for `Webapp`-origin tickets.
    pub issuer_pubkey_base58: String,

    /// Human-readable short code in `XXXX-XXXX` format. Used by CLI-origin
    /// tickets so the user can type it into the webapp. For Webapp-origin
    /// tickets this is the empty string (the UUID path is used instead).
    /// Accessed by `consume_by_short_code`; suppress dead-code lint on the
    /// field because that method is gated on `test-support` feature.
    #[allow(dead_code)]
    pub short_code: String,

    /// JWT subject (base58 user pubkey) of the webapp caller that issued the
    /// ticket. Recorded for the per-user cap accounting only — `consume` does
    /// not require an authenticated caller (the UUID is the capability).
    /// For CLI-origin tickets this is set to the `issuer_pubkey_base58` to
    /// reuse the same per-user accounting path.
    pub jwt_sub: String,
    /// Unix-seconds expiry. `consume` returns None when `now >= expires_at`.
    pub expires_at: i64,
}

/// Internal mutable state — the LRU and per-user counter mutate atomically
/// under a single tokio mutex guard. Lock discipline mirrors `PendingBundles`
/// in `pending.rs` (no `.await` while the guard is held).
struct BootstrapInner {
    lru: LruCache<Uuid, BootstrapTicket>,
    per_user: HashMap<String, usize>,
    per_user_cap: usize,
    ttl_seconds: i64,
}

impl BootstrapInner {
    fn dec_user(&mut self, jwt_sub: &str) {
        if let Some(c) = self.per_user.get_mut(jwt_sub) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.per_user.remove(jwt_sub);
            }
        }
    }
}

/// Errors surfaced by `BootstrapTickets::insert`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BootstrapInsertError {
    /// Per-user cap exceeded → HTTP 429 Too Many Requests.
    PerUserCapExceeded,
    /// Total LRU full AND the eviction would NOT be helpful — only happens
    /// when the LRU is set to a degenerate capacity (production caps allow
    /// graceful eviction). Maps to HTTP 503 Service Unavailable per the task
    /// spec, even though the production code path always evicts an older
    /// ticket instead. Kept as a documented variant so the handler can map
    /// it to 503 once a future LRU implementation surfaces this distinction.
    #[allow(dead_code)]
    LruExhausted,
}

/// In-memory LRU+TTL store of bootstrap tickets.
pub struct BootstrapTickets {
    inner: Mutex<BootstrapInner>,
}

impl BootstrapTickets {
    /// Build a store with explicit caps — `mock_state` and unit tests use
    /// smaller capacities than production.
    pub fn new(lru_capacity: usize, per_user_cap: usize, ttl_seconds: i64) -> Self {
        let cap = NonZeroUsize::new(lru_capacity.max(1)).expect("nonzero capacity");
        Self {
            inner: Mutex::new(BootstrapInner {
                lru: LruCache::new(cap),
                per_user: HashMap::new(),
                per_user_cap,
                ttl_seconds,
            }),
        }
    }

    /// Build with production defaults: 100 LRU, 3 per-user, 300s (5min) TTL.
    pub fn with_defaults() -> Self {
        Self::new(
            BOOTSTRAP_LRU_CAPACITY,
            BOOTSTRAP_PER_USER_CAP,
            BOOTSTRAP_TTL_SECS,
        )
    }

    /// Insert a webapp-origin ticket. Returns the UUID the webapp shows to
    /// the user (and the CLI then submits to /redeem). Atomicity: per-user
    /// counter and LRU mutate under a single guard. Generated UUIDv4 is
    /// collision-safe for the LRU's lifetime.
    pub async fn insert(
        &self,
        jwt_sub: String,
        keypair_json: String,
    ) -> Result<Uuid, BootstrapInsertError> {
        let entry = BootstrapTicket {
            ticket_id: Uuid::new_v4(),
            origin: TicketOrigin::Webapp,
            keypair_json,
            eph_pub: [0u8; 32],
            wrapped_secret: Vec::new(),
            issuer_pubkey_base58: String::new(),
            short_code: String::new(),
            jwt_sub: jwt_sub.clone(),
            expires_at: 0, // filled in below
        };
        self.insert_inner(jwt_sub, entry).await
    }

    /// Insert a CLI-origin ticket. The CLI has already x25519-wrapped its
    /// secret to the server's static public key; we store the ciphertext
    /// and relay it (re-wrapped) to the webapp redeemer. Returns the ticket
    /// UUID and a human-readable short code.
    pub async fn insert_cli(
        &self,
        issuer_pubkey_base58: String,
        wrapped_secret: Vec<u8>,
        eph_pub: [u8; 32],
    ) -> Result<(Uuid, String), BootstrapInsertError> {
        let ticket_id = Uuid::new_v4();
        let short_code = generate_short_code();
        let entry = BootstrapTicket {
            ticket_id,
            origin: TicketOrigin::Cli,
            keypair_json: String::new(),
            eph_pub,
            wrapped_secret,
            issuer_pubkey_base58: issuer_pubkey_base58.clone(),
            short_code: short_code.clone(),
            jwt_sub: issuer_pubkey_base58.clone(),
            expires_at: 0, // filled in below
        };
        self.insert_inner(issuer_pubkey_base58, entry)
            .await
            .map(|id| (id, short_code))
    }

    /// Common insertion logic shared by `insert` and `insert_cli`. Sets
    /// `expires_at` and handles LRU eviction accounting.
    async fn insert_inner(
        &self,
        user_key: String,
        mut entry: BootstrapTicket,
    ) -> Result<Uuid, BootstrapInsertError> {
        let mut guard = self.inner.lock().await;

        // Per-user cap.
        let count = guard.per_user.get(&user_key).copied().unwrap_or(0);
        if count >= guard.per_user_cap {
            return Err(BootstrapInsertError::PerUserCapExceeded);
        }

        let ticket_id = entry.ticket_id;
        let now = chrono::Utc::now().timestamp();
        entry.expires_at = now + guard.ttl_seconds;

        // Insert; if the LRU evicts an unrelated entry, decrement THAT
        // user's counter so accounting stays consistent.
        if let Some((_evicted_id, evicted)) = guard.lru.push(ticket_id, entry) {
            // The evicted entry's user might be the same as ours (if we are
            // already at exactly capacity). Standard `LruCache::push` returns
            // displaced entries; we rely on uniqueness of the freshly-minted
            // UUID so this is always an unrelated eviction — but defensively
            // skip the dec_user when somehow the evicted entry IS our just-
            // inserted one (impossible with a fresh UUID, but cheap to guard).
            if guard.lru.peek(&ticket_id).is_some() || evicted.jwt_sub != user_key {
                guard.dec_user(&evicted.jwt_sub);
            }
        }

        *guard.per_user.entry(user_key).or_insert(0) += 1;
        Ok(ticket_id)
    }

    /// Atomic remove-and-return. Returns the ticket exactly once; concurrent
    /// or replay redeems return None. Expired entries are lazily evicted on
    /// access and surface as None (404 to the caller — indistinguishable from
    /// "not found / already consumed", per the task spec).
    pub async fn consume(&self, ticket_id: Uuid) -> Option<BootstrapTicket> {
        let mut guard = self.inner.lock().await;
        let entry = guard.lru.pop(&ticket_id)?;
        let now = chrono::Utc::now().timestamp();
        if now >= entry.expires_at {
            // Expired — drop the per-user counter and pretend it never existed.
            guard.dec_user(&entry.jwt_sub);
            return None;
        }
        guard.dec_user(&entry.jwt_sub);
        Some(entry)
    }

    /// Atomic find-by-short-code and remove. Walks the LRU (O(n)) to locate
    /// a ticket by `short_code`, then delegates to the same expiry/counter
    /// logic as `consume`. Returns `None` if not found, expired, or redeemed.
    /// Single-use: the entry is removed before returning.
    /// Currently used only in test/test-support contexts; allow dead-code lint.
    #[allow(dead_code)]
    pub async fn consume_by_short_code(&self, short_code: &str) -> Option<BootstrapTicket> {
        let mut guard = self.inner.lock().await;
        // Find the ticket_id for this short_code by scanning the LRU. The
        // LRU is capped at 1000 entries so the O(n) walk is bounded.
        let ticket_id = guard
            .lru
            .iter()
            .find(|(_, t)| t.short_code == short_code)
            .map(|(id, _)| *id)?;
        let entry = guard.lru.pop(&ticket_id)?;
        let now = chrono::Utc::now().timestamp();
        if now >= entry.expires_at {
            guard.dec_user(&entry.jwt_sub);
            return None;
        }
        guard.dec_user(&entry.jwt_sub);
        Some(entry)
    }

    /// Test helper: number of entries currently stored.
    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.lru.len()
    }

    /// Test helper: per-user counter (without mutation).
    #[cfg(test)]
    pub async fn user_count(&self, jwt_sub: &str) -> usize {
        self.inner
            .lock()
            .await
            .per_user
            .get(jwt_sub)
            .copied()
            .unwrap_or(0)
    }

    /// Test helper: force-expire a ticket so `consume` returns None without
    /// having to advance wall-clock time. Mirrors `PendingBundles::force_expire`.
    #[cfg(test)]
    pub async fn force_expire(&self, ticket_id: &Uuid) {
        let mut guard = self.inner.lock().await;
        if let Some(entry) = guard.lru.get_mut(ticket_id) {
            entry.expires_at = chrono::Utc::now().timestamp() - 1;
        }
    }

    /// Test helper: force-expire a CLI-origin ticket by short_code.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub async fn force_expire_by_short_code(&self, short_code: &str) {
        let mut guard = self.inner.lock().await;
        let ticket_id = guard
            .lru
            .iter()
            .find(|(_, t)| t.short_code == short_code)
            .map(|(id, _)| *id);
        if let Some(id) = ticket_id {
            if let Some(entry) = guard.lru.get_mut(&id) {
                entry.expires_at = chrono::Utc::now().timestamp() - 1;
            }
        }
    }
}

/// `POST /api/cli-bootstrap/issue` — webapp issues a ticket bound to its
/// own JWT. Auth: Bearer JWT (the existing `bearer_auth_middleware` decodes
/// `Claims` and inserts them as a request extension before this handler
/// runs). Without a JWT the middleware returns 401 — this handler is never
/// reached.
#[derive(Debug, Deserialize)]
pub struct BootstrapIssueRequest {
    /// JSON-encoded Solana keypair (a JSON array of 64 bytes — secret + pub).
    /// Stored verbatim; the server never parses or interprets the bytes.
    pub keypair_json: String,
}

#[derive(Debug, Serialize)]
pub struct BootstrapIssueResponse {
    pub ticket_id: String,
}

pub async fn bootstrap_issue_handler(
    State(state): State<Arc<McpState>>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<BootstrapIssueRequest>,
) -> Response {
    // Sanity: the server never inspects keypair_json beyond a non-empty
    // check. The CLI parses the JSON on its end.
    if req.keypair_json.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "keypair_json is required"})),
        )
            .into_response();
    }

    match state
        .bootstrap_tickets
        .insert(claims.sub.clone(), req.keypair_json)
        .await
    {
        Ok(ticket_id) => (
            StatusCode::OK,
            Json(BootstrapIssueResponse {
                ticket_id: ticket_id.to_string(),
            }),
        )
            .into_response(),
        Err(BootstrapInsertError::PerUserCapExceeded) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": format!(
                    "per-user bootstrap-ticket cap exceeded ({} active)",
                    BOOTSTRAP_PER_USER_CAP
                ),
            })),
        )
            .into_response(),
        Err(BootstrapInsertError::LruExhausted) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "bootstrap-ticket store exhausted; retry shortly"
            })),
        )
            .into_response(),
    }
}

/// `GET /api/cli-bootstrap/redeem/:ticket` — CLI or webapp consumes a ticket.
/// NO auth: the UUID is the capability (high-entropy single-use bearer token).
/// The `bearer_auth_middleware` allowlist exempts this prefix.
///
/// Handles both origins:
/// - `Webapp` origin: returns `{secret: number[64], pubkey_base58}` (original
///   Decision-7 shape). No crypto involved.
/// - `Cli` origin: expects query/body `{redeemer_eph_pub: <base64 32B>}`.
///   Unwraps with server's static x25519 SK, re-wraps to redeemer's pub, and
///   returns `{wrapped_secret, eph_pub, origin, issuer_pubkey_base58}`.
///
/// 404 on missing / expired / already-consumed tickets. Body is identical for
/// all three states so a probing attacker cannot distinguish.
#[derive(Debug, Serialize)]
pub struct BootstrapRedeemResponse {
    pub secret: Vec<u8>,
    pub pubkey_base58: String,
}

/// Request body for `GET /api/cli-bootstrap/redeem/:ticket` when the ticket
/// has a `Cli` origin. Redeemer supplies its ephemeral x25519 public key so
/// the server can re-wrap the secret to them.
#[derive(Debug, Deserialize, Default)]
pub struct BootstrapRedeemBody {
    /// Base64 (standard) encoded 32-byte x25519 ephemeral public key of the
    /// webapp redeemer. Required only for `Cli`-origin tickets.
    #[serde(default)]
    pub redeemer_eph_pub: Option<String>,
}

/// Response for `Cli`-origin ticket redemption.
#[derive(Debug, Serialize)]
pub struct CliRedeemResponse {
    /// Base64-encoded re-wrapped secret (nonce[24] || ciphertext). The
    /// redeemer uses its ephemeral SK + `eph_pub` to unwrap.
    pub wrapped_secret: String,
    /// Base64-encoded server ephemeral public key (32 bytes). The redeemer
    /// DH's this with its ephemeral SK to derive the shared key.
    pub eph_pub: String,
    /// Origin of the ticket: "Cli" or "Webapp".
    pub origin: TicketOrigin,
    /// Base58-encoded Ed25519 pubkey of the CLI that issued the ticket.
    pub issuer_pubkey_base58: String,
}

/// Shared post-`consume` logic for both redeem handlers. Performs origin
/// dispatch: Webapp-origin tickets return the keypair bytes directly;
/// Cli-origin tickets unwrap the server-encrypted secret and re-wrap it
/// to the redeemer's ephemeral x25519 public key.
///
/// `redeemer_eph_pub` is a base64-encoded 32-byte x25519 public key —
/// required only for `Cli`-origin tickets.
fn finalize_redeem(
    entry: BootstrapTicket,
    redeemer_eph_pub: Option<String>,
    state: &McpState,
) -> Response {
    match entry.origin {
        TicketOrigin::Webapp => {
            // Original Decision-7 flow: return keypair bytes directly.
            let bytes: Vec<u8> = match serde_json::from_str::<Vec<u8>>(&entry.keypair_json) {
                Ok(v) => v,
                Err(e) => {
                    return error_resp(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("stored keypair_json is not a JSON byte array: {e}"),
                    )
                }
            };
            if bytes.len() != 64 {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("stored keypair must be 64 bytes, got {}", bytes.len()),
                );
            }
            let pubkey = solana_sdk::pubkey::Pubkey::try_from(&bytes[32..64])
                .map(|pk| pk.to_string())
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(BootstrapRedeemResponse {
                    secret: bytes,
                    pubkey_base58: pubkey,
                }),
            )
                .into_response()
        }
        TicketOrigin::Cli => {
            // Task-12 symmetric flow: unwrap with server SK, re-wrap to redeemer.
            let redeemer_pub_b64 = match redeemer_eph_pub {
                Some(s) => s,
                None => {
                    return error_resp(
                        StatusCode::BAD_REQUEST,
                        "redeemer_eph_pub is required for Cli-origin tickets",
                    )
                }
            };
            let redeemer_pub_bytes = match base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                redeemer_pub_b64.as_bytes(),
            ) {
                Ok(b) => b,
                Err(e) => {
                    return error_resp(
                        StatusCode::BAD_REQUEST,
                        &format!("redeemer_eph_pub is not valid base64: {e}"),
                    )
                }
            };
            if redeemer_pub_bytes.len() != 32 {
                return error_resp(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "redeemer_eph_pub must be 32 bytes, got {}",
                        redeemer_pub_bytes.len()
                    ),
                );
            }
            let redeemer_pub =
                X25519PublicKey::from(<[u8; 32]>::try_from(redeemer_pub_bytes.as_slice()).unwrap());

            // Unwrap with server static SK + issuer eph pub.
            let issuer_pub = X25519PublicKey::from(entry.eph_pub);
            let server_box = SalsaBox::new(&issuer_pub, &state.bootstrap_server_x25519_secret);
            if entry.wrapped_secret.len() < 24 {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "stored wrapped_secret is too short to contain a nonce",
                );
            }
            let nonce_bytes: [u8; 24] = entry.wrapped_secret[..24].try_into().unwrap();
            let nonce = crypto_box::aead::generic_array::GenericArray::from(nonce_bytes);
            // SAFETY: Plaintext window is deliberately minimal — unwrap and
            // immediately re-wrap without binding to a named variable.
            let plaintext = match server_box.decrypt(&nonce, &entry.wrapped_secret[24..]) {
                Ok(p) => p,
                Err(_) => {
                    return error_resp(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to decrypt wrapped_secret",
                    )
                }
            };

            // Re-wrap to the redeemer's ephemeral pub using a fresh server ephemeral.
            let server_eph_sk = X25519SecretKey::generate(&mut OsRng);
            let server_eph_pub = server_eph_sk.public_key();
            let redeemer_box = SalsaBox::new(&redeemer_pub, &server_eph_sk);
            let out_nonce = SalsaBox::generate_nonce(&mut OsRng);
            let rewrapped = match redeemer_box.encrypt(&out_nonce, plaintext.as_slice()) {
                Ok(c) => c,
                Err(_) => {
                    return error_resp(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to re-encrypt for redeemer",
                    )
                }
            };
            // Plaintext is dropped here — scope is intentionally tight.
            drop(plaintext);

            let mut transport = out_nonce.to_vec();
            transport.extend_from_slice(&rewrapped);

            use base64::Engine;
            let wrapped_b64 = base64::engine::general_purpose::STANDARD.encode(&transport);
            let eph_pub_b64 =
                base64::engine::general_purpose::STANDARD.encode(server_eph_pub.as_bytes());
            let issuer_pubkey_base58 = entry.issuer_pubkey_base58.clone();

            (
                StatusCode::OK,
                Json(CliRedeemResponse {
                    wrapped_secret: wrapped_b64,
                    eph_pub: eph_pub_b64,
                    origin: TicketOrigin::Cli,
                    issuer_pubkey_base58,
                }),
            )
                .into_response()
        }
    }
}

pub async fn bootstrap_redeem_handler(
    State(state): State<Arc<McpState>>,
    Path(ticket_str): Path<String>,
    body: Option<Json<BootstrapRedeemBody>>,
) -> Response {
    // Parse the ticket UUID. Invalid UUIDs collapse to 404.
    let ticket_id = match Uuid::parse_str(&ticket_str) {
        Ok(id) => id,
        Err(_) => return bootstrap_not_found(),
    };

    let entry = match state.bootstrap_tickets.consume(ticket_id).await {
        Some(t) => t,
        None => return bootstrap_not_found(),
    };

    let redeemer_eph_pub = body.and_then(|b| b.redeemer_eph_pub.clone());
    finalize_redeem(entry, redeemer_eph_pub, &state)
}

/// Request body for `POST /api/cli-bootstrap/redeem`.
#[derive(Debug, Deserialize)]
pub struct BootstrapRedeemByCodeRequest {
    pub short_code: String,
    /// Required for Cli-origin tickets; absent for Webapp-origin.
    #[serde(default)]
    pub redeemer_eph_pub: Option<String>,
}

/// `POST /api/cli-bootstrap/redeem` — lookup by short_code (user-visible
/// capability; vs the UUID-based GET variant). Body:
///   { short_code: "ABCD-1234", redeemer_eph_pub: <base64 32B> }
/// Returns same shapes as bootstrap_redeem_handler:
/// - Webapp origin: {secret: number[64], pubkey_base58}
/// - Cli origin: {wrapped_secret, eph_pub, origin, issuer_pubkey_base58}
///
/// 404 on missing / expired / already-consumed codes. Body is identical for
/// all three states so a probing attacker cannot distinguish (same as the
/// UUID-based GET variant).
pub async fn bootstrap_redeem_by_code_handler(
    State(state): State<Arc<McpState>>,
    Json(body): Json<BootstrapRedeemByCodeRequest>,
) -> Response {
    let entry = match state
        .bootstrap_tickets
        .consume_by_short_code(&body.short_code)
        .await
    {
        Some(t) => t,
        None => return bootstrap_not_found(),
    };

    finalize_redeem(entry, body.redeemer_eph_pub, &state)
}

/// Request body for `POST /api/cli-bootstrap/issue-from-cli`.
#[derive(Debug, Deserialize)]
pub struct CliBootstrapIssueRequest {
    /// Base64-encoded ciphertext (nonce[24] || ciphertext). The plaintext
    /// has been encrypted to the server's static x25519 public key.
    pub wrapped_secret: String,
    /// Base64-encoded 32-byte ephemeral x25519 public key of the CLI issuer.
    pub eph_pub: String,
    /// Base58-encoded Ed25519 pubkey of the CLI. Relayed to the redeemer
    /// in the redeem response.
    pub issuer_pubkey_base58: String,
}

/// Response for `POST /api/cli-bootstrap/issue-from-cli`.
#[derive(Debug, Serialize)]
pub struct CliBootstrapIssueResponse {
    pub ticket_id: String,
    pub short_code: String,
    pub expires_at: String,
}

/// `POST /api/cli-bootstrap/issue-from-cli` — CLI issues a ticket destined
/// for webapp redemption. No auth header required: the x25519 wrap to the
/// server's static key IS the capability proof.
///
/// Body: `{wrapped_secret, eph_pub, issuer_pubkey_base58}`
/// Response: `{ticket_id, short_code, expires_at}`
pub async fn bootstrap_issue_from_cli_handler(
    State(state): State<Arc<McpState>>,
    Json(req): Json<CliBootstrapIssueRequest>,
) -> Response {
    // Decode wrapped_secret.
    let wrapped_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.wrapped_secret.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("wrapped_secret is not valid base64: {e}"),
            )
        }
    };
    if wrapped_bytes.len() < 24 {
        return error_resp(
            StatusCode::BAD_REQUEST,
            "wrapped_secret is too short (must contain at least a 24-byte nonce)",
        );
    }

    // Decode eph_pub.
    let eph_pub_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.eph_pub.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                StatusCode::BAD_REQUEST,
                &format!("eph_pub is not valid base64: {e}"),
            )
        }
    };
    if eph_pub_bytes.len() != 32 {
        return error_resp(
            StatusCode::BAD_REQUEST,
            &format!("eph_pub must be 32 bytes, got {}", eph_pub_bytes.len()),
        );
    }
    let eph_pub: [u8; 32] = eph_pub_bytes.try_into().unwrap();

    if req.issuer_pubkey_base58.trim().is_empty() {
        return error_resp(StatusCode::BAD_REQUEST, "issuer_pubkey_base58 is required");
    }

    match state
        .bootstrap_tickets
        .insert_cli(req.issuer_pubkey_base58, wrapped_bytes, eph_pub)
        .await
    {
        Ok((ticket_id, short_code)) => {
            let expires_at = chrono::Utc::now() + chrono::Duration::seconds(BOOTSTRAP_TTL_SECS);
            (
                StatusCode::OK,
                Json(CliBootstrapIssueResponse {
                    ticket_id: ticket_id.to_string(),
                    short_code,
                    expires_at: expires_at.to_rfc3339(),
                }),
            )
                .into_response()
        }
        Err(BootstrapInsertError::PerUserCapExceeded) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": format!(
                    "per-issuer bootstrap-ticket cap exceeded ({} active)",
                    BOOTSTRAP_PER_USER_CAP
                ),
            })),
        )
            .into_response(),
        Err(BootstrapInsertError::LruExhausted) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "bootstrap-ticket store exhausted; retry shortly"
            })),
        )
            .into_response(),
    }
}

/// `GET /api/cli-bootstrap/server-pub` — return the server's static x25519
/// public key. No auth required. The CLI uses this to wrap its secret before
/// calling `/issue-from-cli`. The key is stable for the process lifetime;
/// restarting the server invalidates all in-flight CLI tickets (acceptable
/// given the 5-minute TTL).
#[derive(Debug, Serialize)]
pub struct ServerPubResponse {
    /// Base64-encoded 32-byte x25519 public key.
    pub server_x25519_pub: String,
}

pub async fn bootstrap_server_pub_handler(State(state): State<Arc<McpState>>) -> Response {
    use base64::Engine;
    let pub_b64 = base64::engine::general_purpose::STANDARD
        .encode(state.bootstrap_server_x25519_public.as_bytes());
    (
        StatusCode::OK,
        Json(ServerPubResponse {
            server_x25519_pub: pub_b64,
        }),
    )
        .into_response()
}

// ── Public traction stats (GET /stats) ──────────────────────────────────────
//
// Public, unauthenticated counters surfaced on the webapp landing page.
// Three numbers, no PII: distinct OAuth-resolved owners, total memories
// persisted by this node, and the subset that landed on Solana (i.e. has a
// non-`local:` solana_tx). Cached for 60s to keep the landing page from
// hammering SQLite under concurrent loads.

#[derive(Clone, Copy, Serialize)]
struct PublicStatsBody {
    unique_users: i64,
    saved_on_node: i64,
    saved_onchain: i64,
}

static STATS_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<(std::time::Instant, PublicStatsBody)>>,
> = std::sync::OnceLock::new();

const STATS_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// `GET /stats` — public traction counters for the webapp landing page.
///
/// No auth, no per-IP rate limit (the 60s in-process cache makes the SQL
/// load O(1) per minute regardless of request volume). Always returns a
/// 200 JSON body — on a transient SQLite error we serve zeroes rather
/// than 5xx the public homepage.
pub async fn public_stats_handler(State(state): State<Arc<McpState>>) -> Response {
    let cache = STATS_CACHE.get_or_init(|| std::sync::Mutex::new(None));

    {
        let guard = cache.lock().unwrap();
        if let Some((stamped_at, body)) = *guard {
            if stamped_at.elapsed() < STATS_TTL {
                return Json(body).into_response();
            }
        }
    }

    let body = {
        let store = state.store.lock().unwrap();
        match store.public_stats() {
            Ok(stats) => PublicStatsBody {
                unique_users: stats.unique_users,
                saved_on_node: stats.saved_on_node,
                saved_onchain: stats.saved_onchain,
            },
            Err(e) => {
                tracing::warn!("public_stats query failed: {e}");
                PublicStatsBody {
                    unique_users: 0,
                    saved_on_node: 0,
                    saved_onchain: 0,
                }
            }
        }
    };

    {
        let mut guard = cache.lock().unwrap();
        *guard = Some((std::time::Instant::now(), body));
    }

    Json(body).into_response()
}

// ── Public read routes — Ledger / Analytics / Blog (webapp-rethink T8) ───────
//
// Three unauthenticated GET surfaces consumed cross-origin by the webapp
// (`mnemonik.xyz`, localhost). They render NO HTML (Decision 9 — the mcp
// server is headless JSON); HTML/SEO is produced webapp-side. All rows come
// from the Task-7 core queries on `SqliteStore`; no payment gating, no owner
// scope leaks.
//
// Privacy invariant (Decision 6): `/artifacts` exposes `visibility = public`
// rows ONLY. The plain list uses `list_public_artifacts` (public-only by
// construction); the `?q=` content-search uses `search(.., owner = None,
// Some(Visibility::Public), ..)` — the cross-owner *public pool* path. Neither
// can surface a private or NULL-owner row.
//
// Lock discipline (CLAUDE.md): `rusqlite::Connection` is `!Send`. Every handler
// performs any slow/`async` work (embedding the query) BEFORE taking the
// `std::sync::Mutex` guard, then runs an await-free critical section and drops
// the guard before serialising the response.

/// Default / max page size for `GET /artifacts` and `GET /blog`. The max
/// clamps a hostile `?limit=` so an anonymous caller cannot ask the node to
/// materialise an unbounded result set.
const READ_DEFAULT_LIMIT: usize = 50;
const READ_MAX_LIMIT: usize = 200;

/// Query params for `GET /artifacts?q=&limit=`.
#[derive(Debug, Deserialize)]
pub struct ArtifactsQuery {
    /// Optional content search. When present, routes through the cosine
    /// `search` path restricted to the public pool; when absent, the plain
    /// newest-first public listing is returned.
    #[serde(default)]
    pub q: Option<String>,
    /// Optional page-size override; clamped to `[1, READ_MAX_LIMIT]`.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Project a cross-owner `SearchResult` down to the `PublicArtifact` wire
/// shape so the `?q=` search path and the plain-list path emit byte-identical
/// row shapes. Drops `visibility` (always `Public` here by the query filter)
/// and `relevance_score` (not part of the Ledger contract).
fn public_artifact_from_search(r: SearchResult) -> PublicArtifact {
    PublicArtifact {
        attestation_id: r.attestation_id,
        content: r.content,
        content_hash: r.content_hash,
        tags: r.tags,
        solana_tx: r.solana_tx,
        arweave_tx: r.arweave_tx,
        created_at: r.created_at,
        write_mode: r.write_mode,
    }
}

/// `GET /artifacts?q=&limit=` — public Evidence Ledger listing.
///
/// Returns `{ artifacts: [PublicArtifact], total }`. Public-visibility rows
/// ONLY (Decision 6). With `?q=`, runs cosine search over the cross-owner
/// public pool; without it, returns the newest-first public listing. On a
/// transient SQLite error we log and serve an empty list with 200 rather than
/// 5xx the public page (mirrors `public_stats_handler`).
pub async fn artifacts_handler(
    State(state): State<Arc<McpState>>,
    Query(params): Query<ArtifactsQuery>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(READ_DEFAULT_LIMIT)
        .clamp(1, READ_MAX_LIMIT);
    let query = params.q.as_deref().map(str::trim).filter(|s| !s.is_empty());

    // Embed the search query OUTSIDE the store lock — `embed` is synchronous
    // but potentially slow (ONNX inference in production), and the rusqlite
    // guard must never wrap slow work.
    let query_emb = query.map(|text| state.embedder.embed(text));

    let artifacts: Vec<PublicArtifact> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        let result = match &query_emb {
            // Content search → cross-owner cosine search restricted to the
            // public pool. `owner = None` paired with
            // `Some(Visibility::Public)` is the trait-mandated safe pairing
            // (never exposes private rows).
            Some(emb) => store
                .search(emb, None, Some(Visibility::Public), limit)
                .map(|rows| rows.into_iter().map(public_artifact_from_search).collect()),
            // Plain list → newest-first public-only chronological listing.
            None => store.list_public_artifacts(limit),
        };
        match result {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("artifacts query failed: {e}");
                Vec::new()
            }
        }
    };

    let total = artifacts.len();
    Json(serde_json::json!({ "artifacts": artifacts, "total": total })).into_response()
}

/// Query params for `GET /analytics/attestations?range=`.
#[derive(Debug, Deserialize)]
pub struct AnalyticsQuery {
    /// `30d` | `90d` | `12m` | `all` (or absent). Unrecognised values fall
    /// back to all-time so the public page never 400s on a stray param.
    #[serde(default)]
    pub range: Option<String>,
}

/// Map a `range` token to an inclusive ISO-8601 lower bound on `created_at`.
/// `None` means "all time" (no lower bound). The bound is rendered with the
/// same `to_rfc3339()` format the persistence path uses, so the SQL
/// `created_at >= ?1` lexicographic comparison is well-defined.
fn range_to_since(range: Option<&str>) -> Option<String> {
    let days = match range {
        Some("30d") => 30,
        Some("90d") => 90,
        Some("12m") => 365,
        // "all", absent, or any unrecognised value → all-time.
        _ => return None,
    };
    Some((chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339())
}

/// `GET /analytics/attestations?range=` — attestation traction over time.
///
/// Returns `{ buckets:[{date,on_node,on_chain}], total_on_node,
/// total_on_chain, unique_users }`. Buckets are sparse (days with no rows are
/// absent — the client fills gaps for charting). `unique_users` is the
/// all-time distinct-owner count (`public_stats`); core exposes no
/// range-scoped distinct-owner query, so this field is range-independent by
/// design. Aggregate counts only — never exposes row content.
pub async fn analytics_attestations_handler(
    State(state): State<Arc<McpState>>,
    Query(params): Query<AnalyticsQuery>,
) -> Response {
    let since = range_to_since(params.range.as_deref());

    let (buckets, unique_users) = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        let buckets = store
            .attestation_timeline(since.as_deref())
            .unwrap_or_else(|e| {
                tracing::warn!("attestation_timeline query failed: {e}");
                Vec::new()
            });
        let unique_users = store.public_stats().map(|s| s.unique_users).unwrap_or(0);
        (buckets, unique_users)
    };

    let total_on_node: i64 = buckets.iter().map(|b| b.on_node).sum();
    let total_on_chain: i64 = buckets.iter().map(|b| b.on_chain).sum();

    Json(serde_json::json!({
        "buckets": buckets,
        "total_on_node": total_on_node,
        "total_on_chain": total_on_chain,
        "unique_users": unique_users,
    }))
    .into_response()
}

/// Query params for `GET /blog?limit=`.
#[derive(Debug, Deserialize)]
pub struct BlogQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /blog?limit=` — list public blog posts newest-first.
///
/// Returns `{ posts: [BlogPost] }`. On a transient SQLite error we log and
/// serve an empty list with 200 (the page stays alive).
pub async fn blog_list_handler(
    State(state): State<Arc<McpState>>,
    Query(params): Query<BlogQuery>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(READ_DEFAULT_LIMIT)
        .clamp(1, READ_MAX_LIMIT);

    let posts = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        store.list_blog_posts(limit).unwrap_or_else(|e| {
            tracing::warn!("list_blog_posts query failed: {e}");
            Vec::new()
        })
    };

    Json(serde_json::json!({ "posts": posts })).into_response()
}

/// `GET /blog/:slug` — fetch a single public blog post.
///
/// Returns `{ post: BlogPost }` on hit; `404 { error, slug }` on an unknown
/// slug. A transient SQLite error is also surfaced as 404 (treated as "not
/// found") so the public page falls back gracefully rather than 5xx'ing.
pub async fn blog_post_handler(
    State(state): State<Arc<McpState>>,
    Path(slug): Path<String>,
) -> Response {
    let lookup = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        store.get_blog_post(&slug)
    };

    match lookup {
        Ok(Some(post)) => Json(serde_json::json!({ "post": post })).into_response(),
        Ok(None) => blog_post_not_found(&slug),
        Err(e) => {
            tracing::warn!("get_blog_post query failed: {e}");
            blog_post_not_found(&slug)
        }
    }
}

/// Uniform 404 body for the blog-detail route.
fn blog_post_not_found(slug: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "blog post not found", "slug": slug })),
    )
        .into_response()
}

// ── Agent publish surface — POST /blog (webapp-rethink T9, Decision 5) ────────
//
// W3C-Micropub-shaped authenticated publish endpoint. Sits behind
// `oauth::bearer_auth_middleware` in `main.rs` (anonymous → 401 there); the
// handler ALSO checks for `Claims` so it is self-contained. Accepts BOTH
// `application/json` and `application/x-www-form-urlencoded` (Micropub) bodies
// and funnels them through the SAME `publish::publish_post` pipeline as the
// `mnemonic_publish_post` MCP tool — one signing/persistence path, no drift.
// JSON only on the way out (Decision 9 — the mcp server renders no HTML).

/// `POST /blog` — Micropub-shaped agent publish. Authenticated (OAuth2 Bearer /
/// Ed25519). On success returns `201 Created` with a `Location: /blog/:slug`
/// header and `{ post }` (the same shape `GET /blog/:slug` serves).
pub async fn blog_publish_handler(
    State(state): State<Arc<McpState>>,
    claims: Option<Extension<Claims>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Auth (Decision 5): in production the bearer-auth middleware validates the
    // JWT and inserts `Claims` before this handler runs (anonymous → 401 there).
    // This explicit check is defence-in-depth AND makes the handler reject an
    // unauthenticated request on its own — never publish anonymously.
    let Some(Extension(claims)) = claims else {
        return error_resp(
            StatusCode::UNAUTHORIZED,
            "authentication required to publish",
        );
    };

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let input = if content_type.contains("application/x-www-form-urlencoded") {
        parse_micropub_form(&body)
    } else if content_type.contains("application/json") || content_type.trim().is_empty() {
        // Default to JSON when the content-type is absent (programmatic agents
        // sometimes omit it); a malformed body is reported as 400 below.
        match parse_publish_json(&body) {
            Ok(i) => i,
            Err(e) => return error_resp(StatusCode::BAD_REQUEST, &e),
        }
    } else {
        return error_resp(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json or application/x-www-form-urlencoded",
        );
    };

    match crate::publish::publish_post(&state, &claims.sub, input) {
        Ok(post) => {
            let location = format!("/blog/{}", post.slug);
            let mut resp = (
                StatusCode::CREATED,
                Json(serde_json::json!({ "post": post })),
            )
                .into_response();
            if let Ok(hv) = HeaderValue::from_str(&location) {
                resp.headers_mut().insert(header::LOCATION, hv);
            }
            resp
        }
        Err(e) => blog_publish_error_response(e),
    }
}

/// Map a [`PublishError`] to its HTTP response.
fn blog_publish_error_response(e: PublishError) -> Response {
    let status = match &e {
        PublishError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        PublishError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        PublishError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_resp(status, &e.message())
}

/// Parse a JSON publish body. Accepts the simple shape
/// `{title, body_markdown, tags[], author?}` AND the W3C Micropub h-entry shape
/// `{type:["h-entry"], properties:{name:[..], content:[..], category:[..]}}`.
/// Field validation (required/empty/length) happens downstream in
/// `publish::publish_post`, so this layer only extracts.
fn parse_publish_json(body: &[u8]) -> Result<PublishInput, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))?;

    // Micropub h-entry: values live under `properties` as arrays.
    if let Some(props) = v.get("properties").and_then(|p| p.as_object()) {
        let props = serde_json::Value::Object(props.clone());
        return Ok(PublishInput {
            title: value_first_str(props.get("name")).unwrap_or_default(),
            body_markdown: micropub_content(props.get("content")).unwrap_or_default(),
            tags: value_to_str_vec(props.get("category")),
            author: value_first_str(props.get("author")),
        });
    }

    // Simple shape, tolerating Micropub-ish `name`/`content` aliases.
    Ok(PublishInput {
        title: v
            .get("title")
            .or_else(|| v.get("name"))
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        body_markdown: v
            .get("body_markdown")
            .or_else(|| v.get("content"))
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        tags: value_to_str_vec(v.get("tags").or_else(|| v.get("category"))),
        author: v
            .get("author")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    })
}

/// Parse an `application/x-www-form-urlencoded` Micropub body. Repeated
/// `category` / `category[]` keys accumulate into the tag list (Micropub
/// arrays). Control fields (`h`, `action`, `access_token`, …) are ignored —
/// auth is the Bearer header, not a body field.
fn parse_micropub_form(body: &[u8]) -> PublishInput {
    let mut title = String::new();
    let mut body_markdown = String::new();
    let mut tags: Vec<String> = Vec::new();
    let mut author: Option<String> = None;
    for (k, val) in url::form_urlencoded::parse(body) {
        match k.as_ref() {
            "name" | "title" => title = val.into_owned(),
            "content" | "body_markdown" => body_markdown = val.into_owned(),
            "category" | "category[]" | "tags" | "tags[]" => {
                let s = val.trim().to_string();
                if !s.is_empty() {
                    tags.push(s);
                }
            }
            "author" => {
                let s = val.into_owned();
                if !s.trim().is_empty() {
                    author = Some(s);
                }
            }
            _ => {}
        }
    }
    PublishInput {
        title,
        body_markdown,
        tags,
        author,
    }
}

/// First string of a Micropub value: the value itself when a string, or the
/// first string element when an array.
fn value_first_str(v: Option<&serde_json::Value>) -> Option<String> {
    match v {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(a)) => {
            a.iter().find_map(|x| x.as_str().map(|s| s.to_string()))
        }
        _ => None,
    }
}

/// Collect a Micropub value into a string vector: a single string becomes a
/// one-element vec; an array yields each of its string elements.
fn value_to_str_vec(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract the Micropub `content` property. Handles a bare string, an array of
/// strings, and the object forms `[{"markdown": "…"}]` / `[{"html": "…"}]` /
/// `[{"value": "…"}]` that some Micropub clients send.
fn micropub_content(v: Option<&serde_json::Value>) -> Option<String> {
    match v {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(a)) => a.iter().find_map(|item| match item {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(o) => o
                .get("markdown")
                .or_else(|| o.get("value"))
                .or_else(|| o.get("html"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

/// Uniform 404 body for every "ticket missing / expired / consumed" path so a
/// probing attacker cannot distinguish the three states.
fn bootstrap_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "ticket not found, already consumed, or expired",
        })),
    )
        .into_response()
}

// ── Discovery + syndication (webapp-rethink T10, Decision 5) ─────────────────
//
// "Movement, not memory": these surfaces only *advertise* and *syndicate* the
// agent-native publishing built in T9 — they do not publish anything. Both are
// public, read-only, and sit under the global CORS layer with NO bearer-auth.

/// Webapp public origin used for human-readable blog links emitted in the feed.
///
/// Per Decision 9 the webapp is a SEPARATE deploy from this mcp server, so a
/// feed reader's `<link>` to a post must target the webapp origin
/// (`https://mnemonik.xyz/blog/<slug>`), not this API. Configurable via
/// `WEBAPP_PUBLIC_BASE_URL`; a trailing slash is trimmed so concatenation never
/// double-slashes. Mirrors the `MCP_PUBLIC_BASE_URL` pattern in `oauth::mod`.
fn webapp_public_base() -> String {
    let raw = std::env::var("WEBAPP_PUBLIC_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://mnemonik.xyz".to_string());
    raw.trim().trim_end_matches('/').to_string()
}

/// AgentCard served from the API origin (`GET /.well-known/agent.json`).
///
/// The canonical, full AgentCard is the static file shipped by the webapp at
/// `https://mnemonik.xyz/.well-known/agent.json`. This API-origin card is a
/// focused *discovery* document for agents that only know the mcp endpoint: it
/// advertises the mcp service, the publish capability via the `x-mnemonic`
/// extension (BOTH the native `mnemonic_publish_post` MCP tool AND the Micropub
/// `POST /blog` interop surface, plus the OAuth2-Bearer auth requirement), and
/// points back to the canonical card. Discovery only — publishing lives in T9.
fn agent_card_json() -> serde_json::Value {
    let origin = crate::oauth::server_origin();
    serde_json::json!({
        "name": "Mnemonic Protocol",
        "description": "Verifiable, persistent memory for AI agents, exposed over MCP. This API-origin AgentCard advertises discovery + the agent-native publishing capability.",
        "url": "https://mnemonik.xyz",
        "canonical": "https://mnemonik.xyz/.well-known/agent.json",
        "services": [
            {
                "type": "mcp",
                "uri": format!("{origin}/mcp"),
                "transport": "http",
                "auth": "oauth2",
                "auth_metadata": format!("{origin}/.well-known/oauth-authorization-server"),
            }
        ],
        "x-mnemonic": {
            "publish": {
                "description": "Agents can publish public blog posts. Discovery only — use one of the surfaces below to publish.",
                "auth": "oauth2-bearer",
                "auth_metadata": format!("{origin}/.well-known/oauth-authorization-server"),
                "surfaces": [
                    {
                        "type": "mcp-tool",
                        "tool": "mnemonic_publish_post",
                        "transport": "mcp",
                        "native": true,
                    },
                    {
                        "type": "micropub",
                        "endpoint": format!("{origin}/blog"),
                        "method": "POST",
                        "formats": ["application/json", "application/x-www-form-urlencoded"],
                        "spec": "https://www.w3.org/TR/micropub/",
                    }
                ],
                "syndication": {
                    "feed": format!("{origin}/blog/feed.xml"),
                    "type": "application/atom+xml",
                }
            }
        },
        "version": "1",
        "spec_version": "draft-2026-05",
    })
}

/// `GET /.well-known/agent.json` — public AgentCard discovery for the API
/// origin. No auth, no state. See [`agent_card_json`].
pub async fn agent_card_handler() -> Response {
    Json(agent_card_json()).into_response()
}

/// Escape a string for inclusion in XML *text* content (between tags) or an
/// attribute value. We entity-escape rather than wrap in CDATA: this neutralizes
/// the `]]>` CDATA-break injection vector entirely (the `>` becomes `&gt;`), so
/// agent-authored titles/bodies cannot break out of the feed. Closes the XML
/// injection risk on the syndication surface.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Plain-text excerpt of a markdown body for the feed `<summary>`: collapse
/// whitespace and truncate on a char boundary so multi-byte UTF-8 is never
/// split. Escaping happens later in [`render_atom_feed`].
fn summarize(body: &str, max_chars: usize) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(max_chars).collect();
    if collapsed.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// Render an Atom 1.0 feed for the supplied posts. Pure (no I/O, no lock) so it
/// is unit-testable with synthetic posts. `mcp_origin` is this API's origin (for
/// the `self` link); `webapp_base` is the human blog origin (Decision 9) used
/// for per-entry links + ids. All agent-authored text is XML-escaped.
fn render_atom_feed(posts: &[BlogPost], mcp_origin: &str, webapp_base: &str) -> String {
    // Feed-level <updated>: newest post's timestamp (list is published_at DESC),
    // else now() for an empty feed so the document still validates.
    let updated = posts
        .first()
        .map(|p| p.published_at.clone())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str("  <title>Mnemonic Protocol Blog</title>\n");
    xml.push_str(&format!("  <id>{}/blog</id>\n", xml_escape(webapp_base)));
    xml.push_str(&format!(
        "  <link href=\"{}/blog\"/>\n",
        xml_escape(webapp_base)
    ));
    xml.push_str(&format!(
        "  <link rel=\"self\" type=\"application/atom+xml\" href=\"{}/blog/feed.xml\"/>\n",
        xml_escape(mcp_origin)
    ));
    xml.push_str(&format!("  <updated>{}</updated>\n", xml_escape(&updated)));

    for p in posts {
        let post_url = format!("{}/blog/{}", webapp_base, p.slug);
        let post_url = xml_escape(&post_url);
        xml.push_str("  <entry>\n");
        xml.push_str(&format!("    <title>{}</title>\n", xml_escape(&p.title)));
        xml.push_str(&format!("    <link href=\"{post_url}\"/>\n"));
        xml.push_str(&format!("    <id>{post_url}</id>\n"));
        xml.push_str(&format!(
            "    <updated>{}</updated>\n",
            xml_escape(&p.published_at)
        ));
        xml.push_str(&format!(
            "    <published>{}</published>\n",
            xml_escape(&p.published_at)
        ));
        xml.push_str(&format!(
            "    <author><name>{}</name></author>\n",
            xml_escape(&p.author)
        ));
        for tag in &p.tags {
            xml.push_str(&format!("    <category term=\"{}\"/>\n", xml_escape(tag)));
        }
        xml.push_str(&format!(
            "    <summary>{}</summary>\n",
            xml_escape(&summarize(&p.body_markdown, 280))
        ));
        xml.push_str("  </entry>\n");
    }

    xml.push_str("</feed>\n");
    xml
}

/// `GET /blog/feed.xml` — public Atom syndication of the published blog posts.
///
/// A feed is a data format consumed by feed readers (not browser page
/// rendering), so emitting it from the API is consistent with Decision 9. Reads
/// `list_blog_posts` under the store mutex, releases the lock, then renders.
pub async fn blog_feed_handler(State(state): State<Arc<McpState>>) -> Response {
    let posts = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => {
                return error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("store mutex poisoned: {e}"),
                );
            }
        };
        store
            .list_blog_posts(READ_DEFAULT_LIMIT)
            .unwrap_or_else(|e| {
                tracing::warn!("list_blog_posts query failed for feed: {e}");
                Vec::new()
            })
    };

    let body = render_atom_feed(&posts, crate::oauth::server_origin(), &webapp_public_base());
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/atom+xml; charset=utf-8"),
        )],
        body,
    )
        .into_response()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Decision-7 unit tests for BootstrapTickets — atomic consume,
    //! per-user cap, TTL eviction, and "redeem requires no auth" smoke.

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bootstrap_ticket_atomic_consume() {
        // Concurrent two-thread consume of the SAME ticket: exactly one
        // returns Some, the other returns None. Drives Decision 7's atomic
        // single-use guarantee.
        //
        // Round-2 fix: use a multi_thread runtime so spawned tasks can
        // genuinely race on real OS threads (the previous current_thread
        // runtime serialised them by the scheduler, masking any locking
        // bug). A `tokio::sync::Barrier` synchronises both tasks at the
        // call site so the two `consume` invocations cross the mutex
        // boundary at (effectively) the same instant. Looped 64 times to
        // amplify any race window — a regression where `consume` was
        // reduced to `peek` would flip `some_count` to 2 with very high
        // probability somewhere in the iteration count.
        for _iter in 0..64 {
            let store = Arc::new(BootstrapTickets::new(10, 5, 300));
            let id = store
                .insert("user-a".into(), "[1,2,3]".into())
                .await
                .unwrap();

            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let s1 = store.clone();
            let s2 = store.clone();
            let b1 = barrier.clone();
            let b2 = barrier.clone();
            let h1 = tokio::spawn(async move {
                b1.wait().await;
                s1.consume(id).await
            });
            let h2 = tokio::spawn(async move {
                b2.wait().await;
                s2.consume(id).await
            });
            let r1 = h1.await.unwrap();
            let r2 = h2.await.unwrap();
            let some_count = [&r1, &r2].iter().filter(|x| x.is_some()).count();
            assert_eq!(
                some_count, 1,
                "exactly one concurrent consume must succeed on iter {_iter} (r1={r1:?}, r2={r2:?})"
            );
            // Per-user counter zeroed after the successful consume.
            assert_eq!(store.user_count("user-a").await, 0);
        }
    }

    #[tokio::test]
    async fn test_bootstrap_ticket_single_use() {
        // Issue, consume, second consume returns None. TDD anchor (tasks/6.md).
        let store = BootstrapTickets::new(10, 5, 300);
        let id = store.insert("u".into(), "[]".into()).await.unwrap();
        assert!(store.consume(id).await.is_some());
        assert!(store.consume(id).await.is_none());
    }

    #[tokio::test]
    async fn test_bootstrap_ticket_per_user_cap() {
        // 4th insert by the same user returns 429.
        let store = BootstrapTickets::new(100, 3, 300);
        for _ in 0..3 {
            store.insert("alice".into(), "[]".into()).await.unwrap();
        }
        let r = store.insert("alice".into(), "[]".into()).await;
        assert!(matches!(r, Err(BootstrapInsertError::PerUserCapExceeded)));
        // Another user is unaffected.
        assert!(store.insert("bob".into(), "[]".into()).await.is_ok());
    }

    #[tokio::test]
    async fn test_bootstrap_ticket_ttl_expiry() {
        // Issue, force-expire, consume returns None. Wall-clock advance
        // would couple the test to real time; force_expire mirrors the
        // pattern used by `PendingBundles::force_expire`.
        let store = BootstrapTickets::new(10, 5, 300);
        let id = store.insert("u".into(), "[]".into()).await.unwrap();
        store.force_expire(&id).await;
        let result = store.consume(id).await;
        assert!(result.is_none(), "expired ticket must consume to None");
        // The expired entry was evicted as part of the consume call.
        assert_eq!(store.len().await, 0);
        assert_eq!(store.user_count("u").await, 0);
    }

    #[tokio::test]
    async fn test_bootstrap_ticket_lru_evicts_oldest() {
        // Insert capacity+1 by distinct users so per-user cap is not hit;
        // oldest entry is evicted, newest is retrievable.
        let store = BootstrapTickets::new(3, 5, 300);
        let mut ids = Vec::new();
        for i in 0..4 {
            let id = store
                .insert(format!("user{i}"), format!("[{i}]"))
                .await
                .unwrap();
            ids.push(id);
        }
        assert_eq!(store.len().await, 3);
        assert!(store.consume(ids[0]).await.is_none(), "oldest evicted");
        assert!(store.consume(ids[3]).await.is_some(), "newest retained");
    }

    /// Minimal in-memory router that mirrors the production wiring for
    /// `bootstrap_redeem_handler` so we can verify "no Authorization header
    /// required" end-to-end. The full McpState is unwieldy to mock here, so
    /// we wrap a thin handler over a bare `BootstrapTickets` instance and
    /// test the same code path.
    #[tokio::test]
    async fn test_bootstrap_redeem_no_auth_required() {
        use axum::extract::Path as AxumPath;
        use axum::routing::get;
        use axum::Router;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        // 64-byte fake keypair (zeros). pubkey will be 11111...1 base58.
        let bytes: Vec<u8> = vec![0u8; 64];
        let kp_json = serde_json::to_string(&bytes).unwrap();
        let store = Arc::new(BootstrapTickets::new(10, 5, 300));
        let id = store.insert("u".into(), kp_json).await.unwrap();

        // Tiny handler closure — same signature as production but takes the
        // BootstrapTickets directly so we don't have to construct McpState.
        async fn redeem(
            State(store): State<Arc<BootstrapTickets>>,
            AxumPath(ticket_str): AxumPath<String>,
        ) -> Response {
            let id = match Uuid::parse_str(&ticket_str) {
                Ok(id) => id,
                Err(_) => return bootstrap_not_found(),
            };
            match store.consume(id).await {
                Some(entry) => {
                    let bytes: Vec<u8> = serde_json::from_str(&entry.keypair_json).unwrap();
                    let pubkey = solana_sdk::pubkey::Pubkey::try_from(&bytes[32..64])
                        .map(|pk| pk.to_string())
                        .unwrap_or_default();
                    (
                        StatusCode::OK,
                        Json(BootstrapRedeemResponse {
                            secret: bytes,
                            pubkey_base58: pubkey,
                        }),
                    )
                        .into_response()
                }
                None => bootstrap_not_found(),
            }
        }

        let app: Router = Router::new()
            .route("/api/cli-bootstrap/redeem/{ticket}", get(redeem))
            .with_state(store);

        // No Authorization header — must succeed.
        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/cli-bootstrap/redeem/{id}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let secret = parsed["secret"].as_array().unwrap();
        assert_eq!(secret.len(), 64);
        assert!(parsed["pubkey_base58"].as_str().unwrap().len() > 30);
    }

    #[tokio::test]
    async fn test_bootstrap_redeem_unknown_ticket_404() {
        // Garbage UUID → 404. Indistinguishable from "consumed" / "expired".
        use axum::extract::Path as AxumPath;
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let store = Arc::new(BootstrapTickets::new(10, 5, 300));

        async fn redeem(
            State(store): State<Arc<BootstrapTickets>>,
            AxumPath(ticket_str): AxumPath<String>,
        ) -> Response {
            let id = match Uuid::parse_str(&ticket_str) {
                Ok(id) => id,
                Err(_) => return bootstrap_not_found(),
            };
            match store.consume(id).await {
                Some(_) => StatusCode::OK.into_response(),
                None => bootstrap_not_found(),
            }
        }

        let app: Router = Router::new()
            .route("/api/cli-bootstrap/redeem/{ticket}", get(redeem))
            .with_state(store);

        // Garbage UUID.
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/cli-bootstrap/redeem/not-a-uuid")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // Valid UUID but never inserted.
        let id = Uuid::new_v4();
        let req2 = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/cli-bootstrap/redeem/{id}"))
            .body(Body::empty())
            .unwrap();
        let resp2 = app.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
    }

    // ── T10: discovery + syndication ─────────────────────────────────────────

    fn sample_post(slug: &str, title: &str, body: &str) -> BlogPost {
        BlogPost {
            slug: slug.to_string(),
            title: title.to_string(),
            body_markdown: body.to_string(),
            tags: vec!["agents".to_string(), "memory".to_string()],
            author: "agent-alice".to_string(),
            attestation_id: "att-1".to_string(),
            content_hash: "hash-1".to_string(),
            published_at: "2026-06-27T12:00:00+00:00".to_string(),
        }
    }

    /// The API-origin AgentCard advertises the publish capability via the
    /// `x-mnemonic` extension: BOTH the native MCP tool AND the Micropub
    /// endpoint, plus the OAuth2-Bearer auth requirement.
    #[test]
    fn test_agent_card_advertises_publish_skill() {
        let card = agent_card_json();
        let publish = &card["x-mnemonic"]["publish"];
        assert!(publish.is_object(), "x-mnemonic.publish missing");

        // Auth requirement surfaced.
        assert_eq!(publish["auth"], "oauth2-bearer");

        // Both surfaces present.
        let surfaces = publish["surfaces"].as_array().expect("surfaces array");
        let has_mcp_tool = surfaces
            .iter()
            .any(|s| s["type"] == "mcp-tool" && s["tool"] == "mnemonic_publish_post");
        let has_micropub = surfaces
            .iter()
            .any(|s| s["type"] == "micropub" && s["method"] == "POST");
        assert!(has_mcp_tool, "native mnemonic_publish_post surface missing");
        assert!(has_micropub, "micropub POST surface missing");

        // Micropub endpoint points at the API origin's /blog.
        let micropub = surfaces.iter().find(|s| s["type"] == "micropub").unwrap();
        assert!(micropub["endpoint"].as_str().unwrap().ends_with("/blog"));

        // No private data leaks into the card (no api keys / pubkeys / db).
        let serialized = card.to_string();
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("private"));
    }

    /// The Atom feed lists published posts with the expected structure and
    /// links to the human blog page on the webapp origin (Decision 9).
    #[test]
    fn test_render_atom_feed_well_formed() {
        let posts = vec![
            sample_post("first-post", "First Post", "Hello world body."),
            sample_post("second-post", "Second Post", "Another body."),
        ];
        let xml = render_atom_feed(
            &posts,
            "https://mcp.example.test",
            "https://web.example.test",
        );

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        // self-link is the API origin; entry link is the webapp origin.
        assert!(xml.contains(
            "<link rel=\"self\" type=\"application/atom+xml\" href=\"https://mcp.example.test/blog/feed.xml\"/>"
        ));
        assert!(xml.contains("<link href=\"https://web.example.test/blog/first-post\"/>"));
        assert!(xml.contains("<title>First Post</title>"));
        assert!(xml.contains("<author><name>agent-alice</name></author>"));
        assert_eq!(xml.matches("<entry>").count(), 2);
        assert!(xml.trim_end().ends_with("</feed>"));
    }

    /// An empty feed is still a well-formed Atom document (no entries, valid
    /// `<updated>` so feed readers do not choke).
    #[test]
    fn test_render_atom_feed_empty() {
        let xml = render_atom_feed(&[], "https://mcp.example.test", "https://web.example.test");
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(xml.contains("<updated>"));
        assert_eq!(xml.matches("<entry>").count(), 0);
        assert!(xml.trim_end().ends_with("</feed>"));
    }

    /// XML injection from agent-authored content is neutralized: `<`, `&`, and
    /// the CDATA-break `]]>` are entity-escaped, so a malicious title cannot
    /// break out of the feed element.
    #[test]
    fn test_feed_xml_escapes_injection() {
        let post = sample_post(
            "evil",
            "Pwn <script>&amp; ]]><inject>",
            "body with <tag> & ]]> break",
        );
        let xml = render_atom_feed(
            &[post],
            "https://mcp.example.test",
            "https://web.example.test",
        );

        // The raw dangerous sequences must NOT appear unescaped inside the doc.
        assert!(!xml.contains("<script>"));
        assert!(!xml.contains("<inject>"));
        assert!(!xml.contains("]]>"));
        // They appear escaped instead.
        assert!(xml.contains("&lt;script&gt;"));
        assert!(xml.contains("&amp;amp;")); // the literal "&amp;" in the title re-escaped
        assert!(xml.contains("]]&gt;"));
    }

    #[test]
    fn test_xml_escape_all_entities() {
        assert_eq!(
            xml_escape("a<b>c&d\"e'f"),
            "a&lt;b&gt;c&amp;d&quot;e&apos;f"
        );
        assert_eq!(xml_escape("]]>"), "]]&gt;");
    }

    #[test]
    fn test_summarize_truncates_on_char_boundary() {
        // Multi-byte chars must not be split mid-codepoint.
        let body = " catégorie ".repeat(50);
        let s = summarize(&body, 10);
        assert!(s.chars().count() <= 11); // 10 + ellipsis
        assert!(s.ends_with('…'));
        // Whitespace is collapsed.
        let s2 = summarize("a\n\n  b\t c", 280);
        assert_eq!(s2, "a b c");
    }
}
