//! Google OAuth provider — Decision 5 of `work/chrome-extension/decisions.md`.
//!
//! Adds a second OAuth path alongside the existing Solana-wallet flow in
//! `oauth/mod.rs`. The Chrome extension's `chrome.identity.launchWebAuthFlow`
//! drives the user through Google consent; the server validates the returned
//! `id_token` against Google's cached JWKS, then mints OUR server-issued
//! authorization code via the same single-use LRU as the Solana-wallet flow.
//! The extension exchanges the code at `/oauth/token` with PKCE; the
//! resulting JWT carries an extra `google_sub` claim.
//!
//! ## Endpoints
//!
//! - `GET /oauth/google/start?client_id=...&redirect_uri=...&code_challenge=...&state=...`
//!   — validates inputs, stores PKCE state under a server-generated key, and
//!   302-redirects the user-agent to `https://accounts.google.com/o/oauth2/v2/auth`
//!   with our own redirect URI (`GOOGLE_OAUTH_REDIRECT_URI`).
//!
//! - `GET /oauth/google/callback?code=...&state=...` — exchanges Google's
//!   code for tokens at `https://oauth2.googleapis.com/token`, validates
//!   `id_token` (issuer, audience, RS256 signature, expiry), mints a server
//!   `code` bound to the PKCE challenge + `google_sub`, and 302-redirects the
//!   extension's original `redirect_uri` with `?code=<server-code>&state=<state>`.
//!
//! - `POST /oauth/google/lookup` (Bearer JWT with `google_sub`) — returns
//!   `{existing_pubkey: Option<String>, escrow_present: bool, link_challenge: Option<String>}`.
//!   `link_challenge` is populated only when no existing link is present —
//!   the extension passes it back as a possession proof at `/link`.
//!
//! - `POST /oauth/google/link` (Bearer JWT with `google_sub`, no existing
//!   link) — body `{pubkey_base58, possession_proof_base64, challenge}`.
//!   The server re-derives the expected challenge from the LRU (popping it
//!   atomically for single-use), verifies the Ed25519 signature, and inserts
//!   a row into `google_identity_links`.
//!
//! ## Security
//!
//! - `id_token` issuer must be `accounts.google.com` OR
//!   `https://accounts.google.com`; audience must equal
//!   `GOOGLE_OAUTH_CLIENT_ID`; expiry must be in the future; RS256
//!   signature is verified against the JWKS cache (`google_jwks.rs`).
//! - PKCE is **S256-only** — `code_challenge_method != S256` → 400.
//! - Possession-proof nonce is server-issued, single-use, 5-min TTL.
//! - Rate limit: 5 lookup OR link calls per 24h per `google_sub` (D5/D9).
//! - Redirect URIs are HTTPS-only unless the host is `localhost` /
//!   `127.0.0.1` / `[::1]` (dev). The Chrome extension's
//!   `https://<extid>.chromiumapp.org/...` deeplink is HTTPS by design.
//! - If `GOOGLE_OAUTH_CLIENT_ID` is unset, the handlers return 404 — the
//!   feature is disabled at runtime without panic.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use lru::LruCache;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use super::google_jwks::GoogleJwksCache;
use super::{issue_jwt_with_google_sub, verify_jwt, OAuthState};
use crate::mcp::McpState;

// ── Constants ────────────────────────────────────────────────────────────────

/// PKCE pending-state TTL — 10 minutes. The Google consent screen + user
/// interaction takes up to several minutes; 10 min gives slack without
/// indefinitely-lived state.
pub const GOOGLE_STATE_TTL_SECS: u64 = 600;

/// LRU capacity for in-flight Google OAuth start→callback state.
pub const GOOGLE_STATE_LRU_CAP: usize = 10_000;

/// Possession-proof nonce TTL — 5 minutes per the security checklist.
pub const LINK_CHALLENGE_TTL_SECS: u64 = 300;

/// LRU capacity for outstanding link challenges (keyed by google_sub).
pub const LINK_CHALLENGE_LRU_CAP: usize = 10_000;

/// Rate-limit window for `/oauth/google/lookup` + `/oauth/google/link`:
/// 5 calls per 24h per `google_sub`.
pub const GOOGLE_RATE_LIMIT_PER_24H: u32 = 5;
pub const GOOGLE_RATE_WINDOW_SECS: i64 = 24 * 3600;

