//! Chrome-extension key escrow + extension-bootstrap endpoints — T15 of
//! `work/chrome-extension/`.
//!
//! Implements Decision 9 server-side:
//!
//! - Argon2id-AES-GCM blob stored opaquely per `google_sub`. Server never
//!   sees plaintext.
//! - PUT validates `pubkey_base58` matches the linked `pubkey_base58` in
//!   `google_identity_links` (binds the encrypted key to the user's
//!   on-chain identity; stolen-Google-account → swap-key attack rejected).
//! - GET rate-limited to `KEY_ESCROW_RATE_LIMIT` (default 5) per rolling
//!   24h per `google_sub`. The window resets when `last_fetch_at` is older
//!   than 24h. Returns 429 with `Retry-After` once exceeded.
//! - DELETE is hard delete (D9 explicit revocation).
//!
//! ## Extension bootstrap (handshake)
//!
//! Mirrors the existing `cli-bootstrap` flow (`api.rs`), with one structural
//! difference: the **redeem response is a fresh JWT bound to `aud=extension`**,
//! not a raw keypair (the extension already has its own Ed25519 keypair from
//! T13's seed-restore flow). The webapp calls
//! `POST /api/extension-bootstrap/issue` with its current `aud=mcp` JWT (the
//! one minted via the Google OAuth callback path in T14); the server returns
//! a one-time UUID ticket. The extension polls
//! `GET /api/extension-bootstrap/redeem/:ticket` to consume the ticket and
//! get a JWT it can use against `/api/key-escrow`. Tickets are LRU + TTL
//! 10 min, single-use; the UUID is the capability so the redeem path needs
//! no Authorization header (uri-allowlisted in the bearer-auth middleware,
//! same model as cli-bootstrap).
//!
//! ## Architecture note
//!
//! The migration for `key_escrow_blobs` lives in this module under `mcp/`,
//! NOT in `core/src/storage/sqlite.rs`. This matches T14's precedent for
//! `google_identity_links` (see
//! `oauth/google.rs::migrate_google_identity_links`) and the project-wide
//! rule that OAuth/escrow tables are server concerns (per CLAUDE.md and
//! `decisions.md` D9). The original T15 task spec said migrations live in
//! `core/`; the deviation is documented in `decisions.md`.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use jsonwebtoken::{decode, encode, Algorithm, Header, Validation};
use lru::LruCache;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::mcp::McpState;
use crate::oauth::{Claims, OAuthState, JWT_ISSUER, JWT_TTL_SECS};

// ── Constants ────────────────────────────────────────────────────────────────

/// `aud` claim for extension-scoped JWTs. Separate from the `mcp` audience
/// used by AI-tool clients so the bearer-auth middleware on `/mcp` rejects
/// extension JWTs cleanly — they only get past the explicit escrow handlers
/// (which verify against this audience inline).
pub const EXTENSION_JWT_AUDIENCE: &str = "extension";

/// TTL of an extension-bootstrap ticket: 10 minutes — matches D9.
pub const EXTENSION_BOOTSTRAP_TTL_SECS: i64 = 600;

/// LRU capacity for in-flight extension-bootstrap tickets. Independent of
/// the cli-bootstrap cap (they're different flows).
pub const EXTENSION_BOOTSTRAP_LRU_CAP: usize = 10_000;

/// Per-user cap on outstanding extension-bootstrap tickets. Prevents a
/// compromised JWT from spamming tickets into the LRU.
pub const EXTENSION_BOOTSTRAP_PER_USER_CAP: usize = 3;

/// Rolling rate-limit window for key-escrow GETs: 24h. The DB row tracks the
/// count + last fetch timestamp so we never hold in-memory state for this.
pub const KEY_ESCROW_WINDOW_SECS: i64 = 24 * 3600;

// ── DB migration ─────────────────────────────────────────────────────────────