/// Default Google authorization endpoint. Test harness overrides via
/// `GoogleOAuthState::with_endpoints`.
pub const DEFAULT_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Default Google token endpoint.
pub const DEFAULT_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google OAuth scopes — minimum needed for `id_token` with `sub` + email.
pub const GOOGLE_SCOPES: &str = "openid email profile";

// ── State ────────────────────────────────────────────────────────────────────

/// Per-flow PKCE binding inserted at `start` and consumed at `callback`.
#[derive(Debug, Clone)]
struct PendingGoogleFlow {
    /// PKCE `code_challenge` supplied by the extension. Carried forward into
    /// the issued server-code so `/oauth/token` can verify the
    /// `code_verifier` against it.
    code_challenge: String,
    /// Original CSRF `state` supplied by the extension. Echoed back on the
    /// redirect to the extension's `redirect_uri`.
    client_state: String,
    /// Extension's redirect URI (the `chrome.identity.launchWebAuthFlow`
    /// callback URL). Validated at start-time against HTTPS + dev-loopback.
    redirect_uri: String,
    /// Client id supplied by the extension (informational; not strictly
    /// validated beyond non-empty since we trust the bearer-auth-bound
    /// `redirect_uri` allowlist).
    #[allow(dead_code)]
    client_id: String,
    /// Unix-seconds expiry.
    exp: u64,
}

/// Per-google_sub outstanding link challenge. Stored at `lookup` (when no
/// existing link), consumed by `link` exactly once. Single-use,
/// time-bounded.
#[derive(Debug, Clone)]
struct LinkChallenge {
    /// Raw 32 bytes of nonce (base64-encoded for the client wire format).
    nonce: [u8; 32],
    /// Unix-seconds expiry.
    exp: u64,
}

/// Per-google_sub rate counter for lookup + link calls. Window auto-resets
/// after 24h.
#[derive(Debug, Clone, Default)]
struct RateCounter {
    /// Calls in the current window.
    count: u32,
    /// Window start (unix seconds).
    window_start: i64,
}

/// In-memory shared state for the Google OAuth flow. Wrapped in `Arc`,
/// injected into the router via `with_state`.
pub struct GoogleOAuthState {
    /// `GOOGLE_OAUTH_CLIENT_ID` — Google-side public client id. Empty string
    /// means "Google OAuth is disabled" (router wiring in `main.rs` checks
    /// this and skips installation entirely).
    pub client_id: String,
    /// `GOOGLE_OAUTH_CLIENT_SECRET` — held server-side only, never sent to
    /// the extension. Used in the `oauth2.googleapis.com/token` exchange.
    pub client_secret: String,
    /// `GOOGLE_OAUTH_REDIRECT_URI` — the URI Google redirects back to after
    /// consent. Must be exactly the URI configured in Google Console.
    pub redirect_uri: String,
    /// Authorization endpoint (overridable for tests).
    pub auth_url: String,
    /// Token endpoint (overridable for tests).
    pub token_url: String,
    /// JWKS verifier for the `id_token`.
    pub jwks: Arc<GoogleJwksCache>,
    pending_flows: Mutex<LruCache<String, PendingGoogleFlow>>,
    link_challenges: Mutex<LruCache<String, LinkChallenge>>,
    rate_counters: Mutex<HashMap<String, RateCounter>>,
    /// HTTP client for the Google token-exchange call.
    http: reqwest::Client,
}