/// Create `key_escrow_blobs` if it does not exist. Idempotent — safe to call
/// on every startup. Lives in `mcp/` per the architectural rule that
/// OAuth/escrow tables are server-side, not part of the cross-client
/// `mnemonic-core` storage schema.
///
/// Schema (D9):
///   - `google_sub TEXT PRIMARY KEY`  — link key to `google_identity_links`.
///   - `ciphertext BLOB NOT NULL`     — AES-GCM ciphertext of the Ed25519
///     secret key. Server never decrypts.
///   - `nonce BLOB NOT NULL`          — AES-GCM 96-bit nonce.
///   - `kdf TEXT NOT NULL`            — KDF identifier (e.g. "argon2id").
///   - `kdf_params TEXT NOT NULL`     — JSON-encoded KDF params (mem/time/
///     parallelism/salt). Opaque to the server.
///   - `pubkey_base58 TEXT NOT NULL`  — bound Ed25519 pubkey. MUST equal the
///     `pubkey_base58` in `google_identity_links` for this `google_sub`. PUT
///     rejects mismatches with 403 — closes the stolen-google-account →
///     swap-key attack.
///   - `updated_at TEXT NOT NULL`     — RFC3339 timestamp of last PUT.
///   - `fetch_count_24h INTEGER`      — GET counter in the current 24h
///     window. Reset whenever the previous `last_fetch_at` is > 24h old.
///   - `last_fetch_at TEXT`           — RFC3339 timestamp of the previous GET
///     (NULL until first GET).
///
/// Foreign key constraint cascades on DELETE of the parent row so unlinking
/// a Google account cleans up its escrow. SQLite enforces FKs only when
/// `PRAGMA foreign_keys = ON;` is set — `SqliteStore::open` does this in
/// `core/` (verified at startup).
pub fn migrate_key_escrow_blobs(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create key_escrow_blobs table")?;
    Ok(())
}

/// Raw migration string — kept as a module-level `const` so the T15 TDD
/// anchor `server_never_stores_plaintext` can read it via this re-export
/// and assert that no plaintext-leaking column name is present.
pub const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS key_escrow_blobs (
    google_sub      TEXT PRIMARY KEY,
    ciphertext      BLOB NOT NULL,
    nonce           BLOB NOT NULL,
    kdf             TEXT NOT NULL,
    kdf_params      TEXT NOT NULL,
    pubkey_base58   TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    fetch_count_24h INTEGER NOT NULL DEFAULT 0,
    last_fetch_at   TEXT,
    FOREIGN KEY (google_sub) REFERENCES google_identity_links(google_sub) ON DELETE CASCADE
);";

// ── Helpers — DB ─────────────────────────────────────────────────────────────

/// Look up the `pubkey_base58` linked to `google_sub` in
/// `google_identity_links`. Returns `Ok(None)` when no row is present.
fn lookup_linked_pubkey(conn: &rusqlite::Connection, google_sub: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT pubkey_base58 FROM google_identity_links WHERE google_sub = ?1 LIMIT 1",
        params![google_sub],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .context("query google_identity_links for pubkey_base58")
}

/// Atomic UPSERT of the escrow row. Resets the rolling 24h window
/// (`fetch_count_24h=0`, `last_fetch_at=NULL`) on every PUT — the previous
/// counter is irrelevant once the user rewraps with a fresh ciphertext.
#[allow(clippy::too_many_arguments)]
fn upsert_escrow(
    conn: &rusqlite::Connection,
    google_sub: &str,
    ciphertext: &[u8],
    nonce: &[u8],
    kdf: &str,
    kdf_params: &str,
    pubkey_base58: &str,
    updated_at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO key_escrow_blobs \
         (google_sub, ciphertext, nonce, kdf, kdf_params, pubkey_base58, updated_at, fetch_count_24h, last_fetch_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL) \
         ON CONFLICT(google_sub) DO UPDATE SET \
             ciphertext = excluded.ciphertext, \
             nonce = excluded.nonce, \
             kdf = excluded.kdf, \
             kdf_params = excluded.kdf_params, \
             pubkey_base58 = excluded.pubkey_base58, \
             updated_at = excluded.updated_at, \
             fetch_count_24h = 0, \
             last_fetch_at = NULL",
        params![
            google_sub,
            ciphertext,
            nonce,
            kdf,
            kdf_params,
            pubkey_base58,
            updated_at,
        ],
    )
    .context("upsert key_escrow_blobs")?;
    Ok(())
}

/// One escrow row as held in memory after a successful GET.
#[derive(Debug, Clone)]
pub struct EscrowRow {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub kdf: String,
    pub kdf_params: String,
    pub pubkey_base58: String,
    pub fetch_count_24h: i64,
    pub last_fetch_at: Option<String>,
}

/// Read the escrow row for `google_sub`. Returns `Ok(None)` when absent.
fn read_escrow(conn: &rusqlite::Connection, google_sub: &str) -> Result<Option<EscrowRow>> {
    conn.query_row(
        "SELECT ciphertext, nonce, kdf, kdf_params, pubkey_base58, fetch_count_24h, last_fetch_at \
         FROM key_escrow_blobs WHERE google_sub = ?1 LIMIT 1",
        params![google_sub],
        |r| {
            Ok(EscrowRow {
                ciphertext: r.get(0)?,
                nonce: r.get(1)?,
                kdf: r.get(2)?,
                kdf_params: r.get(3)?,
                pubkey_base58: r.get(4)?,
                fetch_count_24h: r.get(5)?,
                last_fetch_at: r.get(6)?,
            })
        },
    )
    .optional()
    .context("read key_escrow_blobs row")
}

/// Atomic counter update for a GET fetch. Combines "set last_fetch_at" with
/// "increment or reset counter" in a single SQL statement so two concurrent
/// GETs cannot both observe count=N and both write count=N+1.
///
/// Logic (single UPDATE):
///   - If `last_fetch_at IS NULL` OR (`now` - `last_fetch_at`) >= 24h →
///     fetch_count_24h = 1.
///   - Else → fetch_count_24h = fetch_count_24h + 1.
///   - `last_fetch_at = now_rfc3339` in either branch.
///
/// We pass `now_unix` separately so SQLite computes the window-elapsed check
/// off integers — RFC3339 string subtraction is brittle.
fn increment_fetch_counter(
    conn: &rusqlite::Connection,
    google_sub: &str,
    now_rfc3339: &str,
    now_unix: i64,
    window_secs: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE key_escrow_blobs SET \
            fetch_count_24h = CASE \
                WHEN last_fetch_at IS NULL THEN 1 \
                WHEN (?2 - CAST(strftime('%s', last_fetch_at) AS INTEGER)) >= ?3 THEN 1 \
                ELSE fetch_count_24h + 1 \
            END, \
            last_fetch_at = ?4 \
         WHERE google_sub = ?1",
        params![google_sub, now_unix, window_secs, now_rfc3339],
    )
    .context("increment fetch_count_24h on key_escrow_blobs")?;
    Ok(())
}

/// Hard-delete the row for `google_sub`. Returns the number of rows removed
/// so the handler can answer 404 vs 200 cleanly.
fn delete_escrow(conn: &rusqlite::Connection, google_sub: &str) -> Result<usize> {
    let changes = conn
        .execute(
            "DELETE FROM key_escrow_blobs WHERE google_sub = ?1",
            params![google_sub],
        )
        .context("delete key_escrow_blobs row")?;
    Ok(changes)
}

// ── Bootstrap ticket store ───────────────────────────────────────────────────

/// One outstanding extension-bootstrap ticket. Held in the LRU until the
/// extension redeems it or the TTL expires.
#[derive(Debug, Clone)]
struct ExtensionBootstrapTicket {
    /// JWT subject (base58 pubkey) of the webapp caller that issued the
    /// ticket. Used both for the per-user cap and as the subject of the
    /// fresh `aud=extension` JWT minted at redeem time.
    jwt_sub: String,
    /// `google_sub` of the user (carried over from the issuing JWT). The
    /// redeem-time JWT carries this too — `/api/key-escrow` reads it.
    google_sub: Option<String>,
    /// Unix-seconds expiry. `consume` returns `None` after this.
    expires_at: i64,
}

/// In-memory LRU+TTL store of extension-bootstrap tickets. Same lock
/// discipline as `BootstrapTickets` in `api.rs` — `std::sync::Mutex` because
/// the critical sections are pure SQL/LRU work with no `.await` points.
pub struct ExtensionBootstrapTickets {
    inner: Mutex<ExtensionBootstrapInner>,
}