impl GoogleOAuthState {
    /// Build state from configured env vars. Empty `client_id` means the
    /// feature is disabled — the caller (`main.rs`) checks before wiring
    /// any routes.
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        Self::with_endpoints(
            client_id,
            client_secret,
            redirect_uri,
            DEFAULT_AUTH_URL.to_string(),
            DEFAULT_TOKEN_URL.to_string(),
            Arc::new(GoogleJwksCache::new()),
        )
    }

    /// Build state with custom endpoints + JWKS cache — used by integration
    /// tests to inject mock Google servers.
    pub fn with_endpoints(
        client_id: String,
        client_secret: String,
        redirect_uri: String,
        auth_url: String,
        token_url: String,
        jwks: Arc<GoogleJwksCache>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("build reqwest client for Google OAuth");
        let cap_flows = NonZeroUsize::new(GOOGLE_STATE_LRU_CAP).expect("nonzero LRU capacity");
        let cap_challenges =
            NonZeroUsize::new(LINK_CHALLENGE_LRU_CAP).expect("nonzero challenge LRU capacity");
        Self {
            client_id,
            client_secret,
            redirect_uri,
            auth_url,
            token_url,
            jwks,
            pending_flows: Mutex::new(LruCache::new(cap_flows)),
            link_challenges: Mutex::new(LruCache::new(cap_challenges)),
            rate_counters: Mutex::new(HashMap::new()),
            http,
        }
    }

    /// True when env vars were not configured — handlers should return 404.
    pub fn is_disabled(&self) -> bool {
        self.client_id.is_empty()
    }

    /// Insert a new pending-flow entry and return the LRU key (= the
    /// `state` we hand to Google). We use a fresh UUID so the key cannot
    /// be forged by the client; the extension's CSRF `state` is stored as
    /// the `client_state` field and echoed back at callback time.
    fn insert_pending_flow(&self, entry: PendingGoogleFlow) -> Result<String> {
        let key = uuid::Uuid::new_v4().to_string();
        let mut guard = self
            .pending_flows
            .lock()
            .map_err(|_| anyhow!("pending_flows mutex poisoned"))?;
        guard.put(key.clone(), entry);
        Ok(key)
    }

    /// Atomically pop a pending-flow entry by its `state` key.
    fn pop_pending_flow(&self, state: &str) -> Result<Option<PendingGoogleFlow>> {
        let mut guard = self
            .pending_flows
            .lock()
            .map_err(|_| anyhow!("pending_flows mutex poisoned"))?;
        Ok(guard.pop(state))
    }

    /// Issue (and store) a fresh link challenge for `google_sub`. Returns
    /// the base64-encoded nonce.
    fn issue_link_challenge(&self, google_sub: &str) -> Result<String> {
        use rand::RngCore;
        let mut nonce = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let exp = now_secs() + LINK_CHALLENGE_TTL_SECS;
        let challenge = LinkChallenge { nonce, exp };
        let mut guard = self
            .link_challenges
            .lock()
            .map_err(|_| anyhow!("link_challenges mutex poisoned"))?;
        guard.put(google_sub.to_string(), challenge);
        Ok(base64::engine::general_purpose::STANDARD.encode(nonce))
    }

    /// Atomically pop the link challenge for `google_sub`.
    fn pop_link_challenge(&self, google_sub: &str) -> Result<Option<LinkChallenge>> {
        let mut guard = self
            .link_challenges
            .lock()
            .map_err(|_| anyhow!("link_challenges mutex poisoned"))?;
        Ok(guard.pop(google_sub))
    }

    /// Increment the per-google_sub rate counter. Returns true if the call
    /// is allowed, false if the rate limit is exceeded.
    fn rate_check(&self, google_sub: &str) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let mut guard = self
            .rate_counters
            .lock()
            .map_err(|_| anyhow!("rate_counters mutex poisoned"))?;
        let entry = guard.entry(google_sub.to_string()).or_default();
        if now - entry.window_start >= GOOGLE_RATE_WINDOW_SECS {
            entry.window_start = now;
            entry.count = 0;
        }
        if entry.count >= GOOGLE_RATE_LIMIT_PER_24H {
            return Ok(false);
        }
        entry.count += 1;
        Ok(true)
    }
}

// ── DB migration ─────────────────────────────────────────────────────────────

/// Create the `google_identity_links` table if it does not exist.
///
/// Schema (per task T14 spec):
///   - `google_sub TEXT PRIMARY KEY` — opaque Google account id.
///   - `pubkey_base58 TEXT NOT NULL` — Ed25519 user pubkey linked to it.
///   - `linked_at INTEGER NOT NULL` — unix seconds at which the link was minted.
///
/// Idempotent — safe to call on every `McpState` boot. Lives in `mcp/`
/// because Decision 9 places OAuth + escrow tables in the server layer, not
/// in `core/src/storage/sqlite.rs` (which is reserved for the cross-client
/// attestation schema).
pub fn migrate_google_identity_links(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS google_identity_links (
            google_sub    TEXT PRIMARY KEY,
            pubkey_base58 TEXT NOT NULL,
            linked_at     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_google_identity_links_pubkey
            ON google_identity_links(pubkey_base58);",
    )
    .context("create google_identity_links table")?;
    Ok(())
}