struct ExtensionBootstrapInner {
    lru: LruCache<uuid::Uuid, ExtensionBootstrapTicket>,
    per_user: std::collections::HashMap<String, usize>,
    per_user_cap: usize,
    ttl_seconds: i64,
}

impl ExtensionBootstrapInner {
    fn dec_user(&mut self, jwt_sub: &str) {
        if let Some(c) = self.per_user.get_mut(jwt_sub) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.per_user.remove(jwt_sub);
            }
        }
    }
}

/// Errors surfaced by `ExtensionBootstrapTickets::insert`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ExtensionBootstrapInsertError {
    PerUserCapExceeded,
}

impl ExtensionBootstrapTickets {
    pub fn new(lru_capacity: usize, per_user_cap: usize, ttl_seconds: i64) -> Self {
        let cap = NonZeroUsize::new(lru_capacity.max(1)).expect("nonzero capacity");
        Self {
            inner: Mutex::new(ExtensionBootstrapInner {
                lru: LruCache::new(cap),
                per_user: std::collections::HashMap::new(),
                per_user_cap,
                ttl_seconds,
            }),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(
            EXTENSION_BOOTSTRAP_LRU_CAP,
            EXTENSION_BOOTSTRAP_PER_USER_CAP,
            EXTENSION_BOOTSTRAP_TTL_SECS,
        )
    }

    fn insert(
        &self,
        jwt_sub: String,
        google_sub: Option<String>,
    ) -> std::result::Result<uuid::Uuid, ExtensionBootstrapInsertError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| ExtensionBootstrapInsertError::PerUserCapExceeded)?;

        // Sweep TTL-expired entries belonging to this jwt_sub BEFORE counting
        // toward the cap. Without this, a user who issues `per_user_cap`
        // tickets and lets them all expire (10-min TTL) without redeeming is
        // locked out of future bootstrap until the LRU evicts those entries —
        // which in practice requires 10_000+ unrelated tickets to be inserted
        // first. Security finding T15-M-02 (round 1).
        let now = now_unix();
        let expired_ids: Vec<uuid::Uuid> = guard
            .lru
            .iter()
            .filter(|(_id, t)| t.jwt_sub == jwt_sub && now >= t.expires_at)
            .map(|(id, _)| *id)
            .collect();
        for id in expired_ids {
            if let Some(t) = guard.lru.pop(&id) {
                guard.dec_user(&t.jwt_sub);
            }
        }

        let count = guard.per_user.get(&jwt_sub).copied().unwrap_or(0);
        if count >= guard.per_user_cap {
            return Err(ExtensionBootstrapInsertError::PerUserCapExceeded);
        }
        let ticket_id = uuid::Uuid::new_v4();
        let expires_at = now + guard.ttl_seconds;
        let entry = ExtensionBootstrapTicket {
            jwt_sub: jwt_sub.clone(),
            google_sub,
            expires_at,
        };
        if let Some((_evicted_id, evicted)) = guard.lru.push(ticket_id, entry) {
            // Defense: if the LRU evicts an unrelated entry (because cap was
            // reached), decrement THAT user's counter. The freshly-minted
            // UUID guarantees the eviction is not our own row.
            guard.dec_user(&evicted.jwt_sub);
        }
        *guard.per_user.entry(jwt_sub).or_insert(0) += 1;
        Ok(ticket_id)
    }

    /// Atomic remove-and-return. Returns the entry exactly once. Expired
    /// entries are evicted lazily and return None.
    fn consume(&self, ticket_id: uuid::Uuid) -> Option<ExtensionBootstrapTicket> {
        let mut guard = self.inner.lock().ok()?;
        let entry = guard.lru.pop(&ticket_id)?;
        let now = now_unix();
        if now >= entry.expires_at {
            guard.dec_user(&entry.jwt_sub);
            return None;
        }
        guard.dec_user(&entry.jwt_sub);
        Some(entry)
    }
}

// ── Escrow router state ──────────────────────────────────────────────────────

/// Shared state for the escrow + extension-bootstrap routes. Wraps the
/// rate-limit threshold + the bootstrap-ticket store + back-references to
/// the MCP and OAuth states (so handlers can read/write SQLite and verify
/// JWTs without holding any other mutable state).
pub struct EscrowState {
    pub rate_limit: u32,
    pub bootstrap_tickets: Arc<ExtensionBootstrapTickets>,
    pub mcp: Arc<McpState>,
    pub oauth: Arc<OAuthState>,
}

impl EscrowState {
    pub fn new(rate_limit: u32, mcp: Arc<McpState>, oauth: Arc<OAuthState>) -> Self {
        Self {
            rate_limit: rate_limit.max(1),
            bootstrap_tickets: Arc::new(ExtensionBootstrapTickets::with_defaults()),
            mcp,
            oauth,
        }
    }
}

// ── Handlers — extension-bootstrap ───────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ExtensionBootstrapIssueResponse {
    pub ticket_id: String,
}

/// `POST /api/extension-bootstrap/issue` — auth: server JWT (Bearer, aud=mcp,
/// optionally with `google_sub`). Returns a one-time UUID ticket the
/// extension can redeem at `/redeem/:ticket` for a fresh `aud=extension` JWT.
///
/// Auth is enforced by the global `bearer_auth_middleware` which injects
/// `Claims` into the request extension before this handler runs.
pub async fn extension_bootstrap_issue_handler(
    State(state): State<Arc<EscrowState>>,
    axum::Extension(claims): axum::Extension<Claims>,
) -> Response {
    match state
        .bootstrap_tickets
        .insert(claims.sub.clone(), claims.google_sub.clone())
    {
        Ok(ticket_id) => (
            StatusCode::OK,
            Json(ExtensionBootstrapIssueResponse {
                ticket_id: ticket_id.to_string(),
            }),
        )
            .into_response(),
        Err(ExtensionBootstrapInsertError::PerUserCapExceeded) => {
            // Fixed opaque string — do not leak the numeric per-user cap
            // constant into the response body. The server-side log carries
            // the cap value for operators (security finding T15-L-01).
            tracing::debug!(
                "extension-bootstrap per-user cap exceeded (cap={})",
                EXTENSION_BOOTSTRAP_PER_USER_CAP
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "error": "too_many_pending_tickets",
                })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExtensionBootstrapRedeemResponse {
    pub access_token: String,
    pub aud: &'static str,
    pub expires_in: u64,
}

/// `GET /api/extension-bootstrap/redeem/:ticket` — NO auth: the UUID is the
/// capability (single-use, 10-min TTL). Identical 404 body for "garbage UUID",
/// "expired", "already consumed" so an attacker probing tickets cannot
/// distinguish failure modes.
pub async fn extension_bootstrap_redeem_handler(
    State(state): State<Arc<EscrowState>>,
    Path(ticket_str): Path<String>,
) -> Response {
    let ticket_id = match uuid::Uuid::parse_str(&ticket_str) {
        Ok(id) => id,
        Err(_) => return extension_ticket_not_found(),
    };
    let entry = match state.bootstrap_tickets.consume(ticket_id) {
        Some(t) => t,
        None => return extension_ticket_not_found(),
    };

    // Mint a fresh JWT with aud=extension + the carried google_sub. We sign
    // it with the same secret as the production OAuth flow so all server
    // endpoints can verify it against the same key.
    let token = match mint_extension_jwt(&state.oauth, &entry.jwt_sub, entry.google_sub.clone()) {
        Ok(t) => t,
        Err(e) => return internal_error(&format!("mint extension JWT: {e}")),
    };
    (
        StatusCode::OK,
        Json(ExtensionBootstrapRedeemResponse {
            access_token: token,
            aud: EXTENSION_JWT_AUDIENCE,
            expires_in: JWT_TTL_SECS,
        }),
    )
        .into_response()
}

fn extension_ticket_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "ticket not found, already consumed, or expired",
        })),
    )
        .into_response()
}

// ── Handlers — key escrow ────────────────────────────────────────────────────