/// Look up the linked pubkey for `google_sub` (if any).
fn lookup_link(conn: &rusqlite::Connection, google_sub: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT pubkey_base58 FROM google_identity_links WHERE google_sub = ?1 LIMIT 1")
        .context("prepare google_identity_links lookup")?;
    let mut rows = stmt
        .query(params![google_sub])
        .context("query google_identity_links")?;
    if let Some(row) = rows.next().context("step google_identity_links")? {
        let pk: String = row.get(0).context("read pubkey_base58")?;
        Ok(Some(pk))
    } else {
        Ok(None)
    }
}

/// Insert a fresh link. Errors if the row already exists (caller checked).
fn insert_link(
    conn: &rusqlite::Connection,
    google_sub: &str,
    pubkey_base58: &str,
    linked_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO google_identity_links (google_sub, pubkey_base58, linked_at) \
         VALUES (?1, ?2, ?3)",
        params![google_sub, pubkey_base58, linked_at],
    )
    .context("insert google_identity_links row")?;
    Ok(())
}

/// Whether a `key_escrow_blobs` row exists for `google_sub`. The table is
/// created by T15; if it doesn't exist yet we treat it as "no escrow".
fn escrow_present(conn: &rusqlite::Connection, google_sub: &str) -> Result<bool> {
    // Detect whether the table exists yet (T15 may not have run). sqlite_master
    // is the standard discovery path.
    let table_present: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='key_escrow_blobs' LIMIT 1",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !table_present {
        return Ok(false);
    }
    let mut stmt = conn
        .prepare("SELECT 1 FROM key_escrow_blobs WHERE google_sub = ?1 LIMIT 1")
        .context("prepare key_escrow_blobs lookup")?;
    let mut rows = stmt
        .query(params![google_sub])
        .context("query key_escrow_blobs")?;
    Ok(rows.next().context("step key_escrow_blobs")?.is_some())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /oauth/google/start` query parameters.
#[derive(Debug, Deserialize)]
pub struct GoogleStartQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    /// PKCE method — must be `S256`. Optional in serde but required at runtime.
    #[serde(default = "default_pkce_method")]
    pub code_challenge_method: String,
    pub state: String,
}

fn default_pkce_method() -> String {
    "S256".to_string()
}

/// `GET /oauth/google/start` — validates inputs, stores the PKCE flow under a
/// server-generated `state`, redirects to Google with our own `redirect_uri`.
///
/// Takes the same tuple state as `google_callback_handler` so axum's router
/// can group them. The other fields are unused on this path.
pub async fn google_start_handler(
    State((_mcp, _oauth, google)): State<(Arc<McpState>, Arc<OAuthState>, Arc<GoogleOAuthState>)>,
    Query(q): Query<GoogleStartQuery>,
) -> Response {
    if google.is_disabled() {
        return not_found_disabled();
    }
    if q.code_challenge_method != "S256" {
        return bad_request("code_challenge_method must be S256");
    }
    if q.client_id.is_empty()
        || q.redirect_uri.is_empty()
        || q.code_challenge.is_empty()
        || q.state.is_empty()
    {
        return bad_request("client_id, redirect_uri, code_challenge, and state are required");
    }
    if !is_safe_redirect(&q.redirect_uri) {
        return bad_request("redirect_uri must be HTTPS or loopback");
    }

    let exp = now_secs() + GOOGLE_STATE_TTL_SECS;
    let entry = PendingGoogleFlow {
        code_challenge: q.code_challenge.clone(),
        client_state: q.state.clone(),
        redirect_uri: q.redirect_uri.clone(),
        client_id: q.client_id.clone(),
        exp,
    };

    let server_state = match google.insert_pending_flow(entry) {
        Ok(k) => k,
        Err(e) => return internal_error(&format!("store pending flow: {e}")),
    };

    let google_url = format!(
        "{auth}?{params}",
        auth = google.auth_url,
        params = build_query(&[
            ("client_id", &google.client_id),
            ("redirect_uri", &google.redirect_uri),
            ("response_type", "code"),
            ("scope", GOOGLE_SCOPES),
            ("state", &server_state),
            ("access_type", "online"),
            ("prompt", "select_account"),
        ])
    );

    redirect_to(&google_url)
}

/// `GET /oauth/google/callback` query parameters. `error` is populated when
/// Google denies consent — we forward the failure to the extension's
/// `redirect_uri` so its UI can present it.
#[derive(Debug, Deserialize)]
pub struct GoogleCallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Google token endpoint response shape.
#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    #[allow(dead_code)]
    access_token: Option<String>,
    id_token: Option<String>,
}

/// `GET /oauth/google/callback` — exchanges Google's `code` for tokens,
/// validates `id_token`, mints a server `code`, redirects.
pub async fn google_callback_handler(
    State((mcp, oauth, google)): State<(Arc<McpState>, Arc<OAuthState>, Arc<GoogleOAuthState>)>,
    Query(q): Query<GoogleCallbackQuery>,
) -> Response {
    if google.is_disabled() {
        return not_found_disabled();
    }

    let state_val = match q.state {
        Some(s) if !s.is_empty() => s,
        _ => return bad_request("state is required"),
    };

    let pending = match google.pop_pending_flow(&state_val) {
        Ok(Some(p)) => p,
        Ok(None) => return bad_request("unknown or already-used state"),
        Err(e) => return internal_error(&format!("pop pending flow: {e}")),
    };
    if now_secs() > pending.exp {
        return bad_request("state expired");
    }

    if let Some(err) = q.error.as_deref() {
        // Google rejected consent — surface to the extension via its
        // redirect_uri so the UI can present "Google declined".
        let to = format!(
            "{cb}?{params}",
            cb = pending.redirect_uri,
            params = build_query(&[("error", err), ("state", &pending.client_state),])
        );
        return redirect_to(&to);
    }

    let code = match q.code {
        Some(c) if !c.is_empty() => c,
        _ => return bad_request("code is required"),
    };

    // Exchange Google code for tokens.
    let token_resp = match exchange_google_code(&google, &code).await {
        Ok(t) => t,
        Err(e) => return bad_gateway(&format!("Google token exchange failed: {e}")),
    };

    let id_token = match token_resp.id_token {
        Some(t) if !t.is_empty() => t,
        _ => return bad_gateway("Google response missing id_token"),
    };

    // Verify id_token against cached JWKS (RS256 only, aud + iss + exp).
    let claims = match google
        .jwks
        .verify_id_token(&id_token, &google.client_id)
        .await
    {
        Ok(c) => c,
        Err(e) => return unauthorized(&format!("id_token verification failed: {e}")),
    };

    // Resolve existing link if any so the JWT's `sub` is the user's Ed25519
    // pubkey (the protocol identity), with `google_sub` as the link key.
    let existing_pubkey = {
        let store = match mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        match lookup_link(store.conn(), &claims.sub) {
            Ok(x) => x,
            Err(e) => return internal_error(&format!("lookup_link: {e}")),
        }
    };
    let jwt_sub = match existing_pubkey {
        Some(pk) => pk,
        // First-touch: use the google_sub as the JWT subject. The extension
        // then immediately POSTs /oauth/google/link with the pubkey + sig.
        None => format!("google:{}", claims.sub),
    };

    let server_code = oauth.mint_issued_code(
        jwt_sub,
        pending.code_challenge.clone(),
        pending.redirect_uri.clone(),
        Some(claims.sub.clone()),
    );

    let to = format!(
        "{cb}?{params}",
        cb = pending.redirect_uri,
        params = build_query(&[("code", &server_code), ("state", &pending.client_state),])
    );
    redirect_to(&to)
}

/// `POST /oauth/google/lookup` — auth-required (JWT with `google_sub`).
/// Returns the current state of the binding, plus a fresh link-challenge
/// nonce when no existing link is present.
#[derive(Debug, Serialize)]
pub struct LookupResponse {
    pub existing_pubkey: Option<String>,
    pub escrow_present: bool,
    /// Base64-encoded nonce — present only when `existing_pubkey` is None,
    /// to be passed back at `/link` as the possession proof's message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_challenge: Option<String>,
}

pub async fn google_lookup_handler(
    State((state, oauth, google)): State<(Arc<McpState>, Arc<OAuthState>, Arc<GoogleOAuthState>)>,
    headers: HeaderMap,
) -> Response {
    if google.is_disabled() {
        return not_found_disabled();
    }
    let claims = match extract_bearer_claims(&oauth, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };
    let google_sub = match claims.google_sub.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return unauthorized("JWT missing google_sub claim"),
    };
    match google.rate_check(&google_sub) {
        Ok(true) => {}
        Ok(false) => return too_many_requests(),
        Err(e) => return internal_error(&format!("rate check: {e}")),
    }

    let (existing_pubkey, escrow) = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        let conn = store.conn();
        let pk = match lookup_link(conn, &google_sub) {
            Ok(x) => x,
            Err(e) => return internal_error(&format!("lookup_link: {e}")),
        };
        let esc = match escrow_present(conn, &google_sub) {
            Ok(x) => x,
            Err(e) => return internal_error(&format!("escrow_present: {e}")),
        };
        (pk, esc)
    };

    let link_challenge = if existing_pubkey.is_none() {
        match google.issue_link_challenge(&google_sub) {
            Ok(n) => Some(n),
            Err(e) => return internal_error(&format!("issue link challenge: {e}")),
        }
    } else {
        None
    };

    Json(LookupResponse {
        existing_pubkey,
        escrow_present: escrow,
        link_challenge,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct LinkRequest {
    pub pubkey_base58: String,
    /// Base64-encoded Ed25519 signature over the raw bytes of the nonce
    /// previously returned in `LookupResponse.link_challenge`.
    pub possession_proof_base64: String,
    /// The base64-encoded challenge string the client received. Server
    /// re-decodes + compares against the stored nonce for the user.
    pub challenge: String,
}

pub async fn google_link_handler(
    State((mcp, oauth, google)): State<(Arc<McpState>, Arc<OAuthState>, Arc<GoogleOAuthState>)>,
    headers: HeaderMap,
    Json(req): Json<LinkRequest>,
) -> Response {
    if google.is_disabled() {
        return not_found_disabled();
    }
    let claims = match extract_bearer_claims(&oauth, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };
    let google_sub = match claims.google_sub.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return unauthorized("JWT missing google_sub claim"),
    };
    match google.rate_check(&google_sub) {
        Ok(true) => {}
        Ok(false) => return too_many_requests(),
        Err(e) => return internal_error(&format!("rate check: {e}")),
    }

    // Validate the pubkey is a real Ed25519 base58 pubkey.
    let pubkey: Pubkey = match req.pubkey_base58.parse() {
        Ok(pk) => pk,
        Err(_) => return bad_request("pubkey_base58 is not a valid Ed25519 base58 pubkey"),
    };

    // Reject if a link already exists — first-link only per task spec.
    {
        let store = match mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        let conn = store.conn();
        match lookup_link(conn, &google_sub) {
            Ok(Some(_)) => return conflict("google_sub already linked"),
            Ok(None) => {}
            Err(e) => return internal_error(&format!("lookup_link: {e}")),
        }
    }

    // Pop the outstanding challenge (single-use, atomic).
    let challenge = match google.pop_link_challenge(&google_sub) {
        Ok(Some(c)) => c,
        Ok(None) => return unauthorized("no outstanding link challenge — call /lookup first"),
        Err(e) => return internal_error(&format!("pop link challenge: {e}")),
    };
    if now_secs() > challenge.exp {
        return unauthorized("link challenge expired");
    }

    // Client must echo back the SAME challenge bytes — defense against a
    // mismatched-nonce attack where the client signs an unrelated value.
    let supplied_nonce =
        match base64::engine::general_purpose::STANDARD.decode(req.challenge.as_bytes()) {
            Ok(b) => b,
            Err(_) => return bad_request("challenge is not valid base64"),
        };
    if supplied_nonce != challenge.nonce {
        return unauthorized("challenge does not match server-issued nonce");
    }

    // Decode + length-check the signature.
    let sig_bytes = match base64::engine::general_purpose::STANDARD
        .decode(req.possession_proof_base64.as_bytes())
    {
        Ok(b) => b,
        Err(_) => return bad_request("possession_proof_base64 is not valid base64"),
    };
    if sig_bytes.len() != 64 {
        return bad_request("possession_proof must be 64 bytes (Ed25519 signature)");
    }

    if !mnemonic_core::identity::verify_signature(&pubkey, &challenge.nonce, &sig_bytes) {
        return unauthorized("Ed25519 signature invalid");
    }

    // Insert the row. Use parameterized SQL only (no string concat).
    let linked_at = chrono::Utc::now().timestamp();
    {
        let store = match mcp.store.lock() {
            Ok(g) => g,
            Err(_) => return internal_error("store mutex poisoned"),
        };
        let conn = store.conn();
        if let Err(e) = insert_link(conn, &google_sub, &req.pubkey_base58, linked_at) {
            return internal_error(&format!("insert_link: {e}"));
        }
    }

    // Mint a fresh JWT bound to the newly-linked Ed25519 pubkey. The
    // extension uses this in place of the bootstrap token it minted via the
    // Google callback path. We always have an `OAuthState` because the
    // router only mounts this handler when JWT signing is configured.
    let _ = &pubkey; // pubkey validated above; bytes are also embedded in the row
    let fresh_token =
        match issue_jwt_with_google_sub(&oauth, &req.pubkey_base58, Some(google_sub.clone())) {
            Ok(t) => t,
            Err(e) => return internal_error(&format!("JWT mint failed: {e}")),
        };

    Json(serde_json::json!({
        "status": "ok",
        "google_sub": google_sub,
        "pubkey_base58": req.pubkey_base58,
        "linked_at": linked_at,
        "access_token": fresh_token,
    }))
    .into_response()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract + verify a Bearer JWT from the Authorization header. The
/// `bearer_auth_middleware` in `oauth/mod.rs` is URI-allowlisted for
/// `/oauth/*` paths (so it never blocks the OAuth surface), which means the
/// Google lookup/link handlers cannot rely on `Extension<Claims>` being set.
/// We do the verification inline here.
///
/// Returns the response BOXED on the Err arm — `axum::Response` is ~256
/// bytes which trips `clippy::result_large_err`. Callers `?`-propagate.
fn extract_bearer_claims(
    oauth: &OAuthState,
    headers: &HeaderMap,
) -> std::result::Result<super::Claims, Box<Response>> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Box::new(unauthorized("missing Bearer JWT")))?;
    verify_jwt(oauth, &token).map_err(|e| Box::new(unauthorized(&format!("invalid JWT: {e}"))))
}

async fn exchange_google_code(
    google: &GoogleOAuthState,
    code: &str,
) -> Result<GoogleTokenResponse> {
    // RFC 6749 §4.1.3 form-urlencoded exchange. Google requires
    // `application/x-www-form-urlencoded`.
    let params = [
        ("code", code),
        ("client_id", google.client_id.as_str()),
        ("client_secret", google.client_secret.as_str()),
        ("redirect_uri", google.redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let resp = google
        .http
        .post(&google.token_url)
        .form(&params)
        .send()
        .await
        .context("POST Google token endpoint")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Google token endpoint returned {status}: {body}"));
    }
    let parsed: GoogleTokenResponse = resp
        .json()
        .await
        .context("parse Google token response JSON")?;
    Ok(parsed)
}

fn build_query(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&url_encode(k));
        out.push('=');
        out.push_str(&url_encode(v));
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Allow HTTPS redirects and loopback dev URIs only. Chrome extension
/// `https://<extid>.chromiumapp.org/` is HTTPS by design and matches.
fn is_safe_redirect(uri: &str) -> bool {
    if uri.starts_with("https://") {
        return true;
    }
    // Allow loopback (developer convenience) — HTTPS is mandatory for any
    // other host. `http://localhost:`, `http://127.0.0.1:`, `http://[::1]:`.
    if uri.starts_with("http://localhost")
        || uri.starts_with("http://127.0.0.1")
        || uri.starts_with("http://[::1]")
    {
        return true;
    }
    false
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn redirect_to(location: &str) -> Response {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::FOUND;
    if let Ok(hv) = HeaderValue::from_str(location) {
        resp.headers_mut().insert(axum::http::header::LOCATION, hv);
    }
    resp
}

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

fn conflict(msg: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn bad_gateway(msg: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn too_many_requests() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({"error": "rate limit exceeded: 5 calls / 24h / google_sub"})),
    )
        .into_response()
}

fn internal_error(msg: &str) -> Response {
    // Mask details from the client; log server-side for operator triage.
    tracing::error!("google oauth handler internal error: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal error"})),
    )
        .into_response()
}

fn not_found_disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "Google OAuth is not configured"})),
    )
        .into_response()
}