/// PUT body. Base64 transport keeps the wire format JSON-clean while the DB
/// holds raw BLOBs.
#[derive(Debug, Deserialize)]
pub struct KeyEscrowPutRequest {
    pub ciphertext_b64: String,
    pub nonce_b64: String,
    pub kdf: String,
    pub kdf_params: String,
    pub pubkey_base58: String,
}

/// GET response. Symmetric to PUT — base64 BLOBs + opaque KDF params + the
/// bound pubkey (so the extension can verify it matches its local keypair
/// before attempting decryption).
#[derive(Debug, Serialize)]
pub struct KeyEscrowGetResponse {
    pub ciphertext_b64: String,
    pub nonce_b64: String,
    pub kdf: String,
    pub kdf_params: String,
    pub pubkey_base58: String,
}

/// `PUT /api/key-escrow` — auth: extension JWT with `google_sub`. UPSERT the
/// blob; resets the rate-limit counter. Rejects 403 when:
///   - the JWT carries no `google_sub` claim, OR
///   - no `google_identity_links` row exists for the `google_sub`, OR
///   - the `pubkey_base58` in the request body does NOT match the linked
///     pubkey (security: closes the stolen-google-account → swap-key attack).
pub async fn key_escrow_put_handler(
    State(state): State<Arc<EscrowState>>,
    headers: HeaderMap,
    Json(req): Json<KeyEscrowPutRequest>,
) -> Response {
    let claims = match extract_extension_claims(&state.oauth, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };
    let google_sub = match claims.google_sub.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return unauthorized("JWT missing google_sub claim"),
    };

    // Decode base64 BLOBs first so we fail fast on garbage input before
    // touching the DB.
    let ciphertext =
        match base64::engine::general_purpose::STANDARD.decode(req.ciphertext_b64.as_bytes()) {
            Ok(b) => b,
            Err(_) => return bad_request("ciphertext_b64 is not valid base64"),
        };
    let nonce = match base64::engine::general_purpose::STANDARD.decode(req.nonce_b64.as_bytes()) {
        Ok(b) => b,
        Err(_) => return bad_request("nonce_b64 is not valid base64"),
    };
    if ciphertext.is_empty() {
        return bad_request("ciphertext_b64 must not be empty");
    }
    // AES-GCM-256 standard nonce is 96 bits = 12 bytes. We do not enforce
    // the wrap algorithm at the schema layer (D9 keeps the server agnostic),
    // but cap at 16 bytes to reject obviously-malformed nonces while still
    // accommodating alternative AEAD constructions (some use a 128-bit
    // nonce). 64-byte bound from round-1 was over-generous.
    if nonce.is_empty() || nonce.len() > 16 {
        return bad_request("nonce_b64 must be 1..=16 bytes after base64-decode");
    }
    if req.kdf.is_empty() || req.kdf.len() > 64 {
        return bad_request("kdf is required (1..=64 chars)");
    }
    if req.kdf_params.is_empty() || req.kdf_params.len() > 4096 {
        return bad_request("kdf_params is required (1..=4096 chars)");
    }
    // Ed25519 base58-encoded pubkeys are 43–44 chars. 64-byte upper bound
    // gives safe headroom while rejecting obvious storage-amplification
    // payloads (security finding T15-L-03).
    if req.pubkey_base58.is_empty() || req.pubkey_base58.len() > 64 {
        return bad_request("pubkey_base58 is required (1..=64 chars)");
    }

    // Lookup-then-upsert under one critical section so the binding check is
    // atomic w.r.t. concurrent `/oauth/google/link` calls flipping the
    // pubkey for the same `google_sub`.
    let updated_at = chrono::Utc::now().to_rfc3339();
    let result: Response = {
        let store = match state.mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        let conn = store.conn();
        let linked = match lookup_linked_pubkey(conn, &google_sub) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("key_escrow PUT: lookup_linked_pubkey: {e:#}");
                return internal_error("internal error");
            }
        };
        match linked {
            None => return forbidden("no google_identity_links row for google_sub"),
            Some(pk) if pk != req.pubkey_base58 => {
                return forbidden("pubkey_base58 does not match linked owner_pubkey")
            }
            Some(_) => {}
        }
        match upsert_escrow(
            conn,
            &google_sub,
            &ciphertext,
            &nonce,
            &req.kdf,
            &req.kdf_params,
            &req.pubkey_base58,
            &updated_at,
        ) {
            Ok(_) => Json(serde_json::json!({
                "status": "ok",
                "updated_at": updated_at,
            }))
            .into_response(),
            Err(e) => {
                tracing::error!("key_escrow PUT: upsert_escrow: {e:#}");
                internal_error("internal error")
            }
        }
    };
    result
}

/// `GET /api/key-escrow` — auth: extension JWT with `google_sub`. Returns
/// the escrow row if any. Enforces the rolling 24h rate limit by:
///   1. Reading the row.
///   2. If `last_fetch_at` is set AND the window has not elapsed AND the
///      counter is already >= rate_limit → return 429 + `Retry-After:
///      seconds_until_window_resets`.
///   3. Otherwise increment (atomically: the SQL UPDATE handles both
///      reset-on-elapsed and bump-in-window in one statement) and return
///      the body.
pub async fn key_escrow_get_handler(
    State(state): State<Arc<EscrowState>>,
    headers: HeaderMap,
) -> Response {
    let claims = match extract_extension_claims(&state.oauth, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };
    let google_sub = match claims.google_sub.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return unauthorized("JWT missing google_sub claim"),
    };

    let rate_limit = state.rate_limit;
    // Single Utc::now() so the RFC3339 string and the unix timestamp used in
    // the rate-limit check refer to exactly the same instant — eliminates a
    // sub-millisecond skew that would otherwise appear on a contended lock.
    let now = chrono::Utc::now();
    let now_rfc3339 = now.to_rfc3339();
    let now_unix = now.timestamp();

    // Single critical section: read → rate-check → increment → return.
    // No `.await` while the SQLite mutex is held.
    let row: EscrowRow = {
        let store = match state.mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        let conn = store.conn();
        let row = match read_escrow(conn, &google_sub) {
            Ok(Some(r)) => r,
            Ok(None) => return not_found("escrow not found"),
            Err(e) => {
                tracing::error!("key_escrow GET: read_escrow: {e:#}");
                return internal_error("internal error");
            }
        };

        // Compute how long ago the previous fetch was. None → no fetch yet
        // (counter is 0); Some(seconds) → seconds since last fetch.
        let elapsed_secs = match row.last_fetch_at.as_deref() {
            None => None,
            Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(dt) => Some(now_unix - dt.timestamp()),
                Err(_) => {
                    tracing::warn!("key_escrow GET: malformed last_fetch_at in DB, resetting");
                    None
                }
            },
        };

        // The counter only blocks when the window is NOT elapsed AND we are
        // at-or-over the cap. A fresh row (counter 0) or stale window
        // (elapsed >= 24h) is allowed through.
        let within_window = matches!(elapsed_secs, Some(e) if e < KEY_ESCROW_WINDOW_SECS);
        if within_window && row.fetch_count_24h as u32 >= rate_limit {
            let retry_after = KEY_ESCROW_WINDOW_SECS
                .saturating_sub(elapsed_secs.unwrap_or(0))
                .max(1);
            return rate_limited(retry_after);
        }

        if let Err(e) = increment_fetch_counter(
            conn,
            &google_sub,
            &now_rfc3339,
            now_unix,
            KEY_ESCROW_WINDOW_SECS,
        ) {
            tracing::error!("key_escrow GET: increment_fetch_counter: {e:#}");
            return internal_error("internal error");
        }
        row
    };

    Json(KeyEscrowGetResponse {
        ciphertext_b64: base64::engine::general_purpose::STANDARD.encode(&row.ciphertext),
        nonce_b64: base64::engine::general_purpose::STANDARD.encode(&row.nonce),
        kdf: row.kdf,
        kdf_params: row.kdf_params,
        pubkey_base58: row.pubkey_base58,
    })
    .into_response()
}

/// `DELETE /api/key-escrow` — auth: extension JWT with `google_sub`.
/// Removes the row. 200 on success (also when the row was already absent —
/// idempotent revocation).
pub async fn key_escrow_delete_handler(
    State(state): State<Arc<EscrowState>>,
    headers: HeaderMap,
) -> Response {
    let claims = match extract_extension_claims(&state.oauth, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };
    let google_sub = match claims.google_sub.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return unauthorized("JWT missing google_sub claim"),
    };

    let removed = {
        let store = match state.mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        match delete_escrow(store.conn(), &google_sub) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("key_escrow DELETE: {e:#}");
                return internal_error("internal error");
            }
        }
    };
    Json(serde_json::json!({"status": "ok", "removed": removed})).into_response()
}

// ── JWT helpers (aud=extension) ──────────────────────────────────────────────

/// Issue an HS256 JWT bound to `sub` + `google_sub` with `aud=extension`.
/// Shares the signing key with the production OAuth flow so a single
/// `MCP_JWT_SECRET` suffices.
pub fn mint_extension_jwt(
    oauth: &OAuthState,
    sub: &str,
    google_sub: Option<String>,
) -> Result<String> {
    let now = now_unix() as u64;
    let claims = Claims {
        iss: JWT_ISSUER.to_string(),
        aud: EXTENSION_JWT_AUDIENCE.to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + JWT_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
        google_sub,
    };
    let header = Header::new(Algorithm::HS256);
    encode(&header, &claims, oauth.jwt_encoding_key())
        .map_err(|e| anyhow!("JWT encode failed: {e}"))
}

/// Verify the Bearer JWT in `headers` and require `aud=extension`. The
/// production bearer-auth middleware accepts only `aud=mcp`, so the escrow
/// router intentionally runs OUTSIDE of it and verifies inline here.
fn extract_extension_claims(
    oauth: &OAuthState,
    headers: &HeaderMap,
) -> std::result::Result<Claims, Box<Response>> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Box::new(unauthorized("missing Bearer JWT")))?;

    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[EXTENSION_JWT_AUDIENCE]);
    let mut iss_set = std::collections::HashSet::new();
    iss_set.insert(JWT_ISSUER.to_string());
    validation.iss = Some(iss_set);
    validation.validate_exp = true;

    // jsonwebtoken's internal error strings (e.g. `InvalidSignature`,
    // `ExpiredSignature`, `InvalidAudience`, `InvalidIssuer`) are logged
    // server-side; clients receive a fixed `jwt_invalid` so they cannot
    // calibrate forgery attempts against the exact failure mode. Mirrors
    // the T14 round-2 fix for `id_token_invalid` in `oauth/google.rs`.
    let data =
        decode::<Claims>(token.as_str(), oauth.jwt_decoding_key(), &validation).map_err(|e| {
            tracing::warn!("escrow JWT validation failed: {e:#}");
            Box::new(unauthorized("jwt_invalid"))
        })?;
    if data.claims.iss != JWT_ISSUER || data.claims.aud != EXTENSION_JWT_AUDIENCE {
        tracing::warn!(
            "escrow JWT iss/aud mismatch: iss={:?} aud={:?}",
            data.claims.iss,
            data.claims.aud
        );
        return Err(Box::new(unauthorized("jwt_invalid")));
    }
    Ok(data.claims)
}

// ── Response helpers ─────────────────────────────────────────────────────────

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn forbidden(msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn rate_limited(retry_after_secs: i64) -> Response {
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({
            "error": "rate_limited",
            "retry_after_secs": retry_after_secs,
        })),
    )
        .into_response();
    // The header value is always a positive decimal integer, so this is
    // expected to succeed. If construction ever fails (e.g. negative
    // overflow into a non-ASCII string) we still return the 429 — but log
    // at warn! so the missing Retry-After is observable in production.
    match HeaderValue::from_str(&retry_after_secs.to_string()) {
        Ok(hv) => {
            resp.headers_mut().insert("Retry-After", hv);
        }
        Err(e) => {
            tracing::warn!("Retry-After header construction failed: {e}");
        }
    }
    resp
}

fn internal_error(msg: &str) -> Response {
    tracing::error!("key_escrow internal error: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal error"})),
    )
        .into_response()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
