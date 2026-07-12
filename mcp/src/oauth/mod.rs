//! OAuth 2.1 + PKCE server module + Bearer-auth middleware.
//!
//! Implements Decisions 9, 10, 11 of the mnemonic-integrations tech-spec:
//!
//!   - **Decision 9** — Bearer-auth allowlist (`/oauth/*`, `/health`, JSON-RPC
//!     `initialize` and `tools/list`); everything else needs a valid HS256
//!     JWT or returns HTTP 401 with a JSON-RPC error envelope.
//!   - **Decision 10** — `/oauth/authorize` validates a COSE_Sign1-signed
//!     canonical-CBOR challenge (`{server_origin, state, client_id,
//!     redirect_uri, code_challenge, code_challenge_method, nonce, exp}`).
//!     S256-only PKCE; single-use atomic state eviction.
//!   - **Decision 11** — JWT is HS256, secret loaded from `MCP_JWT_SECRET`
//!     env var (32-byte base64), claims `iss="mcp.mnemonik.xyz"`, `aud="mcp"`,
//!     `sub=<base58 pubkey>`, `iat`, `exp` (now+3600s), `jti=Uuid::new_v4()`.
//!     `verify_jwt` uses `Validation::new(Algorithm::HS256)` (NOT
//!     `Validation::default()`) and validates `iss` + `aud`. `alg=none` and
//!     non-HS256 tokens are rejected at the protocol layer.
//!
//! State is in-memory only (LRU 10k entries, TTL 60s). Server restart loses
//! pending OAuth codes — acceptable for the hackathon demo (Phase 1 scope).
//!
//! Architectural rule: `core/` MUST NOT reference anything in this module.
//! All OAuth concerns live in `mcp/`. Reuses `mnemonic_core::codec::canonical`
//! and `mnemonic_core::codec::sign::verify_artifact` for the challenge
//! signature check (Decision 10) — the same primitives as COSE attestations.

// ── Google OAuth submodules (chrome-extension T14, Decision 5) ───────────────
// Sibling files under `mcp/src/oauth/`. Disabled at runtime when
// `GOOGLE_OAUTH_CLIENT_ID` is unset (handlers return 404).
pub mod google;
pub mod google_jwks;
pub mod refresh;

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use lru::LruCache;
use mnemonic_core::codec::{canonical::to_canonical_cbor, hash::hash_bytes as blake3_hex};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// JWT issuer — must match the production server origin.
pub const JWT_ISSUER: &str = "mcp.mnemonik.xyz";
/// JWT audience — fixed, validated on every verify.
pub const JWT_AUDIENCE: &str = "mcp";
/// JWT TTL default (1 hour, Decision 11) — applied when `MCP_JWT_TTL_SECS`
/// is unset, unparseable, empty, or whitespace. `pub(crate)`: only the unit
/// tests in this module and [`jwt_ttl_secs`] read it; no external consumer.
pub(crate) const JWT_TTL_DEFAULT_SECS: u64 = 3600;
/// Lower clamp on `MCP_JWT_TTL_SECS` (Decision 12 — refresh-token-rotation
/// tech-spec). 60s is the minimum usable for the R1 pre-ship empirical gate
/// (Task 10) and avoids the "MCP_JWT_TTL_SECS=1" operator footgun.
pub(crate) const JWT_TTL_MIN_SECS: u64 = 60;
/// Upper clamp on `MCP_JWT_TTL_SECS` (Decision 12). 7 days — a soft cap
/// that keeps long-lived tokens off the wire without forbidding legitimate
/// extended sessions.
pub(crate) const JWT_TTL_MAX_SECS: u64 = 604_800;

/// Process-global JWT TTL, seeded once at startup by [`seed_jwt_ttl_from_env`]
/// (called inside `main::run_http`). Decision 12 mandates `std::sync::OnceLock`
/// — do NOT swap in `once_cell::sync::OnceCell` (that crate is not in the
/// project's dependency surface).
static JWT_TTL: OnceLock<u64> = OnceLock::new();

/// Returns the effective JWT access-token TTL in seconds. Reads the
/// `OnceLock` seeded by [`seed_jwt_ttl_from_env`]; if the lock was never
/// seeded (e.g. stdio transport, which skips `run_http`, or a test that
/// imported this module without booting the HTTP stack) falls back to
/// [`JWT_TTL_DEFAULT_SECS`] so every reader is safe pre-seed.
pub fn jwt_ttl_secs() -> u64 {
    *JWT_TTL.get().unwrap_or(&JWT_TTL_DEFAULT_SECS)
}

/// Seeds [`JWT_TTL`] from the `MCP_JWT_TTL_SECS` env var (clamp + parse
/// failure behaviour documented on [`compute_jwt_ttl_from_env_str`]). Idempotent
/// — second calls silently lose the race against the first-write so test
/// harnesses can re-invoke the seed without panicking.
///
/// The INFO log only fires when *this* call actually populated the lock
/// (CR2-R1-M1). A re-seed that lost the race emits a DEBUG line stating the
/// observed (already-stored) value so logs cannot lie about what
/// `jwt_ttl_secs()` will return next. Neither log echoes the raw env-var
/// string (SA2-R1-M1) — only the resolved numeric TTL — so an operator
/// who accidentally pastes `MCP_JWT_SECRET` into `MCP_JWT_TTL_SECS` does
/// not leak it through log aggregation. The Decision-14-style WARN logs
/// in [`compute_jwt_ttl_from_env_str`] that DO need to surface the raw
/// value (parse failure) bound it to the first 16 chars.
pub fn seed_jwt_ttl_from_env() {
    let raw = std::env::var("MCP_JWT_TTL_SECS").ok();
    let mut newly_seeded_value: Option<u64> = None;
    let stored = *JWT_TTL.get_or_init(|| {
        let v = compute_jwt_ttl_from_env_str(raw.as_deref());
        newly_seeded_value = Some(v);
        v
    });
    if let Some(value) = newly_seeded_value {
        tracing::info!(
            target: "mnemonic_mcp::oauth",
            "JWT access-token TTL seeded to {value}s"
        );
    } else {
        tracing::debug!(
            target: "mnemonic_mcp::oauth",
            "JWT access-token TTL already seeded to {stored}s; re-seed call ignored"
        );
    }
}

/// Truncates `s` to at most `max_chars` Unicode characters, appending an
/// ellipsis when truncation occurred (SA2-R1-M1). Char-boundary-safe — we
/// take `max_chars` from the `chars()` iterator, NOT byte slices, so a
/// multi-byte UTF-8 sequence in attacker input cannot trigger a slice
/// panic. Use this whenever a WARN log needs to echo operator-supplied
/// raw input from an env var.
fn bound_log_value(s: &str, max_chars: usize) -> String {
    let mut iter = s.chars();
    let head: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Pure helper — parses an optional `MCP_JWT_TTL_SECS` string, applies the
/// clamp `[JWT_TTL_MIN_SECS, JWT_TTL_MAX_SECS]`, emits a WARN on
/// clamp / parse failure, and returns the resolved value. Public to the
/// module so `seed_jwt_ttl_from_env` and the unit tests share one
/// codepath — the tests exercise this helper directly so they do NOT
/// pollute the process-global `OnceLock` (test-isolation hint from the
/// task spec).
///
/// Defaults to [`JWT_TTL_DEFAULT_SECS`] on `None`, empty, whitespace, or
/// parse failure. The non-silent WARN closes the silent-default vector
/// for the Task 10 R1 gate (a deploy-typo must be loud).
pub(crate) fn compute_jwt_ttl_from_env_str(raw: Option<&str>) -> u64 {
    let Some(s) = raw else {
        return JWT_TTL_DEFAULT_SECS;
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_JWT_TTL_SECS is empty / whitespace-only; falling back to default {JWT_TTL_DEFAULT_SECS}s"
        );
        return JWT_TTL_DEFAULT_SECS;
    }
    match trimmed.parse::<u64>() {
        Ok(parsed) => {
            if parsed < JWT_TTL_MIN_SECS {
                tracing::warn!(
                    target: "mnemonic_mcp::oauth",
                    "MCP_JWT_TTL_SECS={parsed} below minimum {JWT_TTL_MIN_SECS}s; clamped to {JWT_TTL_MIN_SECS}"
                );
                JWT_TTL_MIN_SECS
            } else if parsed > JWT_TTL_MAX_SECS {
                tracing::warn!(
                    target: "mnemonic_mcp::oauth",
                    "MCP_JWT_TTL_SECS={parsed} above maximum {JWT_TTL_MAX_SECS}s; clamped to {JWT_TTL_MAX_SECS}"
                );
                JWT_TTL_MAX_SECS
            } else {
                parsed
            }
        }
        Err(e) => {
            // Bound the echoed raw value to 16 chars so an attacker- or
            // operator-supplied massive string cannot bloat log aggregation
            // (SA2-R1-M1). Char-boundary-safe slice. Truncation is
            // marked with a trailing ellipsis so the original length is
            // discoverable.
            let bounded = bound_log_value(trimmed, 16);
            tracing::warn!(
                target: "mnemonic_mcp::oauth",
                "MCP_JWT_TTL_SECS={bounded:?} failed to parse as u64 ({e}); falling back to default {JWT_TTL_DEFAULT_SECS}s"
            );
            JWT_TTL_DEFAULT_SECS
        }
    }
}
/// Pending-state TTL: 60s per Decision 10. Consumed by the consent-page
/// bootstrap endpoint (`GET /oauth/authorize`) when inserting a fresh challenge.
pub const STATE_TTL_SECS: u64 = 60;
/// Issued-code TTL: 60s — code must be redeemed at /oauth/token within this.
pub const CODE_TTL_SECS: u64 = 60;
/// LRU bound on both pending-state and issued-code maps.
pub const OAUTH_STATE_CAPACITY: usize = 10_000;
/// Default server origin baked into the binary — used when
/// `MCP_PUBLIC_BASE_URL` is unset. Matches the production hostname so the
/// no-env-var deploy path is byte-identical to the legacy `pub const
/// SERVER_ORIGIN` it replaced. `pub(crate)` — same visibility as
/// [`JWT_TTL_DEFAULT_SECS`]; the test module + [`server_origin`] are the
/// only readers, and exposing it as `pub` would mislead consumers into
/// treating the const as the public API instead of [`server_origin`]
/// (CR-R1-MINOR-3).
pub(crate) const SERVER_ORIGIN_DEFAULT: &str = "https://mcp.mnemonik.xyz";

/// Process-global server origin, seeded once at startup by
/// [`seed_server_origin_from_env`] (called inside `main::run_http`). Reads
/// `MCP_PUBLIC_BASE_URL`; falls back to [`SERVER_ORIGIN_DEFAULT`] when unset
/// or malformed. `std::sync::OnceLock` mirrors the [`JWT_TTL`] pattern — no
/// external `once_cell` dependency.
static SERVER_ORIGIN: OnceLock<String> = OnceLock::new();

/// Returns the effective server origin advertised in OAuth metadata, the
/// canonical-CBOR challenge envelope, and the `/.well-known/*` documents.
/// Falls back to [`SERVER_ORIGIN_DEFAULT`] when the `OnceLock` was never
/// seeded (stdio transport, tests that import this module without booting
/// the HTTP stack) so every reader is safe pre-seed.
pub fn server_origin() -> &'static str {
    SERVER_ORIGIN
        .get()
        .map(String::as_str)
        .unwrap_or(SERVER_ORIGIN_DEFAULT)
}

/// Seeds [`SERVER_ORIGIN`] from the `MCP_PUBLIC_BASE_URL` env var. Idempotent
/// — second calls silently lose the race against the first-write so test
/// harnesses can re-invoke the seed without panicking.
///
/// The validation rules mirror [`compute_jwt_ttl_from_env_str`]'s
/// loud-failure contract: malformed values emit a WARN log and fall back to
/// [`SERVER_ORIGIN_DEFAULT`] rather than aborting boot, so an operator typo
/// on the live deploy does not take the server down. A wholly-unset env var
/// silently uses the default — only explicit misconfiguration is loud.
pub fn seed_server_origin_from_env() {
    let raw = std::env::var("MCP_PUBLIC_BASE_URL").ok();
    let mut newly_seeded_value: Option<String> = None;
    let stored = SERVER_ORIGIN.get_or_init(|| {
        let v = compute_server_origin_from_env_str(raw.as_deref());
        newly_seeded_value = Some(v.clone());
        v
    });
    // Server origins are public URLs, not credentials — we intentionally
    // log the full validated value (unlike `seed_jwt_ttl_from_env`, which
    // logs only the resolved numeric TTL because that env var could be
    // accidentally swapped with `MCP_JWT_SECRET`). Validation in
    // `compute_server_origin_from_env_str` already rejects values
    // containing credentials (userinfo `@`) and control chars (CRLF
    // injection vectors), so what reaches this log is guaranteed to be
    // a well-formed `scheme://authority` URL with no secret-bearing
    // characters (SA-R1-M1, SA-R1-M2, CR-R1-MINOR-1).
    if let Some(value) = newly_seeded_value {
        tracing::info!(
            target: "mnemonic_mcp::oauth",
            "server origin seeded to {value}"
        );
    } else {
        tracing::debug!(
            target: "mnemonic_mcp::oauth",
            "server origin already seeded to {stored}; re-seed call ignored"
        );
    }
}

/// Pure helper — validates an optional `MCP_PUBLIC_BASE_URL` string and
/// returns the resolved origin. Public to the module so
/// [`seed_server_origin_from_env`] and the unit tests share one codepath;
/// the tests exercise this helper directly so they do NOT pollute the
/// process-global `OnceLock`.
///
/// Defaults to [`SERVER_ORIGIN_DEFAULT`] on `None`, empty, whitespace, or
/// any of the validation failures listed below. The non-silent WARN on
/// malformed values closes the silent-default vector for the Task 10 R1
/// gate (a deploy-typo must be loud).
///
/// Rejects (WARN + default):
///   - empty / whitespace-only
///   - missing `http://` or `https://` scheme prefix
///   - any ASCII control character (\r, \n, \t, NUL, etc.) — defense-in-depth
///     against CRLF header-injection if the value is ever interpolated into
///     a raw HTTP header string outside `axum::http::HeaderValue::from_str`'s
///     guard (SA-R1-M2)
///   - userinfo (`@` in the authority) — closes both the credential-leak
///     vector via the seed INFO log and the metadata pollution that would
///     redirect OAuth clients to `https://user:pass@origin/oauth/authorize`
///     (SA-R1-M1, CWE-532)
///   - trailing `/` (would double-slash when concatenated with endpoint paths)
///   - any `?` query or `#` fragment
///   - a path beyond the host:port (e.g. `https://host/sub`) — OAuth
///     metadata advertises endpoint URLs as `{origin}/oauth/authorize` etc.,
///     a base with a path would garble the resulting URL
pub(crate) fn compute_server_origin_from_env_str(raw: Option<&str>) -> String {
    let Some(s) = raw else {
        return SERVER_ORIGIN_DEFAULT.to_string();
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL is empty / whitespace-only; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    let bounded = bound_log_value(trimmed, 64);
    // ASCII control-char rejection (SA-R1-M2). Must come BEFORE any
    // strip_prefix matching: a value like `\rhttps://...` would otherwise
    // fall through to the "missing scheme" WARN — correct outcome, but the
    // WARN message would mislead the operator. The control-char branch
    // surfaces the actual problem.
    if trimmed.chars().any(|c| c.is_ascii_control()) {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} contains ASCII control character; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    let rest = if let Some(r) = trimmed.strip_prefix("https://") {
        r
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        r
    } else {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} missing http:// or https:// scheme; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    };
    if rest.is_empty() {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} has scheme but no host; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    // Userinfo rejection (SA-R1-M1). `https://user:pass@evil.com` would
    // otherwise be reflected verbatim into OAuth metadata and INFO-logged at
    // seed time — leaking the credential and redirecting clients through the
    // attacker-controlled credentialed URL. RFC 8707 resource indicators do
    // not use userinfo, so this is a pure operator-footgun cleanup.
    if rest.contains('@') {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} contains userinfo (@) in authority; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    if trimmed.ends_with('/') {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} has trailing slash; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} contains query or fragment; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    // Reject a path component past the host:port. Splitting on the first
    // `/` after the scheme isolates the authority; anything after it is a
    // path, which would corrupt the metadata URL concatenations.
    if rest.contains('/') {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "MCP_PUBLIC_BASE_URL={bounded:?} has path component beyond host; falling back to default {SERVER_ORIGIN_DEFAULT}"
        );
        return SERVER_ORIGIN_DEFAULT.to_string();
    }
    trimmed.to_string()
}
/// Frontend webapp origin — the consent page lives here. The bootstrap
/// endpoint `GET /oauth/authorize` redirects the user-agent (browser) to
/// `WEBAPP_CONSENT_URL?challenge=<base64-cbor>&state=<state>&mcp_base=<origin>`
/// so the WASM signer can post the approval back to the server that created
/// the pending OAuth state.
pub const WEBAPP_CONSENT_URL: &str = "https://www.mnemonik.xyz/oauth/consent";

/// JWT claim set per Decision 11.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Issuer — fixed `mcp.mnemonik.xyz`.
    pub iss: String,
    /// Audience — fixed `mcp`.
    pub aud: String,
    /// Subject — base58 user pubkey.
    pub sub: String,
    /// Issued at (unix seconds).
    pub iat: u64,
    /// Expiry (unix seconds).
    pub exp: u64,
    /// JWT ID — UUIDv4, unique per token.
    pub jti: String,
    /// Google account `sub` claim — populated when the JWT was issued via the
    /// Google OAuth provider (`/oauth/google/callback` → `/oauth/token`).
    /// Absent for the original Solana-wallet OAuth path (Decision 10/11).
    /// `serde(skip_serializing_if = "Option::is_none")` keeps existing tokens
    /// byte-identical on the wire — the field appears only when populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_sub: Option<String>,
}

/// Pending authorize record (state → expected pubkey + challenge metadata).
#[derive(Debug, Clone)]
struct PendingAuthorize {
    /// blake3 hex of canonical_cbor(challenge_fields). Kept for debug/log
    /// continuity; raw Ed25519 verification uses `challenge_bytes` directly.
    #[allow(dead_code)]
    challenge_hash: String,
    /// Canonical-CBOR bytes of the challenge map. The webapp signs THESE
    /// bytes with WASM `sign_challenge` (raw Ed25519); the server verifies
    /// the signature against them via `identity::verify_signature`.
    /// Storing the bytes (not just the hash) avoids the COSE_Sign1
    /// round-trip — webapp `sign_challenge` returns a 64-byte signature,
    /// not a COSE envelope.
    challenge_bytes: Vec<u8>,
    /// The base58 pubkey expected to sign — encoded into the challenge fields.
    expected_pubkey: String,
    /// `redirect_uri` from the original authorize request.
    redirect_uri: String,
    /// Request-supplied PKCE `code_challenge` (S256). Stored so /token can
    /// verify the `code_verifier` against `SHA256(verifier) == challenge`.
    code_challenge: String,
    /// Original CSRF state from the client (echoed back on success).
    client_state: String,
    /// Unix-seconds expiry — rejected if `now() > exp`.
    exp: u64,
}

/// Issued authorization code (token → resolved pubkey + PKCE binding).
#[derive(Debug, Clone)]
struct IssuedCode {
    /// Resolved user pubkey (will become JWT.sub).
    sub: String,
    /// PKCE code_challenge (S256) — verifier supplied at /token must hash to this.
    code_challenge: String,
    /// `redirect_uri` from the original authorize request. Bound at code-issue
    /// time per RFC 7636 §4.4 (PKCE) + RFC 6749 §4.1.3 (auth-code redirect_uri
    /// binding). The `/oauth/token` exchange compares the supplied redirect_uri
    /// (when present) against this value and rejects mismatches with 400 —
    /// closes a swap-redirect attack where an attacker who guessed `code` would
    /// otherwise drive the JWT to an attacker-controlled callback.
    redirect_uri: String,
    /// Google account `sub` claim — populated only for codes minted by the
    /// Google OAuth callback path (T14). The `/oauth/token` exchange copies
    /// this onto the issued JWT's `google_sub` claim so downstream
    /// `/oauth/google/lookup` + `/oauth/google/link` handlers can read it.
    google_sub: Option<String>,
    /// Unix-seconds expiry.
    exp: u64,
}

/// Dynamic Client Registration record (client_id -> allowed redirect URIs).
#[derive(Debug, Clone)]
struct RegisteredClient {
    redirect_uris: Vec<String>,
}

/// Shared OAuth state — pending challenges + issued codes, both LRU+TTL bound.
/// Wrap in `Arc` and inject into Axum routers via `with_state`.
///
/// Decision 6 (refresh-token-rotation): the OAuth subsystem owns its own
/// physical `rusqlite::Connection` on the same SQLite file as `McpState.store`.
/// Two Connections on one file under WAL mode is well-defined — SQLite's
/// writer lock serialises concurrent writes — and avoids a McpState refactor.
pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
    clients: Mutex<LruCache<String, RegisteredClient>>,
    /// HS256 signing key. Constructed once from `MCP_JWT_SECRET` at startup.
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
    // ── Refresh-token rotation surface (Decision 6, Task 3) ──────────────────
    /// Second physical `rusqlite::Connection` on the same SQLite file as
    /// `McpState.store`. Holds the `refresh_tokens` table; wrapped in
    /// `Arc<Mutex<_>>` because rusqlite::Connection is `!Send` and is read
    /// from async handlers. Migrated + PRAGMA-configured at construction.
    pub refresh_store: Arc<Mutex<Connection>>,
    /// Per-deploy salt for `blake3(salt || plaintext)` at-rest hashing of
    /// refresh tokens. 32+ bytes, generated via `openssl rand -base64 32`,
    /// gated at boot by `validate_refresh_salt`. Immutable for the deploy
    /// lifetime — rotating invalidates every live refresh token.
    pub refresh_salt: Vec<u8>,
    /// Reuse-interval (D4 — Auth0 default 5s). Wrapped legitimate retries
    /// inside this window get the cached `(access_jwt, refresh_plaintext)`
    /// pair from `reuse_cache` (Branch B), preventing a network retry from
    /// looking like a token-reuse attack (Branch B').
    ///
    /// Stored on the state for observability + future-test invariants —
    /// the value is also encoded into `reuse_cache.ttl` at construction
    /// time, so logic does not need to read this field. `#[allow(dead_code)]`
    /// keeps the field in the surface while we satisfy `-D warnings`.
    #[allow(dead_code)]
    pub reuse_interval: Duration,
    /// Background evictor tick period (D9 — production default 1h).
    /// Consumed by `refresh::start_evictor` at boot time.
    pub evictor_tick: Duration,
    /// In-process pair cache backing D5 reuse-interval idempotency.
    /// Capacity and TTL are wired through from `OAuthState::new`'s
    /// `reuse_cache_cap` argument (round-2 m6 — the cap is no longer
    /// silently defaulted).
    pub reuse_cache: Arc<refresh::ReuseCache>,
}

impl OAuthState {
    /// Build OAuthState from a 32-byte base64-decoded secret plus the
    /// refresh-token subsystem configuration. Panics if the secret is
    /// shorter than 32 bytes — caller (main.rs) must reject startup in
    /// that case rather than allowing a weak HMAC key.
    ///
    /// Opens a SECOND physical `rusqlite::Connection` on `database_path`
    /// (NOT a clone of `McpState.store`'s connection — Decision 6: two
    /// connections on one file under WAL is the canonical pattern;
    /// `payment.rs`'s `Arc<Mutex<Connection>>` field is the precedent).
    /// Applies the OAuth-side PRAGMAs (`foreign_keys=ON` + WAL +
    /// `busy_timeout=5000`) and the `refresh_tokens` migration before
    /// returning, so the connection is fully ready for `refresh::rotate`
    /// callers.
    pub fn new(
        secret: &[u8],
        database_path: &Path,
        refresh_salt: Vec<u8>,
        reuse_interval: Duration,
        evictor_tick: Duration,
        reuse_cache_cap: usize,
    ) -> anyhow::Result<Self> {
        assert!(
            secret.len() >= 32,
            "MCP_JWT_SECRET must decode to >= 32 bytes (got {})",
            secret.len()
        );
        assert!(
            refresh_salt.len() >= 32,
            "refresh_salt must be >= 32 bytes (got {}) — validate via validate_refresh_salt before constructing OAuthState",
            refresh_salt.len()
        );
        let cap = NonZeroUsize::new(OAUTH_STATE_CAPACITY).expect("nonzero capacity");
        // D6: open OAuth-side physical connection, apply OAuth PRAGMAs +
        // migrate refresh_tokens table. `migrate_refresh_tokens` already
        // calls `apply_oauth_connection_pragmas` first, so a single call
        // is sufficient.
        let conn = Connection::open(database_path).map_err(|e| {
            anyhow::anyhow!(
                "OAuthState: open refresh-token SQLite connection at {}: {e}",
                database_path.display()
            )
        })?;
        refresh::migrate_refresh_tokens(&conn)?;
        let reuse_cache_cap_nz = NonZeroUsize::new(reuse_cache_cap)
            .ok_or_else(|| anyhow::anyhow!("reuse_cache_cap must be non-zero (got 0)"))?;
        let reuse_cache = Arc::new(refresh::ReuseCache::with_cap_and_ttl(
            reuse_cache_cap_nz,
            reuse_interval,
        ));
        Ok(Self {
            pending: Mutex::new(LruCache::new(cap)),
            codes: Mutex::new(LruCache::new(cap)),
            clients: Mutex::new(LruCache::new(cap)),
            jwt_encoding_key: EncodingKey::from_secret(secret),
            jwt_decoding_key: DecodingKey::from_secret(secret),
            refresh_store: Arc::new(Mutex::new(conn)),
            refresh_salt,
            reuse_interval,
            evictor_tick,
            reuse_cache,
        })
    }

    /// Test-only constructor: in-memory SQLite + fixed test salt + fast
    /// reuse/evictor intervals + default reuse-cache capacity. Every test
    /// call site that previously did `OAuthState::new(TEST_SECRET)` switches
    /// to `OAuthState::with_defaults(TEST_SECRET)` for a one-line diff.
    ///
    /// **NOT for production wiring** — uses an in-memory SQLite database
    /// (every refresh-token row vanishes on process exit) and a fixed
    /// all-`0xAB` salt that an attacker could trivially recompute. The
    /// production binary (`main.rs::run_http`) always uses
    /// `OAuthState::new` with the deploy-time `MCP_REFRESH_SALT`. Kept
    /// outside `#[cfg(test)]` because every integration test crate under
    /// `mcp/tests/*.rs` consumes it via the library facade, and gating
    /// it on `feature = "test-support"` would force ~14 unrelated test
    /// crates to opt in — out of scope for Task 3.
    ///
    /// `#[allow(dead_code)]` — the bin (`mnemonic-mcp`) target never
    /// reaches this; the warning would otherwise fire under `-D warnings`.
    #[allow(dead_code)]
    pub fn with_defaults(secret: &[u8]) -> Self {
        assert!(
            secret.len() >= 32,
            "MCP_JWT_SECRET must decode to >= 32 bytes (got {})",
            secret.len()
        );
        let cap = NonZeroUsize::new(OAUTH_STATE_CAPACITY).expect("nonzero capacity");
        // In-memory SQLite — keeps each test isolated; never touches disk.
        let conn =
            Connection::open_in_memory().expect("OAuthState::with_defaults: open in-memory SQLite");
        refresh::migrate_refresh_tokens(&conn)
            .expect("OAuthState::with_defaults: migrate refresh_tokens table");
        let refresh_salt: Vec<u8> = vec![0xABu8; 32];
        let reuse_interval = Duration::from_millis(100);
        let evictor_tick = Duration::from_millis(50);
        let reuse_cache = Arc::new(refresh::ReuseCache::with_cap_and_ttl(
            refresh::DEFAULT_REUSE_CACHE_CAP_NZ,
            reuse_interval,
        ));
        Self {
            pending: Mutex::new(LruCache::new(cap)),
            codes: Mutex::new(LruCache::new(cap)),
            clients: Mutex::new(LruCache::new(cap)),
            jwt_encoding_key: EncodingKey::from_secret(secret),
            jwt_decoding_key: DecodingKey::from_secret(secret),
            refresh_store: Arc::new(Mutex::new(conn)),
            refresh_salt,
            reuse_interval,
            evictor_tick,
            reuse_cache,
        }
    }

    /// Insert a pending authorize record before issuing the challenge to the
    /// browser. Called by the consent-page bootstrap (`GET /oauth/authorize`)
    /// in `authorize_init_handler` and by unit tests in this module.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_pending(
        &self,
        state: String,
        challenge_hash: String,
        challenge_bytes: Vec<u8>,
        expected_pubkey: String,
        redirect_uri: String,
        code_challenge: String,
        client_state: String,
        exp: u64,
    ) {
        let entry = PendingAuthorize {
            challenge_hash,
            challenge_bytes,
            expected_pubkey,
            redirect_uri,
            code_challenge,
            client_state,
            exp,
        };
        let mut guard = self.pending.lock().expect("pending mutex poisoned");
        guard.put(state, entry);
    }

    /// Persist Dynamic Client Registration redirects for the issued client_id.
    pub fn register_client(&self, client_id: String, redirect_uris: Vec<String>) {
        let mut guard = self.clients.lock().expect("clients mutex poisoned");
        guard.put(client_id, RegisteredClient { redirect_uris });
    }

    /// Mint a one-time authorization code bound to `sub` + `redirect_uri` +
    /// `code_challenge` (PKCE S256) with an optional `google_sub` claim. The
    /// caller (Google OAuth callback) returns the code string to the user-
    /// agent in a redirect; the extension then exchanges it at `/oauth/token`
    /// with the original `code_verifier`. Single-use — `/oauth/token` pops the
    /// entry atomically.
    ///
    /// Returns the generated code (UUIDv4).
    pub fn mint_issued_code(
        &self,
        sub: String,
        code_challenge: String,
        redirect_uri: String,
        google_sub: Option<String>,
    ) -> String {
        let code = uuid::Uuid::new_v4().to_string();
        let entry = IssuedCode {
            sub,
            code_challenge,
            redirect_uri,
            google_sub,
            exp: now_secs() + CODE_TTL_SECS,
        };
        let mut guard = self.codes.lock().expect("codes mutex poisoned");
        guard.put(code.clone(), entry);
        code
    }

    /// Accessor for the HS256 encoding key. Used by the extension key-escrow
    /// module (T15) to mint `aud=extension` JWTs signed by the same secret as
    /// the main `aud=mcp` flow — a single `MCP_JWT_SECRET` suffices for both.
    pub fn jwt_encoding_key(&self) -> &EncodingKey {
        &self.jwt_encoding_key
    }

    /// Accessor for the HS256 decoding key. Used by the extension key-escrow
    /// module (T15) to verify `aud=extension` JWTs against the same secret.
    pub fn jwt_decoding_key(&self) -> &DecodingKey {
        &self.jwt_decoding_key
    }

    /// Validate redirect_uri against DCR first, then the static fallback list.
    pub fn allows_redirect(&self, uri: &str, client_id: &str) -> bool {
        if self.registered_redirect_allowed(uri, client_id) {
            return true;
        }
        allowed_redirect(uri, client_id)
    }

    fn registered_redirect_allowed(&self, uri: &str, client_id: &str) -> bool {
        let mut guard = self.clients.lock().expect("clients mutex poisoned");
        guard
            .get(client_id)
            .map(|client| {
                client
                    .redirect_uris
                    .iter()
                    .any(|registered| registered_redirect_matches(registered, uri))
            })
            .unwrap_or(false)
    }
}

/// Compute the blake3-hex of canonical_cbor over the Decision-10 challenge
/// field set. Reused by the consent-page bootstrap (server side) AND the
/// browser's WASM signer (which independently runs the same encoder).
///
/// Field order matches the canonical-CBOR output: keys are sorted
/// alphabetically by `to_canonical_cbor` because we pass them as a JSON
/// object (no `cbor_field_order` schema). This deterministic ordering is the
/// security property that closes the length-extension / delimiter ambiguity
/// attack mentioned in Decision 10.
#[allow(clippy::too_many_arguments)]
pub fn build_challenge_hash(
    server_origin: &str,
    state: &str,
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    code_challenge_method: &str,
    nonce: &str,
    exp: u64,
) -> Result<String, String> {
    let challenge = serde_json::json!({
        "server_origin": server_origin,
        "state": state,
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "code_challenge": code_challenge,
        "code_challenge_method": code_challenge_method,
        "nonce": nonce,
        "exp": exp,
    });
    // Use a synthetic schema that lists every field — we want them all,
    // sorted by name (canonical CBOR sorts map keys; the schema's
    // `cbor_field_order` is just an explicit override, but here we can use
    // a minimal schema and let the alphabetical fallback in `to_canonical_cbor`'s
    // nested-object path do the work).
    let bytes = to_canonical_cbor(&challenge, &CHALLENGE_SCHEMA)
        .map_err(|e| format!("canonical CBOR encode failed: {e}"))?;
    Ok(blake3_hex(&bytes))
}

/// Schema for the OAuth challenge envelope. The `cbor_field_order` array
/// drives `to_canonical_cbor`'s top-level field order — listing every field
/// explicitly here pins the byte layout against future schema additions.
static CHALLENGE_SCHEMA: mnemonic_core::codec::schema::ArtifactSchema =
    mnemonic_core::codec::schema::ArtifactSchema {
        artifact_type: mnemonic_core::codec::schema::ArtifactType::Receipt, // unused — the schema is consumed only for `cbor_field_order`
        version: 1,
        required_fields: &[
            "client_id",
            "code_challenge",
            "code_challenge_method",
            "exp",
            "nonce",
            "redirect_uri",
            "server_origin",
            "state",
        ],
        optional_fields: &[],
        // Alphabetical — matches the canonical-CBOR sorted-key output.
        cbor_field_order: &[
            "client_id",
            "code_challenge",
            "code_challenge_method",
            "exp",
            "nonce",
            "redirect_uri",
            "server_origin",
            "state",
        ],
    };

/// Current unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── JWT issuance + verification (Decision 11) ────────────────────────────────

/// Issue an HS256 JWT bound to `sub` (base58 user pubkey). Convenience wrapper
/// around `issue_jwt_with_google_sub(state, sub, None)` for the legacy
/// Solana-wallet OAuth path (Decision 10/11).
///
/// Marked `allow(dead_code)` because the binary build path uses
/// `issue_jwt_with_google_sub` directly; this thin wrapper is reachable from
/// integration tests under `mcp/tests/*.rs` via the library facade in
/// `lib.rs`. Removing it would cascade through ~12 test files.
#[allow(dead_code)]
pub fn issue_jwt(state: &OAuthState, sub: &str) -> Result<String, String> {
    issue_jwt_with_google_sub(state, sub, None)
}

/// Issue an HS256 JWT bound to `sub` with an optional `google_sub` claim
/// populated. Used by the Google OAuth path (T14) to carry the Google
/// account identifier alongside the user's Ed25519 pubkey.
pub fn issue_jwt_with_google_sub(
    state: &OAuthState,
    sub: &str,
    google_sub: Option<String>,
) -> Result<String, String> {
    let now = now_secs();
    let claims = Claims {
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + jwt_ttl_secs(),
        jti: uuid::Uuid::new_v4().to_string(),
        google_sub,
    };
    let header = Header::new(Algorithm::HS256);
    encode(&header, &claims, &state.jwt_encoding_key).map_err(|e| format!("JWT encode failed: {e}"))
}

/// Verify an HS256 JWT. Rejects `alg=none`, RS256, mismatched `iss`/`aud`,
/// and expired tokens. Returns the decoded claims on success.
pub fn verify_jwt(state: &OAuthState, token: &str) -> Result<Claims, String> {
    // Construct Validation with a FIXED algorithm — `Validation::default()`
    // accepts multiple algorithms which would let an attacker submit an
    // RS256-signed token verified against the HMAC secret as a public key.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[JWT_AUDIENCE]);
    let mut iss_set = HashSet::new();
    iss_set.insert(JWT_ISSUER.to_string());
    validation.iss = Some(iss_set);
    validation.validate_exp = true;

    let data = decode::<Claims>(token, &state.jwt_decoding_key, &validation)
        .map_err(|e| format!("JWT verify failed: {e}"))?;
    // Defense in depth — `Validation` already checks these, but assert to
    // surface any future jsonwebtoken behavior change loudly.
    if data.claims.iss != JWT_ISSUER {
        return Err(format!("unexpected iss: {}", data.claims.iss));
    }
    if data.claims.aud != JWT_AUDIENCE {
        return Err(format!("unexpected aud: {}", data.claims.aud));
    }
    Ok(data.claims)
}

// ── /oauth/authorize-init handler (Decision 10 bootstrap) ───────────────────

/// Query parameters for `GET /oauth/authorize` (the bootstrap endpoint).
/// Standard OAuth 2.1 + PKCE shape. `pubkey` is a Mnemonic-specific extension
/// — the base58 user pubkey from localStorage, supplied by the webapp consent
/// page when it has identity available; absent on the very first hop from
/// Cursor/Claude.ai when the user-agent has not yet been redirected to
/// `mnemonik.xyz/oauth/consent`.
#[derive(Debug, Deserialize)]
pub struct AuthorizeInitQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    /// Optional response_type — accepted but only `code` is honored.
    #[serde(default)]
    pub response_type: Option<String>,
    /// Base58 user pubkey from webapp localStorage. When present we insert
    /// the pending challenge immediately and return JSON to the caller (or
    /// redirect to the consent page with the challenge embedded). When
    /// absent we redirect to the consent page so the webapp can read
    /// localStorage and re-call this endpoint with `pubkey` filled in.
    #[serde(default)]
    pub pubkey: Option<String>,
}

/// JSON body returned by `GET /oauth/authorize` when the caller is
/// programmatic (Accept: application/json) or supplies a `pubkey`. The
/// browser variant returns a 302 redirect to the consent page instead.
#[derive(Debug, Serialize)]
pub struct AuthorizeInitResponse {
    /// base64-encoded canonical-CBOR challenge bytes — exact bytes the user
    /// must sign with their Ed25519 key (via WASM `sign_challenge`).
    pub challenge_cbor: String,
    /// Echoed for client correlation; same `state` the caller supplied.
    pub state: String,
    /// Unix-second expiry — challenge is rejected if redeemed after this.
    pub exp: u64,
}

/// `GET /oauth/authorize` — OAuth 2.1 + PKCE bootstrap. Validates
/// `code_challenge_method == "S256"` (Decision 10), generates a server
/// nonce + 60s expiry, builds the canonical-CBOR challenge per Decision 10,
/// computes its blake3 hash, and inserts a pending record under `state`.
///
/// Two modes:
///
/// - **JSON mode** (Accept: application/json or `pubkey` query param
///   present): returns `{challenge_cbor, state, exp}` for programmatic
///   clients (the webapp's fetch flow + integration tests). The webapp
///   uses the bytes directly with WASM `sign_challenge`, then POSTs the
///   COSE_Sign1 to `POST /oauth/authorize`.
///
/// - **Redirect mode** (browser default): 302 to the webapp consent page
///   with `?challenge=<base64-cbor>&state=<state>` so the webapp can read
///   the localStorage keypair, sign in WASM, and POST back to
///   `POST /oauth/authorize`.
///
/// The handler is rate-limited at the route layer (`/oauth/*` governor in
/// `main.rs::run_http`).
pub async fn authorize_init_handler(
    State(state): State<Arc<OAuthState>>,
    Query(q): Query<AuthorizeInitQuery>,
    request: Request<Body>,
) -> Response {
    use rand::RngCore;

    // S256-only PKCE per Decision 10 — reject everything else at the
    // protocol layer rather than relying on hash-mismatch downstream.
    if q.code_challenge_method != "S256" {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "code_challenge_method must be S256",
        );
    }
    if q.client_id.is_empty() || q.redirect_uri.is_empty() || q.code_challenge.is_empty() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "client_id, redirect_uri, and code_challenge are required",
        );
    }
    if q.state.is_empty() {
        return oauth_error(StatusCode::BAD_REQUEST, "state is required");
    }
    // redirect_uri allowlist (mnemonic-cli tech-spec Decision 5). Reject
    // arbitrary URIs at the bootstrap so a downstream pending entry is never
    // created with an attacker-controlled callback. Without this gate any
    // `client_id` could ride the OAuth flow to drive the issued code at any
    // origin — open-redirect via the OAuth surface.
    if !state.allows_redirect(&q.redirect_uri, &q.client_id) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "redirect_uri is not on the server allowlist",
        );
    }
    if let Some(rt) = q.response_type.as_deref() {
        if !rt.is_empty() && rt != "code" {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "response_type must be 'code' if supplied",
            );
        }
    }

    let now = now_secs();
    let exp = now + STATE_TTL_SECS;

    // Server-generated 16-byte random nonce. Hex-encoded for stable string
    // representation in the canonical-CBOR map.
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);

    let origin = server_origin();
    let challenge_obj = serde_json::json!({
        "server_origin": origin,
        "state": q.state,
        "client_id": q.client_id,
        "redirect_uri": q.redirect_uri,
        "code_challenge": q.code_challenge,
        "code_challenge_method": q.code_challenge_method,
        "nonce": nonce,
        "exp": exp,
    });
    let challenge_bytes = match to_canonical_cbor(&challenge_obj, &CHALLENGE_SCHEMA) {
        Ok(b) => b,
        Err(e) => {
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("canonical CBOR build failed: {e}"),
            );
        }
    };
    // Hash via the shared `build_challenge_hash` to keep the bootstrap and
    // any external recomputation (tests, future caller-side validators) on
    // a single helper.
    let challenge_hash = match build_challenge_hash(
        origin,
        &q.state,
        &q.client_id,
        &q.redirect_uri,
        &q.code_challenge,
        &q.code_challenge_method,
        &nonce,
        exp,
    ) {
        Ok(h) => h,
        Err(e) => {
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("challenge hash build failed: {e}"),
            );
        }
    };
    // Defense-in-depth: the bootstrap-side `to_canonical_cbor` and the
    // `build_challenge_hash` call must produce the same bytes. If they
    // ever drift the hash chain breaks; surface immediately.
    debug_assert_eq!(blake3_hex(&challenge_bytes), challenge_hash);

    // Decide JSON vs redirect mode. JSON if Accept includes application/json
    // OR if `pubkey` was supplied (programmatic clients always know their
    // identity ahead of time).
    let wants_json = q.pubkey.is_some()
        || request
            .headers()
            .get(axum::http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("application/json"))
            .unwrap_or(false);

    // Bind to expected pubkey if the caller supplied one — otherwise the
    // sentinel empty string lets the consent page resolve the binding when
    // the user clicks Sign and the webapp re-calls with `pubkey` filled.
    let expected_pubkey = q.pubkey.clone().unwrap_or_default();

    state.insert_pending(
        q.state.clone(),
        challenge_hash,
        challenge_bytes.clone(),
        expected_pubkey,
        q.redirect_uri.clone(),
        q.code_challenge.clone(),
        q.state.clone(),
        exp,
    );

    let challenge_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &challenge_bytes);

    if wants_json {
        let body = AuthorizeInitResponse {
            challenge_cbor: challenge_b64,
            state: q.state,
            exp,
        };
        return (StatusCode::OK, Json(body)).into_response();
    }

    // Redirect mode — point the browser at the webapp consent page.
    // URL-encode the base64 (it can contain `+/=`).
    let challenge_param = urlencoding_encode(&challenge_b64);
    let state_param = urlencoding_encode(&q.state);
    let mcp_base_param = urlencoding_encode(origin);
    let location = format!(
        "{WEBAPP_CONSENT_URL}?challenge={challenge_param}&state={state_param}&mcp_base={mcp_base_param}"
    );
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::FOUND;
    if let Ok(hv) = axum::http::HeaderValue::from_str(&location) {
        resp.headers_mut().insert(axum::http::header::LOCATION, hv);
    }
    resp
}

/// Minimal URL-encoder for application/x-www-form-urlencoded values.
/// Avoids pulling in a new dependency for the one redirect target.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => {
                out.push_str(&format!("%{other:02X}"));
            }
        }
    }
    out
}

// ── redirect_uri allowlist (mnemonic-cli tech-spec Decision 5) ──────────────

/// Exact-match allowlisted `redirect_uri`.
const REDIRECT_EXACT: &[&str] = &[
    "https://mnemonik.xyz/oauth/consent",
    "https://www.mnemonik.xyz/oauth/consent",
    "https://vscode.dev/redirect",
];

/// Exact-prefix allowlisted `redirect_uri`. The submitted URI must START with
/// one of these byte-for-byte. Used for AI-tool deeplinks and the Anthropic
/// connector callback whose path varies per session.
const REDIRECT_PREFIXES: &[&str] = &[
    "cursor://anysphere.cursor-deeplink/",
    "vscode:mcp/",
    "https://claude.ai/api/",
];

/// Validate `redirect_uri` against the allowlist (mnemonic-cli tech-spec
/// Decision 5 + RFC 8252 §7).
///
/// Returns `true` if the URI is on one of three lists:
/// 1. Exact match of `REDIRECT_EXACT` (e.g. webapp consent page).
/// 2. Exact-prefix match of `REDIRECT_PREFIXES` (deeplink schemes).
/// 3. Loopback callback `http://127.0.0.1:<port>[/<path>]`,
///    `http://[::1]:<port>[/<path>]`, or
///    `http://localhost:<port>[/<path>]` for any client (RFC 8252 §7.3).
///
/// `client_id` was previously used to gate loopback access to the literal
/// `mnemonic-cli` client only, with a fixed `/callback` path. That broke
/// VS Code's MCP OAuth (which uses DCR-issued UUID client_ids and `/`
/// path) and is not a real defense — PKCE + state already bind the auth
/// code to the originating client, and a loopback URI is on the user's
/// own machine, so there is no impersonation risk that a client_id check
/// would mitigate. Per RFC 8252 §7.3, native clients SHOULD be permitted
/// to use loopback with any port and any path.
///
/// All other URIs (`https://evil.com`, `http://0.0.0.0:1234`,
/// `http://127.0.0.1.evil.com`, `http://localhost.evil.com`) → false.
pub fn allowed_redirect(uri: &str, _client_id: &str) -> bool {
    if REDIRECT_EXACT.contains(&uri) {
        return true;
    }
    if REDIRECT_PREFIXES
        .iter()
        .any(|prefix| uri.starts_with(prefix))
    {
        return true;
    }
    if is_loopback_redirect(uri) {
        return true;
    }
    false
}

/// DCR redirect matching. Exact matches always pass. Loopback callbacks also
/// match when host and path are identical but the port differs, per RFC 8252
/// section 7.3 and VS Code's MCP OAuth behavior.
fn registered_redirect_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }

    match (
        split_loopback_redirect(registered),
        split_loopback_redirect(requested),
    ) {
        (Some((registered_host, registered_path)), Some((requested_host, requested_path))) => {
            registered_host == requested_host && registered_path == requested_path
        }
        _ => false,
    }
}

fn split_loopback_redirect(uri: &str) -> Option<(&'static str, &str)> {
    let (host, rest) = if let Some(rest) = uri.strip_prefix("http://127.0.0.1") {
        ("127.0.0.1", rest)
    } else if let Some(rest) = uri.strip_prefix("http://[::1]") {
        ("::1", rest)
    } else {
        ("localhost", uri.strip_prefix("http://localhost")?)
    };

    let path = if let Some(rest) = rest.strip_prefix(':') {
        let slash = rest.find('/')?;
        let port = &rest[..slash];
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        &rest[slash..]
    } else if rest.starts_with('/') {
        rest
    } else {
        return None;
    };

    Some((host, path))
}

/// Match `^http://127\.0\.0\.1:\d+(/.*)?$`,
/// `^http://\[::1\]:\d+(/.*)?$`, and
/// `^http://localhost:\d+(/.*)?$` per RFC 8252 §7.3 without pulling in a
/// `regex` crate. Hand-rolled is cheaper and fully covered by unit tests.
///
/// Rules:
///   - host MUST be exactly `127.0.0.1`, `[::1]`, or `localhost` (the
///     leading-`http://` + exact-host strip rejects e.g.
///     `http://127.0.0.1.evil.com:80/`)
///   - port MUST be present and all-ASCII-digits (RFC 8252 forbids
///     omitting the port for loopback)
///   - path is OPTIONAL. If present, it MUST start with `/` and contain
///     no fragment (`#`). Any path is accepted — VS Code uses `/`,
///     mnemonic-cli uses `/callback`, future clients may use anything.
///
/// Was previously called `is_loopback_callback` and required the literal
/// path `/callback`; renamed + relaxed because that constraint was a
/// `mnemonic-cli`-specific assumption that broke real-world OAuth clients
/// (VS Code MCP OAuth uses path `/`).
fn is_loopback_redirect(uri: &str) -> bool {
    // Strip the scheme + host prefix; require the EXACT host string so
    // `http://127.0.0.1.evil.com:80/...` does not match.
    let rest = if let Some(r) = uri.strip_prefix("http://127.0.0.1:") {
        r
    } else if let Some(r) = uri.strip_prefix("http://[::1]:") {
        r
    } else if let Some(r) = uri.strip_prefix("http://localhost:") {
        r
    } else {
        return false;
    };
    // `rest` is `<digits>` (no path) or `<digits>/<anything-except-#>`.
    let (port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // Reject fragments — they're never sent by the browser anyway, but
    // an attacker-controlled `redirect_uri` registration shouldn't be
    // able to slip a `#fragment` past us.
    if let Some(p) = path {
        if p.contains('#') {
            return false;
        }
    }
    true
}

// ── /oauth/authorize handler (Decision 10) ───────────────────────────────────

/// Request body for `POST /oauth/authorize`. The browser receives the
/// canonical-CBOR challenge bytes from `GET /oauth/authorize`, signs them
/// with the user's localStorage Ed25519 key via WASM `sign_challenge`
/// (returns raw 64-byte signature, NOT a COSE_Sign1 envelope), and POSTs
/// the signature + signer pubkey back here. The server re-derives the
/// challenge bytes from the pending record and verifies with Ed25519.
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    /// Original CSRF state — must match a record in OAuthState.pending.
    pub state: String,
    /// Raw 64-byte Ed25519 signature (base64) over the challenge bytes
    /// the server returned from the bootstrap. The legacy `cose_signed`
    /// field name is preserved as a deserialization alias for older
    /// clients that still wrap in COSE_Sign1; we no longer prefer that
    /// path because in-browser COSE wrapping was the principal source of
    /// hard-to-debug "extraneous data in CBOR" failures.
    #[serde(alias = "cose_signed")]
    pub signature: String,
    /// Base58 Ed25519 pubkey of the signer. The server verifies the
    /// signature against the pending entry's `challenge_bytes` using
    /// THIS pubkey. The pending entry's `expected_pubkey` (when set)
    /// must equal this — otherwise we reject the cross-identity claim.
    #[serde(default)]
    pub signer_pubkey: String,
}

/// Response for a successful `/oauth/authorize` POST. Mirrors the OAuth 2.1
/// redirect-back convention: the client uses `code` at `/oauth/token` to
/// exchange for a JWT.
#[derive(Debug, Serialize)]
pub struct AuthorizeResponse {
    pub code: String,
    pub state: String,
    pub redirect_uri: String,
}

/// `POST /oauth/authorize` — verify the user's COSE_Sign1 over the canonical
/// CBOR challenge, then issue a single-use authorization code.
pub async fn authorize_handler(
    State(state): State<Arc<OAuthState>>,
    Json(req): Json<AuthorizeRequest>,
) -> Response {
    // Atomic single-use: pop the entry now so a second concurrent submission
    // with the same `state` cannot replay the signature.
    let pending = {
        let mut guard = state.pending.lock().expect("pending mutex poisoned");
        guard.pop(&req.state)
    };
    let pending = match pending {
        Some(p) => p,
        None => return oauth_error(StatusCode::UNAUTHORIZED, "unknown or already-used state"),
    };

    // Expiry check.
    if now_secs() > pending.exp {
        return oauth_error(StatusCode::UNAUTHORIZED, "challenge expired");
    }

    // Decode the raw 64-byte Ed25519 signature from base64. (Field name is
    // `signature`; legacy `cose_signed` is accepted via serde alias.)
    let sig_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.signature.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                &format!("signature is not valid base64: {e}"),
            );
        }
    };

    // Validate signature length (Ed25519 = 64 bytes).
    if sig_bytes.len() != 64 {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            &format!("signature must be 64 bytes, got {}", sig_bytes.len()),
        );
    }

    // Validate signer_pubkey is non-empty + parseable base58 Ed25519 pubkey.
    if req.signer_pubkey.trim().is_empty() {
        return oauth_error(StatusCode::BAD_REQUEST, "signer_pubkey is required");
    }
    let signer_pubkey: solana_sdk::pubkey::Pubkey = match req.signer_pubkey.parse() {
        Ok(pk) => pk,
        Err(e) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                &format!("signer_pubkey is not valid base58 Ed25519 pubkey: {e}"),
            );
        }
    };

    // Bind to expected pubkey when present (bootstrap-with-pubkey path).
    // Empty `expected_pubkey` is the sentinel for "any keypair valid for
    // first-touch consent" — the signature itself authoritatively names
    // the signer, and there's no cross-identity claim to defeat. When
    // populated, the expected_pubkey was bound at bootstrap time and
    // must match the submitted signer.
    if !pending.expected_pubkey.is_empty() && req.signer_pubkey != pending.expected_pubkey {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "signer pubkey does not match expected pubkey",
        );
    }

    // Verify the raw Ed25519 signature against the canonical-CBOR challenge
    // bytes the server stored at bootstrap. This is the gate Decision 10
    // depends on — a valid signature proves the user authorizes the
    // client's redirect_uri/state pair, since both are part of the bytes.
    if !mnemonic_core::identity::verify_signature(
        &signer_pubkey,
        &pending.challenge_bytes,
        &sig_bytes,
    ) {
        return oauth_error(StatusCode::UNAUTHORIZED, "Ed25519 signature invalid");
    }

    // Issue a single-use code, store binding for /token. The `redirect_uri`
    // recorded here is the same one the client supplied to /oauth/authorize
    // (the GET bootstrap) and that we already gated through
    // `allowed_redirect`. Storing it on the IssuedCode lets /oauth/token
    // verify the body's `redirect_uri` matches — RFC 6749 §4.1.3 binding
    // that closes a swap-redirect attack against a leaked authorization code.
    let code = uuid::Uuid::new_v4().to_string();
    {
        let mut guard = state.codes.lock().expect("codes mutex poisoned");
        guard.put(
            code.clone(),
            IssuedCode {
                sub: req.signer_pubkey.clone(),
                code_challenge: pending.code_challenge.clone(),
                redirect_uri: pending.redirect_uri.clone(),
                google_sub: None,
                exp: now_secs() + CODE_TTL_SECS,
            },
        );
    }

    let body = AuthorizeResponse {
        code,
        state: pending.client_state,
        redirect_uri: pending.redirect_uri,
    };
    (StatusCode::OK, Json(body)).into_response()
}

// ── /oauth/token handler (PKCE + JWT issue) ──────────────────────────────────

/// `POST /oauth/token` request body — covers BOTH grant types
/// (`authorization_code` and `refresh_token`, refresh-token-rotation
/// Decision 11). Every field is `Option<String>` so the same struct
/// deserialises either grant; the post-parse dispatch in `token_handler`
/// validates the per-grant required fields.
///
/// `grant_type` defaults to `"authorization_code"` via `default_grant_type`
/// so legacy clients (existing webapp redeem, integration tests,
/// `scripts/test-oauth-flow.sh`) that omit the field still hit the
/// existing code+verifier path byte-for-byte.
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    /// OAuth 2.1 grant type. Defaults to `"authorization_code"` for legacy
    /// clients that pre-date the dispatch. Known values:
    /// `"authorization_code"` | `"refresh_token"`. Anything else returns
    /// `400 unsupported_grant_type` (Decision 11 + RFC 6749 §5.2).
    #[serde(default = "default_grant_type")]
    pub grant_type: String,
    /// Authorization code from `/oauth/authorize`. Required when
    /// `grant_type == "authorization_code"`.
    #[serde(default)]
    pub code: Option<String>,
    /// PKCE code_verifier. Required when `grant_type == "authorization_code"`.
    #[serde(default)]
    pub code_verifier: Option<String>,
    /// `redirect_uri` from the original authorize request. Optional for
    /// backward compatibility with clients that omit it (legacy webapp,
    /// integration tests). When present on an `authorization_code` grant,
    /// MUST equal the value bound at `/oauth/authorize` time — RFC 6749
    /// §4.1.3 + RFC 7636 §4.4 require this equality check to defeat a
    /// swap-redirect attack on a leaked code.
    #[serde(default)]
    pub redirect_uri: Option<String>,
    /// Refresh token plaintext. Required when `grant_type == "refresh_token"`.
    /// Length-capped at 4096 bytes BEFORE any hashing or DB lookup
    /// (Decision 16) to short-circuit oversized payloads.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// `serde(default)` for `TokenRequest.grant_type` — keeps legacy clients
/// (no `grant_type` field) hitting the `authorization_code` path.
fn default_grant_type() -> String {
    "authorization_code".to_string()
}

/// Maximum accepted `refresh_token` payload length (Decision 16). Rejected
/// pre-hash + pre-DB to short-circuit oversized inputs.
pub const REFRESH_TOKEN_MAX_LEN: usize = 4096;

/// Pure helper — validates `MCP_REFRESH_SALT` (refresh-token-rotation
/// Decision 2). Decoder is `base64::engine::general_purpose::STANDARD`,
/// matching the `openssl rand -base64 32` recipe shipped in `.env.example`
/// (which emits standard padded base64 with `+/=` charset, NOT
/// url-safe-no-pad).
///
/// Rejects:
/// - `None` — env var unset.
/// - base64 decode failure.
/// - decoded byte length < 32 (closes the operator-typed-32-ASCII-chars
///   footgun where the env value looks "long enough" but standard-base64
///   decodes to `~5` bytes of effective entropy).
///
/// Pure and side-effect-free so unit tests can exercise it directly
/// without booting `run_http`.
pub fn validate_refresh_salt(env_value: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let raw = env_value.ok_or_else(|| {
        anyhow::anyhow!(
            "MCP_REFRESH_SALT env var is required (generate via `openssl rand -base64 32`)"
        )
    })?;
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw.trim())
        .map_err(|e| {
            anyhow::anyhow!(
                "MCP_REFRESH_SALT is not valid standard-padded base64: {e} \
                 (generate via `openssl rand -base64 32`)"
            )
        })?;
    if decoded.len() < 32 {
        anyhow::bail!(
            "MCP_REFRESH_SALT decoded to {} bytes — must be >= 32 \
             (generate via `openssl rand -base64 32`)",
            decoded.len()
        );
    }
    Ok(decoded)
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    /// OAuth 2.1 / RFC 6749 §5.1 — the scope the token was granted.
    /// We only have one scope (`mcp`); echoed for clients that read it.
    pub scope: String,
    /// Refresh token plaintext (RFC 6749 §5.1). Emitted on
    /// `authorization_code` mint AND on every `refresh_token` rotation
    /// (Branch A). Absent for early `authorization_code` exchanges that
    /// pre-date Task 3 — `skip_serializing_if` keeps wire-compat with
    /// clients that didn't ask for one. Branch B (legitimate retry inside
    /// reuse-interval) returns the byte-identical previous refresh_token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// `POST /oauth/token` — exchange a fresh authorization code for a JWT.
///
/// PKCE: `SHA256(code_verifier)` (base64url, no padding) must equal the
/// stored `code_challenge`. Single-use — code is removed atomically.
///
/// Per OAuth 2.1 (RFC 6749 §3.2), the token endpoint accepts requests
/// with `Content-Type: application/x-www-form-urlencoded` — VS Code,
/// Claude.ai, and most OAuth client libraries default to that.
/// Cursor sends `application/json`, so we accept BOTH and ignore any
/// extra standard fields (`grant_type`, `redirect_uri`, `client_id`)
/// that we do not need to validate (PKCE alone closes the auth-code
/// confused-deputy attack — `redirect_uri` was bound at bootstrap and
/// `client_id` is opaque to us per Decision 11 / DCR comments).
pub async fn token_handler(
    State(state): State<Arc<OAuthState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let resp = token_handler_inner(&state, &headers, &body).await;
    apply_no_store_headers(resp)
}

/// Inner handler — returns a `Response` from any exit point. The outer
/// `token_handler` wraps the result with `apply_no_store_headers` so every
/// `/oauth/token` response (success AND error) carries
/// `Cache-Control: no-store` + `Pragma: no-cache` (Decision 15 / RFC 6749
/// §5.1). Single helper at the boundary eliminates the N-return-site
/// header-insert pattern.
async fn token_handler_inner(
    state: &Arc<OAuthState>,
    headers: &axum::http::HeaderMap,
    body: &axum::body::Bytes,
) -> Response {
    // CR3-R1-M2 / D14 forensic context. `remote_addr` is the first hop in
    // `X-Forwarded-For` (set by the front nginx) if present, else
    // `X-Real-IP`, else the "-" sentinel. `request_id` is a per-call
    // UUIDv4 — propagated into `RotateContext` so Branch C family-revoke
    // and Branch E credential-stuffing log lines can be correlated to a
    // specific caller for forensic review.
    let remote_addr = extract_forensic_remote_addr(headers);
    let request_id = uuid::Uuid::new_v4().to_string();
    // Decide between JSON and x-www-form-urlencoded. Default to JSON if
    // the header is missing — keeps existing programmatic clients
    // (`scripts/test-oauth-flow.sh`, integration tests) working.
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_lowercase();

    let req: TokenRequest = if ct.starts_with("application/x-www-form-urlencoded") {
        match serde_urlencoded::from_bytes(body) {
            Ok(r) => r,
            Err(e) => {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    &format!("token request form parse failed: {e}"),
                );
            }
        }
    } else {
        // JSON (or unknown content-type — JSON is our default).
        match serde_json::from_slice(body) {
            Ok(r) => r,
            Err(e) => {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    &format!("token request JSON parse failed: {e}"),
                );
            }
        }
    };

    match req.grant_type.as_str() {
        "authorization_code" => token_handler_authorization_code(state, req).await,
        "refresh_token" => token_handler_refresh(state, req, remote_addr, request_id).await,
        // RFC 6749 §5.2 / Decision 11 — any other grant_type must NOT
        // silently fall through to the authorization_code path with empty
        // code/verifier (which would surface a confusing 401). The
        // explicit `unsupported_grant_type` 400 closes that vector.
        //
        // SA3-R1-M1: do NOT echo the user-supplied `req.grant_type` into
        // the response body. The code-only envelope keeps attacker-
        // controlled strings off the wire.
        _ => oauth_error_typed(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type is not one of authorization_code, refresh_token",
        ),
    }
}

/// Branch: `grant_type=authorization_code`. Existing PKCE + JWT-issue path,
/// now with explicit non-empty validation on `code` and `code_verifier`
/// (Decision 11 — previously enforced by serde's non-Option fields; the
/// widened `TokenRequest` makes the check explicit).
async fn token_handler_authorization_code(state: &Arc<OAuthState>, req: TokenRequest) -> Response {
    let code = match req.code.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c.to_string(),
        None => {
            return oauth_error_typed(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "code is required",
            );
        }
    };
    let code_verifier = match req.code_verifier.as_deref().filter(|s| !s.is_empty()) {
        Some(v) => v.to_string(),
        None => {
            return oauth_error_typed(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "code_verifier is required",
            );
        }
    };

    let issued = {
        let mut guard = state.codes.lock().expect("codes mutex poisoned");
        guard.pop(&code)
    };
    let issued = match issued {
        Some(c) => c,
        None => return oauth_error(StatusCode::UNAUTHORIZED, "unknown or already-used code"),
    };

    if now_secs() > issued.exp {
        return oauth_error(StatusCode::UNAUTHORIZED, "code expired");
    }

    // Bind /oauth/token's `redirect_uri` to the value supplied at
    // /oauth/authorize. RFC 6749 §4.1.3: when the authorize request included
    // a redirect_uri, the token request MUST include the IDENTICAL value.
    // We treat the field as optional for legacy clients but reject any
    // mismatch — silent acceptance of a wrong `redirect_uri` would let an
    // attacker who guessed/leaked the auth code drive the JWT to a callback
    // they control.
    if let Some(supplied) = req.redirect_uri.as_deref() {
        if supplied != issued.redirect_uri {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "redirect_uri does not match the value bound at /oauth/authorize",
            );
        }
    }

    // Verify PKCE: SHA256(verifier) base64url-no-pad == challenge.
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(code_verifier.as_bytes());
    let derived = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest);
    if derived != issued.code_challenge {
        return oauth_error(StatusCode::UNAUTHORIZED, "code_verifier does not match");
    }

    // SA3-R1-M2: keep the underlying `{e}` in the server-side warn log
    // only; the response body carries a fixed prose so an internal
    // error string is never echoed to the wire.
    let token = match issue_jwt_with_google_sub(state, &issued.sub, issued.google_sub.clone()) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                target: "mnemonic_mcp::oauth",
                error = %e,
                "JWT issuance failed during authorization_code redeem"
            );
            return oauth_error_typed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "JWT issuance failed",
            );
        }
    };
    // Best-effort cache the minted JWT to `~/.mnemonic/token.json` so the
    // Rust binary's own subsequent outbound calls can reuse it without
    // re-running the OAuth loopback (Task 6 of agent-native-distribution).
    // Failure is non-fatal — the response to the calling agent still ships
    // the token, and a corrupted/missing cache only forces a re-OAuth next
    // time. Decision 7: no OS keychain wrapper in V1.
    cache_minted_token(&token, &issued.sub);

    // Mint the refresh token paired with this authorization_code redemption.
    // New family_id per mint (D3 — every authorize handshake is its own
    // device family). Failure here is best-effort + non-fatal: the access
    // token is still surfaced; the client will fall back to a fresh
    // `authorization_code` flow on access-token expiry rather than a
    // refresh-token rotation. Operational metric: count of mints that
    // failed surfaces via the WARN log.
    let refresh_token = {
        let guard = state.refresh_store.lock().ok();
        match guard {
            Some(conn) => match refresh::mint_for_authorization_code(
                &conn,
                &state.refresh_salt,
                &issued.sub,
                issued.google_sub.as_deref(),
            ) {
                Ok(rt) => Some(rt.plaintext),
                Err(e) => {
                    tracing::warn!(
                        target: "mnemonic_mcp::oauth",
                        error = %e,
                        "refresh_token mint failed after authorization_code redeem; access_token still surfaced"
                    );
                    None
                }
            },
            None => {
                tracing::warn!(
                    target: "mnemonic_mcp::oauth",
                    "refresh_store mutex poisoned; skipping refresh_token mint"
                );
                None
            }
        }
    };

    let body = TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: jwt_ttl_secs(),
        scope: "mcp".to_string(),
        refresh_token,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Branch: `grant_type=refresh_token`. Length-cap (D16) + non-empty check
/// FIRST so an oversized payload is rejected before hashing or DB work.
/// Then call `refresh::rotate` and translate the outcome to the 4-field
/// JSON response (or `invalid_grant` on Branch B'/C/D/E).
///
/// `remote_addr` + `request_id` are threaded into `RotateContext` so
/// D14 log lines from Branches C/D/E carry the forensic correlation
/// fields the tech-spec mandates (round-1 code-reviewer M2).
async fn token_handler_refresh(
    state: &Arc<OAuthState>,
    req: TokenRequest,
    remote_addr: String,
    request_id: String,
) -> Response {
    let plaintext = match req.refresh_token.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::info!(
                target: "mnemonic_mcp::oauth",
                outcome = "invalid_request",
                reason = "refresh_token_missing",
                remote_addr = %remote_addr,
                request_id = %request_id,
                "refresh_grant rejected (empty / missing refresh_token)"
            );
            return oauth_error_typed(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "refresh_token is required",
            );
        }
    };
    // Decision 16: enforce the 4096-byte cap BEFORE any hashing or DB
    // lookup. Both gates short-circuit oversized inputs before they can
    // consume CPU or row-lock budget.
    if plaintext.len() > REFRESH_TOKEN_MAX_LEN {
        tracing::info!(
            target: "mnemonic_mcp::oauth",
            outcome = "invalid_request",
            reason = "refresh_token_too_long",
            length = plaintext.len(),
            remote_addr = %remote_addr,
            request_id = %request_id,
            "refresh_grant rejected (length cap)"
        );
        return oauth_error_typed(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refresh_token exceeds 4096 bytes",
        );
    }

    // Clone the Arcs the minter closure needs — the rusqlite Connection
    // guard must stay sync (no `.await` while held), so we hop onto a
    // blocking thread for the rotation.
    let store = Arc::clone(&state.refresh_store);
    let cache = Arc::clone(&state.reuse_cache);
    let salt = state.refresh_salt.clone();
    let state_for_minter = Arc::clone(state);
    let remote_addr_for_ctx = remote_addr.clone();
    let request_id_for_ctx = request_id.clone();
    let outcome_result = tokio::task::spawn_blocking(move || {
        let conn = store
            .lock()
            .map_err(|_| anyhow::anyhow!("refresh_store mutex poisoned"))?;
        let minter = move |sub: &str, google_sub: Option<&str>| -> anyhow::Result<String> {
            issue_jwt_with_google_sub(&state_for_minter, sub, google_sub.map(str::to_string))
                .map_err(|e| anyhow::anyhow!("JWT issuance failed: {e}"))
        };
        let ctx = refresh::RotateContext {
            remote_addr: Some(remote_addr_for_ctx.as_str()),
            request_id: Some(request_id_for_ctx.as_str()),
        };
        refresh::rotate(&conn, &salt, &cache, &plaintext, &minter, &ctx)
    })
    .await;

    let outcome = match outcome_result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            tracing::warn!(
                target: "mnemonic_mcp::oauth",
                remote_addr = %remote_addr,
                request_id = %request_id,
                error = %e,
                "refresh_token rotation failed (internal)"
            );
            return oauth_error_typed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "refresh rotation failed",
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "mnemonic_mcp::oauth",
                remote_addr = %remote_addr,
                request_id = %request_id,
                error = %e,
                "refresh_token rotation task panicked"
            );
            return oauth_error_typed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "refresh rotation task panicked",
            );
        }
    };

    match outcome {
        refresh::RotateOutcome::Rotated {
            new_token,
            sub,
            access_jwt,
            ..
        } => {
            // CR3-R1-C1 fix: surface the SAME `access_jwt` minted inside
            // `refresh::rotate` (and already published to the reuse
            // cache before COMMIT, per D5 ordering). A subsequent
            // Branch B retry of the OLD plaintext now receives a
            // byte-identical `(access_jwt, refresh_plaintext)` pair to
            // the one returned here — closing the AC12 idempotency
            // gap that a re-mint in this scope previously broke.
            cache_minted_token(&access_jwt, &sub);
            // D14: handler-side INFO log with sub + family_id +
            // remote_addr + request_id. Mirrors the rotate-internal
            // log line but emitted on the async tokio task (NOT inside
            // `spawn_blocking`) so test harnesses with thread-local
            // tracing subscribers (`tracing-test`) can observe it.
            tracing::info!(
                target: "mnemonic_mcp::oauth",
                outcome = "rotated",
                branch = "A",
                family_id = %new_token.family_id,
                sub = %sub,
                remote_addr = %remote_addr,
                request_id = %request_id,
                "refresh_grant rotated"
            );
            let body = TokenResponse {
                access_token: access_jwt,
                token_type: "Bearer".to_string(),
                expires_in: jwt_ttl_secs(),
                scope: "mcp".to_string(),
                refresh_token: Some(new_token.plaintext),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        refresh::RotateOutcome::Replayed {
            access_jwt,
            refresh_plaintext,
        } => {
            // D14: handler-side DEBUG with remote_addr + request_id.
            // We don't have sub / family_id at this layer because the
            // `Replayed` outcome doesn't surface them — the rotate
            // internal log already carries the full forensic set, and
            // this handler-side line exists for harness observability.
            tracing::debug!(
                target: "mnemonic_mcp::oauth",
                outcome = "replayed",
                branch = "B",
                remote_addr = %remote_addr,
                request_id = %request_id,
                "refresh_grant replayed"
            );
            let body = TokenResponse {
                access_token: access_jwt,
                token_type: "Bearer".to_string(),
                expires_in: jwt_ttl_secs(),
                scope: "mcp".to_string(),
                refresh_token: Some(refresh_plaintext),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        refresh::RotateOutcome::InvalidGrant => {
            // Decision 11 / RFC 6749 §5.2 — Branch B' / C / D / E are all
            // surfaced as `invalid_grant` to the client. The granular
            // distinction lives in the server-side tracing log per D14.
            tracing::info!(
                target: "mnemonic_mcp::oauth",
                outcome = "invalid_grant",
                branch = "B_prime_or_C_or_D_or_E",
                remote_addr = %remote_addr,
                request_id = %request_id,
                "refresh_grant rejected"
            );
            oauth_error_typed(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "refresh_token rejected",
            )
        }
    }
}

/// Apply RFC 6749 §5.1 / Decision 15 cache-suppression headers to a
/// `/oauth/token` response. Called from the outer `token_handler` so
/// every success-and-error exit point picks them up.
fn apply_no_store_headers(mut resp: Response) -> Response {
    use axum::http::HeaderValue;
    let h = resp.headers_mut();
    h.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    h.insert(
        axum::http::header::PRAGMA,
        HeaderValue::from_static("no-cache"),
    );
    resp
}

/// Persist `token` to `~/.mnemonic/token.json` as a best-effort cache. The
/// on-disk `expires_at` is derived from the JWT's own `exp` claim so it
/// matches the issuer's notion of expiry byte-for-byte — no clock-skew
/// window between the JWT mint and the cache write (round-1 code review
/// R1-MINOR-2). Errors are logged at `warn` and swallowed — a cache
/// failure must never break the OAuth response (the calling agent has
/// already received its token).
///
/// **Security contract** (round-1 security audit SA6-003): `sub` MUST be
/// a PKCE-validated subject (already proven against the COSE_Sign1
/// signature at `authorize_handler`). Callers that mint JWTs without a
/// PKCE handshake must NOT call this helper — the cached file would
/// associate the caller's identity with someone else's keypair on disk.
/// Currently the only production callsite is `token_handler` immediately
/// after `issue_jwt_with_google_sub`, where `issued.sub` is the
/// PKCE-bound subject.
///
/// Public (rather than `pub(crate)`) so integration tests at
/// `mcp/tests/oauth_loopback.rs` can exercise the cache path directly;
/// no production callsite outside this module exists today.
pub fn cache_minted_token(jwt: &str, sub: &str) {
    let expires_at = match extract_exp_unix_no_verify(jwt) {
        Some(exp) => match chrono::DateTime::<chrono::Utc>::from_timestamp(exp as i64, 0) {
            Some(dt) => dt.to_rfc3339(),
            // exp claim outside chrono's representable range — fall back to
            // the conservative "now + TTL" estimate; logs a debug line so
            // an operator can investigate.
            None => {
                tracing::debug!(
                    target: "mnemonic_mcp::oauth",
                    "JWT exp {exp} out of chrono range; falling back to now+TTL"
                );
                (chrono::Utc::now() + chrono::Duration::seconds(jwt_ttl_secs() as i64)).to_rfc3339()
            }
        },
        None => {
            // No parseable exp claim — the cache becomes useless after the
            // first read (Expired returned), but at least no panic, and a
            // re-OAuth fixes the cache on the next call. Same fallback.
            tracing::debug!(
                target: "mnemonic_mcp::oauth",
                "could not extract exp from minted JWT; falling back to now+TTL"
            );
            (chrono::Utc::now() + chrono::Duration::seconds(jwt_ttl_secs() as i64)).to_rfc3339()
        }
    };
    let token = mnemonic_core::identity::TokenJson {
        jwt: jwt.to_string(),
        expires_at,
        sub: sub.to_string(),
    };
    if let Err(e) = mnemonic_core::identity::save_token(&token) {
        tracing::warn!(
            target: "mnemonic_mcp::oauth",
            "best-effort cache of minted JWT to ~/.mnemonic/token.json failed: {e}"
        );
    }
}

/// Extract the `exp` claim (unix seconds) from a JWT WITHOUT verifying the
/// signature. Used by [`cache_minted_token`] — the JWT was just minted in
/// this same process by [`issue_jwt_with_google_sub`], so signature
/// re-verification would be a tautology. Returns `None` if the JWT is not
/// in standard three-part form or the payload does not carry a numeric
/// `exp` claim.
fn extract_exp_unix_no_verify(jwt: &str) -> Option<u64> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let payload_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload_b64,
    )
    .ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("exp")?.as_u64()
}

/// Build a uniform OAuth-style error envelope (legacy single-field shape).
/// Pre-Task-3 callsites build `{"error": "<freeform prose>"}`. New
/// /oauth/token error paths should use [`oauth_error_typed`] instead so
/// the response shape matches RFC 6749 §5.2 exactly:
/// `{"error": "<code>", "error_description": "<prose>"}`.
fn oauth_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}

/// RFC 6749 §5.2 — `/oauth/token` error envelope. `error` is the bare
/// machine-readable code (`invalid_request`, `invalid_grant`,
/// `unsupported_grant_type`, `internal_error`); `error_description` is
/// the optional human-readable prose. Closes round-1 code-reviewer M1.
///
/// **D14 + SA3 R1 M1 + M2 compliance:** `error_description` carries
/// fixed, operator-authored prose only. NEVER format user-supplied
/// strings (`grant_type` echoes, `format!("...: {other}")`) or internal
/// error values (`format!("...: {e}")`) into this field — both would
/// leak attacker-controlled or implementation-detail strings to the
/// wire. Internal `{e}` belongs in the tracing::warn server-side log.
fn oauth_error_typed(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

/// Extract the forensic remote-address header for D14 logging. First
/// hop in `X-Forwarded-For` (the canonical nginx-set header), else
/// `X-Real-IP`, else `"-"`. Trims the value to the first comma so a
/// proxy chain's full list does not bloat the log line; bounded to
/// 64 chars char-boundary-safe so an attacker-supplied massive value
/// cannot pollute log aggregation.
///
/// The connection-level peer address is NOT used here because in
/// production the binary sits behind nginx; the peer is always
/// 127.0.0.1. `X-Forwarded-For` from a trusted reverse proxy is the
/// only meaningful client-IP signal.
fn extract_forensic_remote_addr(headers: &axum::http::HeaderMap) -> String {
    let raw = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok());
    let Some(s) = raw else {
        return "-".to_string();
    };
    let first_hop = s.split(',').next().unwrap_or("").trim();
    if first_hop.is_empty() {
        return "-".to_string();
    }
    bound_log_value(first_hop, 64)
}

// ── /.well-known discovery endpoints (RFC 8414 + MCP spec) ───────────────────
//
// Anthropic's MCP connector probes `GET /.well-known/oauth-authorization-server`
// (RFC 8414) and `GET /.well-known/oauth-protected-resource` (MCP spec)
// BEFORE attempting the OAuth flow. Without these the connector reports
// "Couldn't reach the MCP server" with a reference id even though
// `/mcp initialize` and `/mcp tools/list` work fine.
//
// Both endpoints are public metadata — no auth, no body inspection. They are
// allowlisted in `bearer_auth_middleware` by URI prefix (`/.well-known/`).

/// `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata document.
///
/// Returned fields are the minimum the MCP spec + Anthropic connector probe
/// require. The `code_challenge_methods_supported` advertises only `S256`
/// because Decision 10 enforces S256-only PKCE.
pub async fn oauth_authorization_server_metadata() -> Response {
    let origin = server_origin();
    let body = serde_json::json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "response_types_supported": ["code"],
        // refresh-token-rotation Task 3 / Decision 11: advertise both grants.
        // The `/oauth/token` handler accepts `grant_type=refresh_token`
        // dispatching to `refresh::rotate`; clients MUST observe this
        // metadata before attempting a refresh.
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /.well-known/oauth-protected-resource` — MCP spec metadata document
/// for the server origin as a whole.
pub async fn oauth_protected_resource_metadata() -> Response {
    let origin = server_origin();
    let body = serde_json::json!({
        "resource": origin,
        "authorization_servers": [origin],
        "scopes_supported": ["mcp"],
        "bearer_methods_supported": ["header"],
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /.well-known/oauth-protected-resource/mcp` — RFC 9728 §3.1
/// path-specific protected-resource metadata. The `resource` value here is
/// `<origin>/mcp` to match the URL the MCP client is actually connecting to.
///
/// Cursor's MCP OAuth provider (3.2+) requests this path-specific endpoint
/// FIRST, falls back to the root `/.well-known/oauth-protected-resource`
/// only if missing, and silently aborts the OAuth flow if the `resource`
/// claim does not match the URL it is connecting to. Without this endpoint
/// returning the path-qualified resource value, Cursor never opens the
/// browser to launch /oauth/authorize.
pub async fn oauth_protected_resource_metadata_mcp() -> Response {
    let origin = server_origin();
    let body = serde_json::json!({
        "resource": format!("{origin}/mcp"),
        "authorization_servers": [origin],
        "scopes_supported": ["mcp"],
        "bearer_methods_supported": ["header"],
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// `POST /oauth/register` — RFC 7591 Dynamic Client Registration.
///
/// Open registration: any client may register without authentication.
/// We return a generated `client_id` and echo back the request fields.
/// No `client_secret` is issued (we are a public OAuth client per
/// `token_endpoint_auth_methods_supported: ["none"]`); MCP clients use PKCE
/// (S256) for the auth-code flow instead of client authentication.
///
/// This endpoint exists so VS Code / Claude.ai do not stall at the
/// "Dynamic Client Registration not supported" prompt — they POST the
/// `redirect_uris` they intend to use (e.g., `http://127.0.0.1:33418`,
/// `https://vscode.dev/redirect`, `https://claude.ai/...`) and we mint a
/// `client_id` for them to use against `/oauth/authorize` and `/oauth/token`.
///
/// Registrations are stored in-memory and bounded by the same LRU cap as
/// pending auth state. A client that loses its `client_id` re-registers; replay
/// risk is bounded by the per-`state` single-use guard already in place.
pub async fn oauth_register_handler(
    State(state): State<Arc<OAuthState>>,
    body: axum::body::Bytes,
) -> Response {
    // Parse request body opportunistically — most fields are echoed back as-is
    // per RFC 7591. Tolerate empty / non-JSON bodies (return defaults).
    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    let client_id = uuid::Uuid::new_v4().to_string();
    let issued_at = chrono::Utc::now().timestamp();

    // Echo back the registration metadata RFC 7591 §3.2.1 spec-mandates.
    // Empty / missing fields use sane defaults.
    let redirect_uris = req
        .get("redirect_uris")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let registered_redirects = redirect_uris
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let client_name = req
        .get("client_name")
        .and_then(|v| v.as_str())
        .unwrap_or("mcp-client")
        .to_string();
    let grant_types = req
        .get("grant_types")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(["authorization_code"]));
    let response_types = req
        .get("response_types")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(["code"]));
    let token_endpoint_auth_method = req
        .get("token_endpoint_auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string();

    let resp = serde_json::json!({
        "client_id": client_id.clone(),
        "client_id_issued_at": issued_at,
        "redirect_uris": redirect_uris,
        "client_name": client_name,
        "grant_types": grant_types,
        "response_types": response_types,
        "token_endpoint_auth_method": token_endpoint_auth_method,
    });

    state.register_client(client_id, registered_redirects);

    (StatusCode::CREATED, Json(resp)).into_response()
}

// ── Bearer-auth middleware (Decision 9) ──────────────────────────────────────

/// Maximum body size accepted by the body-peeking middleware (1 MiB).
/// Larger bodies short-circuit with HTTP 413; the JSON-RPC dispatcher's own
/// limit is 2 MiB so this is the tighter gate for request inspection.
const MAX_PEEK_BODY: usize = 1024 * 1024;

/// Allowlist of JSON-RPC methods that bypass JWT validation. `initialize`
/// and `tools/list` are required for MCP discovery — Cursor / Claude.ai
/// post these BEFORE completing OAuth, so blocking them breaks the install
/// handshake. JSON-RPC notifications (no `id`) and MCP `notifications/*`
/// methods (e.g. `notifications/initialized`, `notifications/cancelled`,
/// `notifications/progress`) are also pass-through: they are sent during
/// the connection lifecycle and rejecting them with 401 breaks
/// streamable-HTTP transport (observed during T15 — Cursor sends
/// `notifications/initialized` immediately after `initialize` response).
///
/// The four `prompts/*` and `resources/*` methods join the allowlist as
/// part of the agent-native-distribution feature: skill manifests and
/// their rendered markdown are public read-only discovery surfaces and
/// must work without OAuth so MCP clients can preview them before the
/// user signs in. The dispatcher only reads from compile-time embedded
/// constants on these paths — no per-tenant data ever leaves the server,
/// no storage is touched, so the anonymous read is safe.
const ALLOWLIST_METHODS: &[&str] = &[
    "initialize",
    "tools/list",
    "ping",
    "prompts/list",
    "prompts/get",
    "resources/list",
    "resources/read",
];

/// Extract the JSON-RPC `method` field from a request body without consuming
/// the parser state. Returns `None` if the body is not valid JSON or the
/// `method` field is missing/non-string. Cheap — does not allocate the full
/// parsed structure beyond what serde_json's lazy `Value` requires.
pub fn extract_json_rpc_method(bytes: &[u8]) -> Option<String> {
    let val: Value = serde_json::from_slice(bytes).ok()?;
    val.get("method")?.as_str().map(|s| s.to_string())
}

/// Extract `params.name` (MCP tool name) from a `tools/call` body. Returns
/// `None` for any other method, invalid JSON, or missing `name`. Used by the
/// bearer-auth middleware to allow anonymous `mnemonic_recall` calls per
/// Decision 5 / AC13 (agent-native-distribution). The recall handler enforces
/// visibility filtering downstream; the middleware just gates which tool
/// names are reachable without a JWT.
pub fn extract_tools_call_name(bytes: &[u8]) -> Option<String> {
    let val: Value = serde_json::from_slice(bytes).ok()?;
    if val.get("method")?.as_str()? != "tools/call" {
        return None;
    }
    val.get("params")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

/// Tool names that pass `bearer_auth_middleware` without a JWT (the
/// `tools/call` analogue of `ALLOWLIST_METHODS`). The downstream handler in
/// `mcp::handle_tool_call` enforces tool-specific access rules
/// (e.g. `mnemonic_recall` filters by `visibility='public'` when
/// `jwt_sub` is absent — AC13).
///
/// Only `mnemonic_recall` belongs here in v1 of agent-native-distribution.
/// Adding a new entry expands the anonymous attack surface — review
/// carefully and pair with explicit visibility / tenancy guards in the
/// downstream handler.
const ALLOWLIST_TOOLS_CALL_NAMES: &[&str] = &["mnemonic_recall"];

/// Bearer-auth middleware. Inserts the resolved `Claims` into the request
/// extension on success so downstream handlers can read `jwt.sub` via
/// `Request::extensions()` / `axum::Extension(Claims)`.
///
/// On `/oauth/*` and `/health` the body is never read — those routes are
/// allowlisted by URI path. For `/mcp` the body is buffered (capped at 1 MiB)
/// and parsed for the JSON-RPC `method` field — `initialize` and `tools/list`
/// pass through; everything else demands a valid Bearer JWT.
pub async fn bearer_auth_middleware(
    State(state): State<Arc<OAuthState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    // URI-based allowlist — never inspect the body for these. `.well-known/`
    // endpoints (RFC 8414 OAuth discovery + MCP protected-resource metadata)
    // are public by design — Anthropic's MCP connector probes them BEFORE
    // attempting OAuth.
    //
    // `/api/pending/*` and `/api/sign-callback` are also public — Decision 12's
    // browser-mediated signing flow opens these from the user's webapp on
    // `mnemonik.xyz`, which has its OWN identity (the localStorage keypair)
    // but does NOT have the JWT that the AI-tool client (Cursor/VS Code/etc.)
    // received from /oauth/token. Auth on these endpoints is enforced via the
    // cryptographic chain instead: `correlation_id` is a capability, and
    // `sign-callback` validates `signer_pubkey == entry.jwt_sub` plus a valid
    // COSE_Sign1 signature over the canonical-CBOR bundle. An attacker
    // holding only a guessed correlation_id cannot forge a valid sign-back
    // because they don't have the user's private key.
    if path.starts_with("/oauth/")
        || path == "/health"
        || path.starts_with("/.well-known/")
        || path.starts_with("/api/pending/")
        || path == "/api/sign-callback"
        // CLI bootstrap-ticket redeem endpoint (mnemonic-cli tech-spec
        // Decision 7). The UUID in the path IS the capability — the CLI
        // does not yet have a JWT at redeem time (the whole point of
        // this flow is to bootstrap one). The /issue counterpart is NOT
        // on this allowlist; it uses standard Bearer-JWT auth so only an
        // already-authenticated webapp can mint tickets for its own user.
        || path.starts_with("/api/cli-bootstrap/redeem/")
        // POST /api/cli-bootstrap/redeem — short_code lookup (T13/T14 interop).
        // Same no-auth semantics as the UUID-based GET variant: the short_code
        // is the capability. Exact-path match so it doesn't shadow sub-paths.
        || path == "/api/cli-bootstrap/redeem"
        // CLI-origin ticket issue: the CLI has not yet redeemed a JWT — the
        // x25519 wrap to the server's static key is the capability. No Bearer
        // JWT required.
        || path == "/api/cli-bootstrap/issue-from-cli"
        // Server's static x25519 public key — needed before issuing a ticket.
        || path == "/api/cli-bootstrap/server-pub"
        // Extension bootstrap-ticket redeem endpoint (chrome-extension T15,
        // Decision 9). Same UUID-as-capability model as cli-bootstrap; the
        // extension exchanges the ticket for a fresh `aud=extension` JWT
        // without sending the webapp's `aud=mcp` JWT (which would not be
        // present in the extension service worker's request context).
        || path.starts_with("/api/extension-bootstrap/redeem/")
        // Key-escrow endpoints verify their own `aud=extension` JWTs inline
        // (the production bearer-auth middleware only accepts `aud=mcp`).
        || path == "/api/key-escrow"
    {
        return next.run(request).await;
    }

    // Buffer the body for method inspection AND downstream re-injection.
    // axum's `Body` is consumed by `to_bytes`, so we must rebuild the request
    // from the collected bytes before forwarding.
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_PEEK_BODY).await {
        Ok(b) => b,
        Err(e) => {
            return jsonrpc_unauthorized(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("body too large or unreadable: {e}"),
            );
        }
    };

    let method = extract_json_rpc_method(&body_bytes);
    // Tool name extracted iff method == "tools/call". `None` for every other
    // method. Used to pick out the per-tool allowlist (AC13 anonymous
    // recall) without expanding the method allowlist.
    let tools_call_name = extract_tools_call_name(&body_bytes);
    let allowlisted = method
        .as_deref()
        .map(|m| {
            // Explicit allowlist: discovery + lifecycle methods.
            // Notifications (per JSON-RPC 2.0 spec — no `id` field) and MCP
            // `notifications/*` methods are also pass-through. Without this
            // Cursor's post-initialize `notifications/initialized` is rejected
            // with 401, breaking the handshake.
            //
            // Decision 5 / AC13 — agent-native-distribution: `tools/call`
            // with a tool name listed in `ALLOWLIST_TOOLS_CALL_NAMES` is
            // also allowlisted. Only `mnemonic_recall` is on that list in
            // v1; the downstream handler enforces visibility filtering so
            // anonymous callers see only `visibility='public'` rows.
            ALLOWLIST_METHODS.contains(&m)
                || m.starts_with("notifications/")
                || (m == "tools/call"
                    && tools_call_name
                        .as_deref()
                        .map(|t| ALLOWLIST_TOOLS_CALL_NAMES.contains(&t))
                        .unwrap_or(false))
        })
        .unwrap_or(false);

    // Try to extract a Bearer JWT from the Authorization header. We do this
    // for BOTH gated and allowlisted requests so the downstream handler can
    // see `Claims` when present — the allowlist only relaxes "JWT MUST be
    // present and valid", it does not mean "ignore the JWT if the client
    // sent one". Allowlisted discovery methods (`initialize` / `tools/list`)
    // may still arrive with a Bearer token mid-session; downstream code can
    // branch on `Claims` if it cares.
    let bearer = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    if !allowlisted {
        // Gated path — JWT is required AND must verify.
        let token = match bearer {
            Some(t) if !t.is_empty() => t,
            _ => return jsonrpc_unauthorized(StatusCode::UNAUTHORIZED, "missing Bearer JWT"),
        };
        let claims = match verify_jwt(&state, &token) {
            Ok(c) => c,
            Err(e) => {
                return jsonrpc_unauthorized(
                    StatusCode::UNAUTHORIZED,
                    &format!("invalid JWT: {e}"),
                );
            }
        };
        // Re-inject the body and attach Claims for downstream handlers.
        let mut new_req = Request::from_parts(parts, Body::from(body_bytes));
        new_req.extensions_mut().insert(claims);
        return next.run(new_req).await;
    }

    // Allowlisted path — JWT is OPTIONAL. If a Bearer header is present and
    // verifies, attach Claims so downstream handlers that want the caller
    // identity can branch on it. If absent or invalid, proceed without
    // Claims (allowlisted requests must not 401 on bad tokens — discovery
    // methods are reached before the client has a token).
    let mut new_req = Request::from_parts(parts, Body::from(body_bytes));
    if let Some(token) = bearer.filter(|t| !t.is_empty()) {
        if let Ok(claims) = verify_jwt(&state, &token) {
            new_req.extensions_mut().insert(claims);
        }
    }
    next.run(new_req).await
}

/// Emit a JSON-RPC-shaped 401 envelope for failed bearer-auth checks.
///
/// MCP authorization spec + RFC 6750 §3 require a `WWW-Authenticate` header
/// on any 401 from a Bearer-protected resource. The `resource_metadata`
/// parameter tells the MCP client where the protected-resource metadata
/// lives (`/.well-known/oauth-protected-resource`); without this header,
/// some MCP-OAuth clients (Cursor's recent versions) fail the connection
/// silently instead of prompting the user to authenticate.
///
/// We always emit the same realm + resource_metadata pair regardless of
/// `status` because the client behavior is the same for any 401-class auth
/// failure (missing/invalid/expired Bearer).
fn jsonrpc_unauthorized(status: StatusCode, msg: &str) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": -32001, "message": format!("unauthorized: {msg}")}
    });
    let mut resp = (status, Json(body)).into_response();
    if status == StatusCode::UNAUTHORIZED {
        // Must be a single header value per RFC 7235 §4.1. Choose `error=` per
        // RFC 6750 §3.1 to match invalid_token semantics; `error_description`
        // is the human-readable hint the client may surface to the user.
        // `issuer` reads from the env-driven OnceLock so non-prod deploys
        // (cloudflared tunnel, dev subdomain, third-party operator) advertise
        // the correct resource_metadata URL instead of the prod default.
        // `compute_server_origin_from_env_str` already rejects CRLF / control
        // chars and userinfo, so the value cannot break the
        // `HeaderValue::from_str` parse — the SA-R1-L1 fix.
        let www_auth = format!(
            "Bearer realm=\"{issuer}\", error=\"invalid_token\", \
             error_description=\"{esc_msg}\", \
             resource_metadata=\"{issuer}/.well-known/oauth-protected-resource\"",
            issuer = server_origin(),
            // Strip embedded double-quotes / CR / LF from the message to keep
            // the header well-formed; we don't expect any in caller-supplied
            // strings but defense-in-depth is cheap.
            esc_msg = msg.replace('"', "'").replace(['\r', '\n'], " "),
        );
        if let Ok(hv) = axum::http::HeaderValue::from_str(&www_auth) {
            resp.headers_mut()
                .insert(axum::http::header::WWW_AUTHENTICATE, hv);
        }
    }
    resp
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Decision-9/10/11 unit tests — 15 OAuth + 3 PKCE/JWT-shape tests in
    //! all. Network-free: routes are exercised via `tower::ServiceExt::oneshot`
    //! against a router built in-test. The COSE_Sign1 challenge is signed
    //! using the same `mnemonic_core::codec::sign::sign_cose` primitive the
    //! browser WASM uses, so test fidelity matches production exactly.

    use super::*;
    use axum::{middleware as axum_middleware, routing::post, Router};
    use http_body_util::BodyExt;
    use mnemonic_core::codec::sign::sign_cose;
    use solana_sdk::signature::{Keypair, Signer};
    use tower::ServiceExt;

    /// 32-byte test secret. Matches the production length requirement.
    const TEST_SECRET: &[u8; 32] = b"unit-test-secret-32-bytes-long!!";

    fn fresh_state() -> Arc<OAuthState> {
        Arc::new(OAuthState::with_defaults(TEST_SECRET))
    }

    /// Build a signed challenge for a given keypair + state. Returns
    /// `(challenge_hash, cose_signed_base64)`. Pubkey is read from `kp` by
    /// the caller via `kp.pubkey().to_string()`.
    fn make_signed_challenge(
        kp: &Keypair,
        client_state: &str,
        redirect_uri: &str,
        code_challenge: &str,
        nonce: &str,
        exp: u64,
    ) -> (String, String) {
        let challenge = serde_json::json!({
            "server_origin": server_origin(),
            "state": client_state,
            "client_id": "test-client",
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": "S256",
            "nonce": nonce,
            "exp": exp,
        });
        let cbor = to_canonical_cbor(&challenge, &CHALLENGE_SCHEMA).unwrap();
        let hash = blake3_hex(&cbor);
        let cose = sign_cose(&cbor, kp).expect("sign_cose");
        let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cose);
        (hash, cose_b64)
    }

    /// Build a raw-Ed25519 signed challenge for a given keypair + state.
    /// Mirrors `make_signed_challenge` but signs the canonical-CBOR bytes
    /// directly with Ed25519 (no COSE_Sign1 wrap), returning the base64 of
    /// the raw 64-byte signature plus the canonical-CBOR bytes the caller
    /// must store as `pending.challenge_bytes` (the handler verifies the
    /// signature against those exact bytes). This matches the post-refactor
    /// wire contract enforced at `authorize_handler` (signature MUST be 64
    /// bytes).
    fn make_raw_signed_challenge(
        kp: &Keypair,
        client_state: &str,
        redirect_uri: &str,
        code_challenge: &str,
        nonce: &str,
        exp: u64,
    ) -> (String, String, Vec<u8>) {
        let challenge = serde_json::json!({
            "server_origin": server_origin(),
            "state": client_state,
            "client_id": "test-client",
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": "S256",
            "nonce": nonce,
            "exp": exp,
        });
        let cbor = to_canonical_cbor(&challenge, &CHALLENGE_SCHEMA).unwrap();
        let hash = blake3_hex(&cbor);
        let sig = mnemonic_core::identity::sign_bytes(kp, &cbor);
        let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig);
        (hash, sig_b64, cbor)
    }

    fn build_authorize_router(state: Arc<OAuthState>) -> Router {
        Router::new()
            .route("/oauth/authorize", post(authorize_handler))
            .route("/oauth/token", post(token_handler))
            .with_state(state)
    }

    async fn post_json(app: Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&body_bytes).into()));
        (status, parsed)
    }

    /// Helper: PKCE S256 challenge from verifier.
    fn pkce_challenge(verifier: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(verifier.as_bytes());
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest)
    }

    #[tokio::test]
    async fn test_authorize_valid_signature() {
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let verifier = "test-verifier-min-43-chars-long-aaaaaaaaaaa";
        let challenge = pkce_challenge(verifier);
        let (hash, sig_b64, cbor) = make_raw_signed_challenge(
            &kp,
            "csrf-state-1",
            "https://app/callback",
            &challenge,
            "nonce-1",
            now_secs() + 30,
        );
        st.insert_pending(
            "csrf-state-1".to_string(),
            hash,
            cbor,
            pubkey.clone(),
            "https://app/callback".to_string(),
            challenge.clone(),
            "csrf-state-1".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (status, body) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-state-1",
                "signature": sig_b64,
                "signer_pubkey": pubkey,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={body}");
        assert!(body["code"].as_str().unwrap().len() > 8);
        assert_eq!(body["state"], "csrf-state-1");
        assert_eq!(body["redirect_uri"], "https://app/callback");
    }

    #[tokio::test]
    async fn test_authorize_invalid_signature_401() {
        let st = fresh_state();
        let kp = Keypair::new();
        let attacker = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let challenge = pkce_challenge("v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (hash, _good_sig, cbor) = make_raw_signed_challenge(
            &kp,
            "csrf-2",
            "https://app/cb",
            &challenge,
            "n2",
            now_secs() + 30,
        );
        // Tamper: re-sign the SAME canonical CBOR with the attacker's key
        // (raw Ed25519, no COSE wrap — matches the post-refactor wire
        // contract). Hash matches the pending entry, but the signature
        // will not verify under the bound `expected_pubkey`.
        let bad_sig = mnemonic_core::identity::sign_bytes(&attacker, &cbor);
        let bad_sig_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bad_sig);
        let attacker_pubkey = attacker.pubkey().to_string();
        st.insert_pending(
            "csrf-2".to_string(),
            hash,
            cbor,
            pubkey,
            "https://app/cb".to_string(),
            challenge,
            "csrf-2".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-2",
                "signature": bad_sig_b64,
                "signer_pubkey": attacker_pubkey,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authorize_tampered_sub_401() {
        // Pending record expects pubkey A, but the challenge is signed by
        // pubkey B (different keypair). signer != expected_pubkey → 401.
        let st = fresh_state();
        let kp_b = Keypair::new();
        let kp_b_pubkey = kp_b.pubkey().to_string();
        let pubkey_a = Keypair::new().pubkey().to_string(); // unrelated
        let challenge = pkce_challenge("v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (hash, sig_b64, cbor) = make_raw_signed_challenge(
            &kp_b,
            "csrf-tamper",
            "https://app/cb",
            &challenge,
            "n",
            now_secs() + 30,
        );
        st.insert_pending(
            "csrf-tamper".to_string(),
            hash,
            cbor,
            pubkey_a, // attacker's keypair signed but we expect somebody else
            "https://app/cb".to_string(),
            challenge,
            "csrf-tamper".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-tamper",
                "signature": sig_b64,
                "signer_pubkey": kp_b_pubkey,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authorize_expired_challenge_401() {
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let challenge = pkce_challenge("v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        // exp set to a past timestamp.
        let past = now_secs().saturating_sub(120);
        let (hash, cose_b64) =
            make_signed_challenge(&kp, "csrf-exp", "https://app/cb", &challenge, "n", past);
        st.insert_pending(
            "csrf-exp".to_string(),
            hash,
            Vec::new(),
            pubkey,
            "https://app/cb".to_string(),
            challenge,
            "csrf-exp".to_string(),
            past,
        );
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({"state": "csrf-exp", "cose_signed": cose_b64}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authorize_unknown_state_401() {
        let st = fresh_state();
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({"state": "never-seen", "cose_signed": "AAAA"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authorize_pkce_method_must_be_s256() {
        // Decision 10 — code_challenge_method other than S256 is rejected.
        // We model this by signing a challenge with method="plain" and
        // verifying that the server's challenge-hash check fails (the
        // server will compute `to_canonical_cbor` over a different field
        // set on its side, so the hashes diverge).
        //
        // Note: the policy gate "S256-only" is enforced upstream of
        // /authorize (the consent-page bootstrap rejects non-S256 before
        // inserting pending). To test the protocol-layer behaviour, we
        // demonstrate that a "plain" challenge does NOT verify: build
        // pending with S256 metadata, then sign a payload that says "plain"
        // → verify_artifact's content_integrity flag flips false → 401.
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let plain_challenge = "plain-challenge-string";
        // Pending stores S256 expectations.
        let s256_hash = build_challenge_hash(
            server_origin(),
            "csrf-plain",
            "test-client",
            "https://app/cb",
            plain_challenge,
            "S256",
            "n",
            now_secs() + 30,
        )
        .unwrap();
        st.insert_pending(
            "csrf-plain".to_string(),
            s256_hash,
            Vec::new(),
            pubkey.clone(),
            "https://app/cb".to_string(),
            plain_challenge.to_string(),
            "csrf-plain".to_string(),
            now_secs() + 30,
        );
        // But the user signs a "plain" envelope (raw Ed25519 over the
        // mismatched CBOR — handler verifies against the pending entry's
        // S256 challenge_bytes which was never written, so verification
        // fails on the empty-bytes path).
        let bad_obj = serde_json::json!({
            "server_origin": server_origin(),
            "state": "csrf-plain",
            "client_id": "test-client",
            "redirect_uri": "https://app/cb",
            "code_challenge": plain_challenge,
            "code_challenge_method": "plain",
            "nonce": "n",
            "exp": now_secs() + 30,
        });
        let cbor = to_canonical_cbor(&bad_obj, &CHALLENGE_SCHEMA).unwrap();
        let sig = mnemonic_core::identity::sign_bytes(&kp, &cbor);
        let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig);
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-plain",
                "signature": sig_b64,
                "signer_pubkey": pubkey,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_authorize_single_use_replay_401() {
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let challenge = pkce_challenge("v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (hash, sig_b64, cbor) = make_raw_signed_challenge(
            &kp,
            "csrf-replay",
            "https://app/cb",
            &challenge,
            "n",
            now_secs() + 30,
        );
        st.insert_pending(
            "csrf-replay".to_string(),
            hash,
            cbor,
            pubkey.clone(),
            "https://app/cb".to_string(),
            challenge,
            "csrf-replay".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (status1, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-replay",
                "signature": sig_b64.clone(),
                "signer_pubkey": pubkey.clone(),
            }),
        )
        .await;
        assert_eq!(status1, StatusCode::OK);
        // Second submission of the same `state` must fail — entry is gone.
        let app2 = build_authorize_router(st);
        let (status2, _) = post_json(
            app2,
            "/oauth/authorize",
            serde_json::json!({
                "state": "csrf-replay",
                "signature": sig_b64,
                "signer_pubkey": pubkey,
            }),
        )
        .await;
        assert_eq!(status2, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_token_valid_verifier_returns_jwt() {
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let verifier = "v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let challenge = pkce_challenge(verifier);
        let (hash, sig_b64, cbor) = make_raw_signed_challenge(
            &kp,
            "tok-state",
            "https://app/cb",
            &challenge,
            "n",
            now_secs() + 30,
        );
        st.insert_pending(
            "tok-state".to_string(),
            hash,
            cbor,
            pubkey.clone(),
            "https://app/cb".to_string(),
            challenge,
            "tok-state".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (s1, body) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "tok-state",
                "signature": sig_b64,
                "signer_pubkey": pubkey.clone(),
            }),
        )
        .await;
        assert_eq!(s1, StatusCode::OK);
        let code = body["code"].as_str().unwrap().to_string();
        let app2 = build_authorize_router(st.clone());
        let (s2, body2) = post_json(
            app2,
            "/oauth/token",
            serde_json::json!({"code": code, "code_verifier": verifier}),
        )
        .await;
        assert_eq!(s2, StatusCode::OK);
        let token = body2["access_token"].as_str().unwrap().to_string();
        let claims = verify_jwt(&st, &token).unwrap();
        assert_eq!(claims.sub, pubkey);
    }

    #[tokio::test]
    async fn test_token_invalid_verifier_401() {
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let verifier = "v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let challenge = pkce_challenge(verifier);
        let (hash, sig_b64, cbor) = make_raw_signed_challenge(
            &kp,
            "tok-bad",
            "https://app/cb",
            &challenge,
            "n",
            now_secs() + 30,
        );
        st.insert_pending(
            "tok-bad".to_string(),
            hash,
            cbor,
            pubkey.clone(),
            "https://app/cb".to_string(),
            challenge,
            "tok-bad".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (_, body) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "tok-bad",
                "signature": sig_b64,
                "signer_pubkey": pubkey,
            }),
        )
        .await;
        let code = body["code"].as_str().unwrap().to_string();
        let app2 = build_authorize_router(st);
        let (s, _) = post_json(
            app2,
            "/oauth/token",
            serde_json::json!({"code": code, "code_verifier": "wrong-verifier"}),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_token_expired_code_60s_401() {
        let st = fresh_state();
        // Insert a code directly with `exp` in the past.
        {
            let mut g = st.codes.lock().unwrap();
            g.put(
                "expired-code".to_string(),
                IssuedCode {
                    sub: "test-pubkey".to_string(),
                    code_challenge: pkce_challenge("v"),
                    redirect_uri: "https://app/cb".to_string(),
                    google_sub: None,
                    exp: now_secs().saturating_sub(120),
                },
            );
        }
        let app = build_authorize_router(st);
        let (s, _) = post_json(
            app,
            "/oauth/token",
            serde_json::json!({"code": "expired-code", "code_verifier": "v"}),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_jwt_roundtrip_iss_aud_sub() {
        let st = fresh_state();
        let token = issue_jwt(&st, "test-pubkey-base58").unwrap();
        let claims = verify_jwt(&st, &token).unwrap();
        assert_eq!(claims.iss, JWT_ISSUER);
        assert_eq!(claims.aud, JWT_AUDIENCE);
        assert_eq!(claims.sub, "test-pubkey-base58");
        assert!(claims.exp > claims.iat);
        // jti must be a UUID — parse it.
        let parsed = uuid::Uuid::parse_str(&claims.jti).expect("jti is uuid");
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn test_jwt_alg_none_rejected_401() {
        let st = fresh_state();
        // Build a header with alg="none" and forge an unsigned token.
        // Format: base64url(header).base64url(payload).
        let header = serde_json::json!({"alg": "none", "typ": "JWT"});
        let claims = Claims {
            iss: JWT_ISSUER.to_string(),
            aud: JWT_AUDIENCE.to_string(),
            sub: "evil-sub".to_string(),
            iat: now_secs(),
            exp: now_secs() + 3600,
            jti: uuid::Uuid::new_v4().to_string(),
            google_sub: None,
        };
        let h_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            serde_json::to_vec(&header).unwrap(),
        );
        let p_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            serde_json::to_vec(&claims).unwrap(),
        );
        let alg_none_token = format!("{h_b64}.{p_b64}.");
        let result = verify_jwt(&st, &alg_none_token);
        assert!(result.is_err(), "alg=none must be rejected");
    }

    #[test]
    fn test_jwt_iss_aud_mismatch_rejected() {
        let st = fresh_state();
        // Build a JWT with the same secret but wrong iss / aud.
        let header = Header::new(Algorithm::HS256);
        let evil = Claims {
            iss: "evil".to_string(),
            aud: JWT_AUDIENCE.to_string(),
            sub: "x".to_string(),
            iat: now_secs(),
            exp: now_secs() + 3600,
            jti: uuid::Uuid::new_v4().to_string(),
            google_sub: None,
        };
        let token = encode(&header, &evil, &EncodingKey::from_secret(TEST_SECRET)).unwrap();
        assert!(
            verify_jwt(&st, &token).is_err(),
            "iss=evil must be rejected"
        );

        let bad_aud = Claims {
            iss: JWT_ISSUER.to_string(),
            aud: "other".to_string(),
            sub: "x".to_string(),
            iat: now_secs(),
            exp: now_secs() + 3600,
            jti: uuid::Uuid::new_v4().to_string(),
            google_sub: None,
        };
        let token2 = encode(&header, &bad_aud, &EncodingKey::from_secret(TEST_SECRET)).unwrap();
        assert!(
            verify_jwt(&st, &token2).is_err(),
            "aud=other must be rejected"
        );
    }

    #[test]
    fn test_jwt_concurrent_unique_jti() {
        let st = fresh_state();
        let t1 = issue_jwt(&st, "p").unwrap();
        let t2 = issue_jwt(&st, "p").unwrap();
        let c1 = verify_jwt(&st, &t1).unwrap();
        let c2 = verify_jwt(&st, &t2).unwrap();
        assert_ne!(c1.jti, c2.jti, "consecutive jti must differ");
    }

    // ── /oauth/authorize-init (bootstrap) tests ──────────────────────────────

    fn build_init_router(state: Arc<OAuthState>) -> Router {
        use axum::routing::get;
        Router::new()
            .route(
                "/oauth/authorize",
                get(authorize_init_handler).post(authorize_handler),
            )
            .with_state(state)
    }

    fn build_init_register_router(state: Arc<OAuthState>) -> Router {
        use axum::routing::get;
        Router::new()
            .route(
                "/oauth/authorize",
                get(authorize_init_handler).post(authorize_handler),
            )
            .route("/oauth/register", post(oauth_register_handler))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_authorize_init_creates_pending() {
        // Given a fresh OAuthState, GET /oauth/authorize?...code_challenge_method=S256
        // with Accept: application/json must respond 200, return a base64-encoded
        // canonical-CBOR challenge, and insert exactly one pending entry under
        // the provided `state`.
        let st = fresh_state();
        let app = build_init_router(st.clone());
        let req = Request::builder()
            .method("GET")
            .uri(
                "/oauth/authorize\
                 ?client_id=cursor\
                 &redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fcb\
                 &code_challenge=abc123\
                 &code_challenge_method=S256\
                 &state=csrf-init-1\
                 &response_type=code",
            )
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed["state"], "csrf-init-1");
        assert!(parsed["challenge_cbor"].as_str().unwrap().len() > 8);
        assert!(parsed["exp"].as_u64().unwrap() > now_secs());
        // Pending map now contains the entry.
        let pending_present = {
            let g = st.pending.lock().unwrap();
            g.peek("csrf-init-1").is_some()
        };
        assert!(pending_present, "insert_pending must have been called");
    }

    #[tokio::test]
    async fn test_authorize_init_rejects_plain_pkce() {
        let st = fresh_state();
        let app = build_init_router(st);
        let req = Request::builder()
            .method("GET")
            .uri(
                "/oauth/authorize\
                 ?client_id=cursor\
                 &redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fcb\
                 &code_challenge=abc\
                 &code_challenge_method=plain\
                 &state=csrf-plain-rejected",
            )
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_authorize_init_browser_redirects_to_consent() {
        // No Accept header + no `pubkey` → 302 to the webapp consent page.
        let st = fresh_state();
        let app = build_init_router(st.clone());
        let req = Request::builder()
            .method("GET")
            .uri(
                "/oauth/authorize\
                 ?client_id=cursor\
                 &redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fcb\
                 &code_challenge=abc\
                 &code_challenge_method=S256\
                 &state=csrf-redir",
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        let location = resp
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(location.starts_with(WEBAPP_CONSENT_URL), "got {location}");
        assert!(location.contains("challenge="));
        assert!(location.contains("state=csrf-redir"));
        assert!(
            location.contains("mcp_base=https%3A%2F%2Fmcp.mnemonik.xyz"),
            "redirect must tell the hosted consent page which MCP origin owns the state: {location}"
        );
        // Pending entry inserted regardless of redirect vs JSON mode.
        let present = {
            let g = st.pending.lock().unwrap();
            g.peek("csrf-redir").is_some()
        };
        assert!(present);
    }

    #[tokio::test]
    async fn test_authorize_init_then_post_round_trip() {
        // End-to-end: GET /oauth/authorize with a pubkey query param (so the
        // expected_pubkey binding is set); decode the returned base64 CBOR;
        // sign it with the matching keypair; POST /oauth/authorize succeeds.
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let app = build_init_router(st.clone());

        let init_uri = format!(
            "/oauth/authorize?client_id=cursor&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fcb\
             &code_challenge=ch&code_challenge_method=S256&state=rt-state\
             &pubkey={pubkey}"
        );
        let req = Request::builder()
            .method("GET")
            .uri(init_uri)
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap();
        let cbor_b64 = parsed["challenge_cbor"].as_str().unwrap().to_string();
        let cbor_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cbor_b64).unwrap();

        // Sign the canonical-CBOR with the user's keypair (raw Ed25519 — no
        // COSE wrap, matching the post-refactor wire contract).
        let sig = mnemonic_core::identity::sign_bytes(&kp, &cbor_bytes);
        let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig);

        // POST /oauth/authorize → success (200 with `code`).
        let app2 = build_init_router(st);
        let post_req = Request::builder()
            .method("POST")
            .uri("/oauth/authorize")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "state": "rt-state",
                    "signature": sig_b64,
                    "signer_pubkey": pubkey,
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp2 = app2.oneshot(post_req).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_oauth_register_then_authorize_accepts_vscode_redirects() {
        let app = build_init_register_router(fresh_state());

        let register_req = Request::builder()
            .method("POST")
            .uri("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "client_name": "VS Code",
                    "redirect_uris": [
                        "https://vscode.dev/redirect",
                        "http://127.0.0.1:33418/"
                    ],
                    "grant_types": ["authorization_code"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": "none"
                })
                .to_string(),
            ))
            .unwrap();
        let register_resp = app.clone().oneshot(register_req).await.unwrap();
        assert_eq!(register_resp.status(), StatusCode::CREATED);
        let register_body: Value = serde_json::from_slice(
            &register_resp
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes(),
        )
        .unwrap();
        let client_id = register_body["client_id"].as_str().expect("client_id");

        let authorize_uri = format!(
            "/oauth/authorize\
             ?client_id={client_id}\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A59656%2F\
             &code_challenge=abc\
             &code_challenge_method=S256\
             &state=vs-code-state"
        );
        let authorize_req = Request::builder()
            .method("GET")
            .uri(authorize_uri)
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let authorize_resp = app.oneshot(authorize_req).await.unwrap();
        assert_eq!(authorize_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_authorize_missing_state_csrf_401() {
        let st = fresh_state();
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            // No `state` field in body.
            serde_json::json!({"cose_signed": "AAAA"}),
        )
        .await;
        // serde rejects missing required field → 422 Unprocessable Entity
        // from axum's Json extractor. We assert any 4xx.
        assert!(status.is_client_error(), "got status {status}");
    }

    // ── bearer_auth_middleware tests ─────────────────────────────────────────

    /// Build a tiny router with a `/mcp` POST handler that returns 200 only
    /// if reached, plus the bearer-auth middleware in front.
    fn build_authn_router(state: Arc<OAuthState>) -> Router {
        async fn ok() -> &'static str {
            "ok"
        }
        Router::new()
            .route("/mcp", post(ok))
            .route("/health", axum::routing::get(|| async { "ok" }))
            .route("/oauth/authorize", post(authorize_handler))
            .layer(axum_middleware::from_fn_with_state(
                state.clone(),
                bearer_auth_middleware,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_middleware_initialize_method_allowlisted() {
        let st = fresh_state();
        let app = build_authn_router(st);
        let body = serde_json::json!({"jsonrpc": "2.0", "method": "initialize", "id": 1});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_tools_call_requires_jwt() {
        let st = fresh_state();
        let app = build_authn_router(st);
        let body = serde_json::json!({"jsonrpc": "2.0", "method": "tools/call", "id": 1});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_middleware_tools_call_with_valid_jwt_passes() {
        let st = fresh_state();
        let token = issue_jwt(&st, "test-sub").unwrap();
        let app = build_authn_router(st);
        let body = serde_json::json!({"jsonrpc": "2.0", "method": "tools/call", "id": 1});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_health_bypasses_auth() {
        let st = fresh_state();
        let app = build_authn_router(st);
        let req = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_extract_json_rpc_method_helper() {
        let body = b"{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"id\":1}";
        assert_eq!(extract_json_rpc_method(body).as_deref(), Some("tools/list"));
        assert_eq!(extract_json_rpc_method(b"not-json"), None);
        assert_eq!(extract_json_rpc_method(b"{}"), None);
    }

    // ── /.well-known discovery tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_oauth_authorization_server_metadata() {
        // RFC 8414 — Anthropic's MCP connector probes this endpoint before
        // starting the OAuth flow. Verify the JSON shape exposes all required
        // fields with stable values.
        use axum::routing::get;
        let app: Router = Router::new().route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server_metadata),
        );
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap();
        // Anchor to SERVER_ORIGIN_DEFAULT directly (TR-R1-M3): asserting
        // against `server_origin()` would be reflexively true — both the
        // endpoint and the assertion would derive from the same OnceLock
        // read, so a seeding regression would produce matching-but-wrong
        // values. The test never calls `seed_server_origin_from_env`, so the
        // OnceLock is either unset or seeded to the default by a sibling
        // test — either way `server_origin()` MUST return the default here.
        assert_eq!(parsed["issuer"], SERVER_ORIGIN_DEFAULT);
        assert_eq!(
            parsed["authorization_endpoint"],
            format!("{SERVER_ORIGIN_DEFAULT}/oauth/authorize")
        );
        assert_eq!(
            parsed["token_endpoint"],
            format!("{SERVER_ORIGIN_DEFAULT}/oauth/token")
        );
        assert_eq!(parsed["response_types_supported"][0], "code");
        // refresh-token-rotation Task 3 / Decision 11 — discovery MUST
        // advertise BOTH grants so MCP clients can opt into the
        // rotation flow. Use set-membership (TR3-R1-L2) rather than
        // positional indexing so re-ordering the array doesn't break
        // the test.
        let grants: Vec<&str> = parsed["grant_types_supported"]
            .as_array()
            .expect("grant_types_supported must be an array")
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        assert!(grants.contains(&"authorization_code"), "grants={grants:?}");
        assert!(grants.contains(&"refresh_token"), "grants={grants:?}");
        assert_eq!(parsed["code_challenge_methods_supported"][0], "S256");
        assert_eq!(parsed["token_endpoint_auth_methods_supported"][0], "none");
        assert_eq!(parsed["scopes_supported"][0], "mcp");
    }

    #[tokio::test]
    async fn test_oauth_protected_resource_metadata() {
        use axum::routing::get;
        let app: Router = Router::new().route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource_metadata),
        );
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-protected-resource")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap();
        // TR-R1-M3 — anchor to the const directly; see metadata test above.
        assert_eq!(parsed["resource"], SERVER_ORIGIN_DEFAULT);
        assert_eq!(parsed["authorization_servers"][0], SERVER_ORIGIN_DEFAULT);
        assert_eq!(parsed["scopes_supported"][0], "mcp");
        assert_eq!(parsed["bearer_methods_supported"][0], "header");
    }

    // ── New tests for today's changes (cursor-vscode-e2e-tests feature) ────

    #[tokio::test]
    async fn test_oauth_protected_resource_metadata_mcp_path_specific() {
        // RFC 9728 §3.1: path-specific protected-resource metadata. Cursor's
        // MCP OAuth provider (3.2+) requests this URL FIRST and silently
        // aborts if it 404s — falling back to the root variant whose
        // `resource` claim doesn't match the URL the client connects to. This
        // test guards against regression.
        use axum::routing::get;
        let app: Router = Router::new().route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth_protected_resource_metadata_mcp),
        );
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-protected-resource/mcp")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&body_bytes).unwrap();
        // TR-R1-M3 — anchor to the const directly; see metadata test above.
        assert_eq!(
            parsed["resource"],
            format!("{SERVER_ORIGIN_DEFAULT}/mcp"),
            "resource MUST equal the URL the MCP client connects to (path-specific)"
        );
        assert_eq!(parsed["authorization_servers"][0], SERVER_ORIGIN_DEFAULT);
        assert_eq!(parsed["scopes_supported"][0], "mcp");
        assert_eq!(parsed["bearer_methods_supported"][0], "header");
    }

    #[tokio::test]
    async fn test_401_includes_www_authenticate_header() {
        // MCP authorization spec + RFC 6750 §3 require 401 responses from a
        // Bearer-protected resource to advertise the realm + protected-
        // resource metadata URL via WWW-Authenticate. Cursor / Claude.ai
        // recent MCP OAuth providers fail silently when this header is
        // missing. Regression guard.
        let st = fresh_state();
        let app = build_authn_router(st);
        let body = serde_json::json!({"jsonrpc": "2.0", "method": "tools/call", "id": 1});
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .expect("401 MUST include WWW-Authenticate header (RFC 6750 §3)")
            .to_str()
            .expect("WWW-Authenticate must be a valid header value");
        assert!(
            www_auth.starts_with("Bearer "),
            "WWW-Authenticate scheme MUST be Bearer, got: {www_auth}"
        );
        assert!(
            www_auth.contains("resource_metadata="),
            "WWW-Authenticate MUST include resource_metadata param so MCP clients can discover the metadata URL: {www_auth}"
        );
        assert!(
            www_auth.contains("error=\"invalid_token\""),
            "WWW-Authenticate SHOULD include error=invalid_token per RFC 6750: {www_auth}"
        );
    }

    #[tokio::test]
    async fn test_middleware_extracts_claims_on_allowlisted_request_with_valid_jwt() {
        // Allowlisted discovery methods (`initialize` / `tools/list`) may be
        // invoked mid-session with a Bearer token. When that happens the
        // middleware must still attach Claims so downstream handlers can
        // branch on caller identity.
        use axum::{routing::post, Extension};
        async fn echo_claims(claims: Option<Extension<Claims>>) -> String {
            match claims {
                Some(Extension(c)) => format!("authed:{}", c.sub),
                None => "unauth".to_string(),
            }
        }

        let st = fresh_state();
        let token = issue_jwt(&st, "test-claims-sub").unwrap();
        let app: Router = Router::new()
            .route("/mcp", post(echo_claims))
            .layer(axum_middleware::from_fn_with_state(
                st.clone(),
                bearer_auth_middleware,
            ))
            .with_state(st);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 1
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert_eq!(
            body_str, "authed:test-claims-sub",
            "Allowlisted handler MUST see Claims when valid JWT is present"
        );
    }

    #[tokio::test]
    async fn test_middleware_allowlisted_request_without_jwt_passes_no_claims() {
        // Mirror of the above: allowlisted method WITHOUT a Bearer header
        // passes through, handler sees Claims=None. The whole point of the
        // allowlist is to NOT 401 when the caller has no token yet.
        use axum::{routing::post, Extension};
        async fn echo_claims(claims: Option<Extension<Claims>>) -> String {
            match claims {
                Some(Extension(_)) => "authed".to_string(),
                None => "unauth".to_string(),
            }
        }

        let st = fresh_state();
        let app: Router = Router::new()
            .route("/mcp", post(echo_claims))
            .layer(axum_middleware::from_fn_with_state(
                st.clone(),
                bearer_auth_middleware,
            ))
            .with_state(st);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 1
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(std::str::from_utf8(&body_bytes).unwrap(), "unauth");
    }

    #[tokio::test]
    async fn test_middleware_tool_call_requires_jwt() {
        // Every `tools/call` requires a valid Bearer JWT — the discovery
        // surfaces (`initialize`, `tools/list`, `ping`, plus the four
        // agent-native-distribution methods `prompts/list`, `prompts/get`,
        // `resources/list`, `resources/read`) are the JSON-RPC methods
        // allowlisted on /mcp. See `ALLOWLIST_METHODS` for the canonical
        // list.
        let st = fresh_state();
        let app = build_authn_router(st);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_whoami", "arguments": {}},
            "id": 1
        });
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "tools/call MUST 401 without JWT"
        );
    }

    #[tokio::test]
    async fn test_middleware_well_known_bypasses_auth() {
        // `.well-known/*` must bypass bearer-auth — Anthropic's connector
        // probes it WITHOUT a JWT.
        use axum::routing::get;
        let st = fresh_state();
        let app: Router = Router::new()
            .route(
                "/.well-known/oauth-authorization-server",
                get(oauth_authorization_server_metadata),
            )
            .route(
                "/.well-known/oauth-protected-resource",
                get(oauth_protected_resource_metadata),
            )
            .layer(axum_middleware::from_fn_with_state(
                st.clone(),
                bearer_auth_middleware,
            ))
            .with_state(st);

        for uri in [
            "/.well-known/oauth-authorization-server",
            "/.well-known/oauth-protected-resource",
        ] {
            let req = Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "uri={uri}");
        }
    }

    // ── allowed_redirect tests (Decision 5: redirect_uri allowlist) ──────────

    #[test]
    fn test_oauth_allowed_redirect_exact_and_prefix() {
        // Exact-match webapp consent page.
        assert!(allowed_redirect(
            "https://mnemonik.xyz/oauth/consent",
            "anything"
        ));
        assert!(allowed_redirect(
            "https://www.mnemonik.xyz/oauth/consent",
            "anything"
        ));
        assert!(allowed_redirect(
            "https://vscode.dev/redirect",
            "cached-dcr-client"
        ));
        // Cursor / VS Code / Claude.ai deeplink prefixes.
        assert!(allowed_redirect(
            "cursor://anysphere.cursor-deeplink/abc",
            "cursor"
        ));
        assert!(allowed_redirect("vscode:mcp/handle", "vscode"));
        assert!(allowed_redirect("https://claude.ai/api/return", "claude"));
        // Negative: arbitrary URLs must be rejected for every client.
        assert!(!allowed_redirect("https://evil.com", "mnemonic-cli"));
        assert!(!allowed_redirect("https://evil.com", "cursor"));
        assert!(!allowed_redirect(
            "https://vscode.dev/redirect-anything",
            "vscode"
        ));
        // Negative: non-host-equal lookalike (cursor:// without the
        // exact-prefix tail is rejected).
        assert!(!allowed_redirect("cursor://other-vendor/", "cursor"));
        // Negative: 0.0.0.0 is NOT loopback under the regex.
        assert!(!allowed_redirect(
            "http://0.0.0.0:1234/callback",
            "mnemonic-cli"
        ));
        // Negative: lookalike host (127.0.0.1.evil.com) rejected.
        assert!(!allowed_redirect(
            "http://127.0.0.1.evil.com:1234/callback",
            "mnemonic-cli"
        ));
        assert!(!allowed_redirect(
            "http://localhost.evil.com:1234/callback",
            "mnemonic-cli"
        ));
        // Negative: non-numeric port rejected.
        assert!(!allowed_redirect(
            "http://127.0.0.1:abc/callback",
            "mnemonic-cli"
        ));
        // Negative: missing port rejected (RFC 8252 forbids omitting it).
        assert!(!allowed_redirect(
            "http://127.0.0.1/callback",
            "mnemonic-cli"
        ));
        // Negative: fragment in path rejected (cannot be slipped past
        // a registered redirect).
        assert!(!allowed_redirect(
            "http://127.0.0.1:1234/cb#frag",
            "mnemonic-cli"
        ));
    }

    #[test]
    fn test_oauth_allowed_redirect_loopback_any_client_any_path() {
        // RFC 8252 §7.3: native clients are permitted to use loopback
        // with ANY port and ANY path. The previous `mnemonic-cli`-only
        // and `/callback`-only constraints broke VS Code's MCP OAuth
        // (UUID client_id from DCR + path "/") without providing real
        // security — PKCE+state already prevent code interception, and
        // the loopback is on the user's own machine.
        //
        // Regression guard: 2026-05-04. VS Code 1.118.1 with
        // client_id=<DCR-UUID> and redirect_uri=http://127.0.0.1:<port>/
        // was rejected with "redirect_uri is not on the server allowlist"
        // until this constraint was relaxed.
        for client in [
            "mnemonic-cli",
            "cursor",
            "vscode",
            "0c0009bc-3404-4898-b935-8862ee3a03b2", // VS Code DCR UUID
            "claude",
            "",
        ] {
            // Path "/" — VS Code's MCP OAuth callback.
            assert!(
                allowed_redirect("http://127.0.0.1:33418/", client),
                "loopback / must accept client_id={client:?}"
            );
            assert!(
                allowed_redirect("http://localhost:33418/", client),
                "localhost loopback / must accept client_id={client:?}"
            );
            assert!(
                allowed_redirect("http://[::1]:33418/", client),
                "loopback v6 / must accept client_id={client:?}"
            );
            // Path "/callback" — mnemonic-cli's path.
            assert!(allowed_redirect("http://127.0.0.1:1234/callback", client));
            assert!(allowed_redirect("http://localhost:1234/callback", client));
            // Path with extra segments — also allowed per RFC 8252.
            assert!(allowed_redirect(
                "http://127.0.0.1:1234/callback/extra",
                client
            ));
            assert!(allowed_redirect(
                "http://localhost:1234/callback/extra",
                client
            ));
            // No path at all (just port).
            assert!(allowed_redirect("http://127.0.0.1:1234", client));
            assert!(allowed_redirect("http://localhost:1234", client));
        }
    }

    #[test]
    fn test_oauth_registered_redirects_allow_vscode_callbacks() {
        let st = fresh_state();
        let client_id = "vscode-dcr-client".to_string();
        st.register_client(
            client_id.clone(),
            vec![
                "https://vscode.dev/redirect".to_string(),
                "http://127.0.0.1:33418/".to_string(),
                "http://localhost:33418/".to_string(),
                "http://[::1]:33418/".to_string(),
            ],
        );

        assert!(st.allows_redirect("https://vscode.dev/redirect", &client_id));
        assert!(st.allows_redirect("http://127.0.0.1:59656/", &client_id));
        assert!(st.allows_redirect("http://localhost:59656/", &client_id));
        assert!(st.allows_redirect("http://[::1]:59656/", &client_id));
        assert!(!st.allows_redirect("https://evil.com/redirect", &client_id));
        assert!(!st.allows_redirect("http://127.0.0.1.evil.com:59656/", &client_id));
        assert!(!st.allows_redirect("http://localhost.evil.com:59656/", &client_id));
    }

    #[test]
    fn test_oauth_registered_redirect_loopback_any_path() {
        // Policy change 2026-05-04: loopback URIs are accepted for ANY
        // client_id with ANY path per RFC 8252 §7.3 (see
        // `test_oauth_allowed_redirect_loopback_any_client_any_path`).
        // The previous test asserted that a DCR-registered client with
        // `/callback` was port-flexible but path-pinned; that constraint
        // was defense-in-depth (PKCE+state already prevent code
        // interception) and broke real OAuth clients (VS Code with a
        // wiped-from-memory DCR client_id).
        //
        // This test now asserts the new policy: a registered client gets
        // both port AND path flexibility on loopback. Non-loopback
        // hosts still go through DCR's exact-or-loopback matcher.
        let st = fresh_state();
        let client_id = "registered-client".to_string();
        st.register_client(
            client_id.clone(),
            vec!["http://127.0.0.1:33418/callback".to_string()],
        );

        // Loopback — any port, any path, registered or not.
        assert!(st.allows_redirect("http://127.0.0.1:50000/callback", &client_id));
        assert!(st.allows_redirect("http://127.0.0.1:50000/other", &client_id));
        assert!(st.allows_redirect("http://127.0.0.1:50000/", &client_id));
        assert!(st.allows_redirect("http://[::1]:50000/anything", &client_id));
        // Non-loopback — still strictly checked.
        assert!(!st.allows_redirect("https://evil.com/callback", &client_id));
        assert!(!st.allows_redirect("http://127.0.0.1.evil.com:50000/callback", &client_id));
    }

    #[tokio::test]
    async fn test_oauth_authorize_init_rejects_non_allowlisted_redirect() {
        // Bootstrap with redirect_uri=https://evil.com → 400.
        let st = fresh_state();
        let app = build_init_router(st.clone());
        let req = Request::builder()
            .method("GET")
            .uri(
                "/oauth/authorize\
                 ?client_id=cursor\
                 &redirect_uri=https%3A%2F%2Fevil.com\
                 &code_challenge=abc\
                 &code_challenge_method=S256\
                 &state=csrf-evil",
            )
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // Pending map remains empty — rejection short-circuits BEFORE insert.
        let present = {
            let g = st.pending.lock().unwrap();
            g.peek("csrf-evil").is_some()
        };
        assert!(!present, "rejected redirect_uri must not store pending");
    }

    #[tokio::test]
    async fn test_oauth_state_binding_validates_redirect_uri_too() {
        // Bind redirect_uri at /oauth/authorize, then submit /oauth/token with
        // a different value → 400. RFC 6749 §4.1.3.
        //
        // The current `authorize_handler` consumes a raw 64-byte Ed25519
        // signature over the canonical-CBOR challenge bytes (the
        // `signature` field with `cose_signed` legacy alias). We sign the
        // exact bytes we insert as `pending.challenge_bytes` so the server-
        // side verifier (`mnemonic_core::identity::verify_signature`) accepts.
        let st = fresh_state();
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let verifier = "v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let challenge = pkce_challenge(verifier);
        let bound_uri = "https://claude.ai/api/cb-bound";

        // Build the challenge canonical-CBOR bytes deterministically and
        // sign them with the user's keypair. Same fields the bootstrap
        // handler would store.
        let challenge_obj = serde_json::json!({
            "server_origin": server_origin(),
            "state": "rb-state",
            "client_id": "test-client",
            "redirect_uri": bound_uri,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "nonce": "n",
            "exp": now_secs() + 30,
        });
        let cbor_bytes = to_canonical_cbor(&challenge_obj, &CHALLENGE_SCHEMA).unwrap();
        let raw_sig = mnemonic_core::identity::sign_bytes(&kp, &cbor_bytes);
        assert_eq!(raw_sig.len(), 64);
        let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &raw_sig);
        let hash = blake3_hex(&cbor_bytes);

        st.insert_pending(
            "rb-state".to_string(),
            hash,
            cbor_bytes,
            pubkey.clone(),
            bound_uri.to_string(),
            challenge.clone(),
            "rb-state".to_string(),
            now_secs() + 30,
        );

        let app = build_authorize_router(st.clone());
        let (s1, body) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({
                "state": "rb-state",
                "signature": sig_b64,
                "signer_pubkey": pubkey,
            }),
        )
        .await;
        assert_eq!(s1, StatusCode::OK, "authorize must succeed (body={body})");
        let code = body["code"].as_str().unwrap().to_string();

        // Token exchange with a DIFFERENT redirect_uri → 400.
        let app2 = build_authorize_router(st.clone());
        let (s2, _) = post_json(
            app2,
            "/oauth/token",
            serde_json::json!({
                "code": code.clone(),
                "code_verifier": verifier,
                "redirect_uri": "https://claude.ai/api/cb-other",
            }),
        )
        .await;
        assert_eq!(
            s2,
            StatusCode::BAD_REQUEST,
            "mismatched redirect_uri must be rejected"
        );

        // After the 400 above the code was already popped from the LRU
        // (`codes.lock().pop` runs BEFORE the redirect_uri check). A second
        // attempt — even with the correct value — must therefore fail with
        // 401, proving the code is single-use even after a 400 mismatch
        // (security property: a failed exchange must not leak a usable code).
        let app3 = build_authorize_router(st);
        let (s3, body3) = post_json(
            app3,
            "/oauth/token",
            serde_json::json!({
                "code": code,
                "code_verifier": verifier,
                "redirect_uri": bound_uri,
            }),
        )
        .await;
        assert_eq!(
            s3,
            StatusCode::UNAUTHORIZED,
            "code must be single-use even after a 400 mismatch, got body={body3}"
        );
    }

    // ── Decision 12 — MCP_JWT_TTL_SECS env-plumbing ──────────────────────────
    // Both tests exercise the pure helper `compute_jwt_ttl_from_env_str` so
    // they never seed the process-global `JWT_TTL` OnceLock — running them
    // in parallel with other tests (or one another) cannot pollute the
    // shared static.

    #[tracing_test::traced_test]
    #[test]
    fn jwt_ttl_seeded_from_env_and_clamped() {
        // Below-min clamp: `10` → `JWT_TTL_MIN_SECS` (60).
        assert_eq!(compute_jwt_ttl_from_env_str(Some("10")), JWT_TTL_MIN_SECS);
        assert!(logs_contain(
            "MCP_JWT_TTL_SECS=10 below minimum 60s; clamped to 60"
        ));
        // Above-max clamp: `9999999` → `JWT_TTL_MAX_SECS` (604_800).
        assert_eq!(
            compute_jwt_ttl_from_env_str(Some("9999999")),
            JWT_TTL_MAX_SECS
        );
        assert!(logs_contain(
            "MCP_JWT_TTL_SECS=9999999 above maximum 604800s; clamped to 604800"
        ));
        // In-range passthrough: `3600` → `3600` (the legacy const value;
        // guarantees the env-unset path stays byte-identical to the prior
        // const so existing OAuth flows do not regress).
        assert_eq!(compute_jwt_ttl_from_env_str(Some("3600")), 3600);
    }

    #[tracing_test::traced_test]
    #[test]
    fn jwt_ttl_parse_failure_logs_warn_and_uses_default() {
        // Non-numeric: must NOT silently default — operator typo on the
        // Task 10 R1 gate would otherwise turn a 60s observation window
        // into a 1h one. WARN log is the loud-failure contract.
        assert_eq!(
            compute_jwt_ttl_from_env_str(Some("notanumber")),
            JWT_TTL_DEFAULT_SECS
        );
        assert!(logs_contain(
            "MCP_JWT_TTL_SECS=\"notanumber\" failed to parse as u64"
        ));
        // Empty string and whitespace-only get the same loud treatment.
        assert_eq!(compute_jwt_ttl_from_env_str(Some("")), JWT_TTL_DEFAULT_SECS);
        assert!(logs_contain("MCP_JWT_TTL_SECS is empty / whitespace-only"));
        assert_eq!(
            compute_jwt_ttl_from_env_str(Some("   ")),
            JWT_TTL_DEFAULT_SECS
        );
        // Whitespace-only must emit the same loud WARN as the empty case —
        // both trim to "" and hit the `is_empty()` branch (TR2-R1-1).
        assert!(logs_contain("MCP_JWT_TTL_SECS is empty / whitespace-only"));
        // Sanity-check the silent path: env wholly unset (None) returns
        // the default without emitting a WARN — only the explicit
        // misconfiguration cases above are loud.
        assert_eq!(compute_jwt_ttl_from_env_str(None), JWT_TTL_DEFAULT_SECS);
    }

    #[tracing_test::traced_test]
    #[test]
    fn jwt_ttl_parse_failure_warn_bounds_raw_value() {
        // SA2-R1-M1: the parse-failure WARN must bound the echoed raw value
        // so a 1 MB attacker-supplied string doesn't bloat log aggregation.
        // 17 chars should truncate to 16 chars + ellipsis.
        let raw = "abcdefghijklmnopq"; // 17 chars, none of which parse
        assert_eq!(
            compute_jwt_ttl_from_env_str(Some(raw)),
            JWT_TTL_DEFAULT_SECS
        );
        assert!(logs_contain("abcdefghijklmnop")); // first 16 chars
        assert!(logs_contain("…")); // truncation marker
        assert!(
            !logs_contain("abcdefghijklmnopq"),
            "the 17th char must NOT appear in the WARN log"
        );
    }

    // ── MCP_PUBLIC_BASE_URL env-plumbing ─────────────────────────────────────
    // Mirrors the JWT-TTL pattern above: exercise the pure helper
    // `compute_server_origin_from_env_str` so tests never seed the
    // process-global `SERVER_ORIGIN` OnceLock.

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_defaults_when_env_unset() {
        // Wholly-unset env → silent default; no WARN.
        assert_eq!(
            compute_server_origin_from_env_str(None),
            SERVER_ORIGIN_DEFAULT
        );
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_uses_env_when_set() {
        // https override.
        assert_eq!(
            compute_server_origin_from_env_str(Some(
                "https://car-damages-controlling-blocked.trycloudflare.com"
            )),
            "https://car-damages-controlling-blocked.trycloudflare.com"
        );
        // TR-R1-L1: negative WARN assertion on the success path — catches a
        // regression where the validation logic spuriously rejects a
        // well-formed URL and silently falls back.
        assert!(!logs_contain("falling back to default"));
        // http override (loopback-style dev).
        assert_eq!(
            compute_server_origin_from_env_str(Some("http://127.0.0.1:3000")),
            "http://127.0.0.1:3000"
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            compute_server_origin_from_env_str(Some("  https://mcp.dev.mnemonik.xyz  ")),
            "https://mcp.dev.mnemonik.xyz"
        );
        // Re-assert the negative — `logs_contain` is over the whole scope.
        assert!(!logs_contain("falling back to default"));
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_valid_with_port() {
        // TR-R1-missing-1: port numbers in the authority are accepted
        // verbatim. `rest = "localhost:8443"` contains no '/', '?', '#', or
        // '@', so it passes every rejection rule.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://localhost:8443")),
            "https://localhost:8443"
        );
        assert!(!logs_contain("falling back to default"));
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_rejects_uppercase_scheme() {
        // TR-R1-missing-2: byte-exact `strip_prefix` rejects mixed-case
        // schemes. RFC 3986 says schemes are case-insensitive but
        // recommends normalising to lowercase; rejecting upper-case is
        // conservative and documents intent (operators must use
        // lowercase).
        assert_eq!(
            compute_server_origin_from_env_str(Some("HTTPS://mcp.example.com")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("missing http:// or https:// scheme"));
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_rejects_userinfo_in_authority() {
        // SA-R1-M1: a value with embedded credentials (`user:pass@host`)
        // would otherwise INFO-log the credential via
        // `seed_server_origin_from_env` and propagate into OAuth metadata
        // documents. Reject at validation time.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://user:pass@example.com")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("contains userinfo"));
        // Userinfo without password is also rejected.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://admin@example.com")),
            SERVER_ORIGIN_DEFAULT
        );
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_rejects_control_chars() {
        // SA-R1-M2: CRLF / null / tab / other ASCII control chars are
        // rejected at validation time as defense-in-depth against header
        // injection if `server_origin()` is ever interpolated into a raw
        // HTTP header string outside `HeaderValue::from_str`'s guard.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://evil.com\r\nX-Injected: pwned")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("contains ASCII control character"));
        // Newline alone.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://evil.com\nfoo")),
            SERVER_ORIGIN_DEFAULT
        );
        // Tab embedded mid-host.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://evil\t.com")),
            SERVER_ORIGIN_DEFAULT
        );
        // NUL byte.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://evil.com\0bar")),
            SERVER_ORIGIN_DEFAULT
        );
    }

    #[tracing_test::traced_test]
    #[test]
    fn server_origin_falls_back_on_malformed_env() {
        // Empty / whitespace-only — WARN + default. Loud failure closes the
        // silent-deploy-typo vector on the Task 10 R1 gate.
        assert_eq!(
            compute_server_origin_from_env_str(Some("")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain(
            "MCP_PUBLIC_BASE_URL is empty / whitespace-only"
        ));
        // TR-R1-M1: whitespace-only must emit the same loud WARN as the
        // empty case. Both `trim()` to "" and hit the `is_empty()` branch.
        // Removing the WARN for this sub-case would leave the test green
        // without this assertion.
        assert_eq!(
            compute_server_origin_from_env_str(Some("   ")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain(
            "MCP_PUBLIC_BASE_URL is empty / whitespace-only"
        ));
        // Missing scheme.
        assert_eq!(
            compute_server_origin_from_env_str(Some("mcp.mnemonik.xyz")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain(
            "missing http:// or https:// scheme; falling back to default"
        ));
        // Trailing slash — would double-slash on metadata URL concat.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://example.com/")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("has trailing slash"));
        // Path component beyond host — would garble the metadata URL.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://example.com/sub")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("has path component beyond host"));
        // Query string.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://example.com?x=1")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("contains query or fragment"));
        // Fragment.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://example.com#frag")),
            SERVER_ORIGIN_DEFAULT
        );
        // Scheme without host.
        assert_eq!(
            compute_server_origin_from_env_str(Some("https://")),
            SERVER_ORIGIN_DEFAULT
        );
        assert!(logs_contain("has scheme but no host"));
    }

    #[test]
    fn server_origin_accessor_anti_panic_smoke() {
        // TR-R1-M2: anti-panic smoke ONLY — NOT a default-value assertion.
        // The OnceLock is process-global and a sibling test may have
        // legitimately seeded it before this test runs, so we cannot
        // assert exact equality to the default. Asserting only the
        // scheme-prefixed-non-empty invariant catches: (a) a regression
        // where `server_origin()` panics on pre-seed access (would fail
        // immediately at call time), (b) a regression where it returns
        // an empty string or non-scheme-prefixed garbage. The actual
        // default-value contract is covered by
        // `server_origin_defaults_when_env_unset` (which exercises the
        // pure helper directly). Renamed from
        // `server_origin_accessor_default_pre_seed` for honesty.
        let origin = server_origin();
        assert!(
            origin.starts_with("https://") || origin.starts_with("http://"),
            "server_origin() must always return a scheme-prefixed URL, got {origin}"
        );
        assert!(!origin.is_empty());
    }

    #[test]
    fn bound_log_value_truncates_to_max_chars() {
        // No truncation when len ≤ max.
        assert_eq!(bound_log_value("abc", 5), "abc");
        assert_eq!(bound_log_value("abcde", 5), "abcde");
        // Truncates with ellipsis when len > max.
        assert_eq!(bound_log_value("abcdef", 5), "abcde…");
        // Empty stays empty.
        assert_eq!(bound_log_value("", 5), "");
        // Char-boundary safe — multi-byte UTF-8 must not panic.
        // "ü" is 2 bytes; "🦀" is 4. With max_chars=3, take "üü🦀" wholly.
        assert_eq!(bound_log_value("üü🦀abc", 3), "üü🦀…");
    }

    // ── Task 3 TDD anchors (refresh-token-rotation) ─────────────────────────

    /// Build a router for /oauth/token only — used by the Task 3 dispatch
    /// and cache-header anchors. Shares state across calls so a successful
    /// `authorization_code` exchange can be followed by a `refresh_token`
    /// rotation in the same test.
    fn build_token_router(state: Arc<OAuthState>) -> Router {
        Router::new()
            .route("/oauth/token", post(token_handler))
            .with_state(state)
    }

    /// POST helper that returns the full `Response` so the caller can
    /// inspect headers (Cache-Control / Pragma) AND the JSON body.
    async fn post_token_form_full(app: Router, form: &str) -> Response {
        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form.to_string()))
            .unwrap();
        app.oneshot(req).await.unwrap()
    }

    /// Decode the JSON body of a token response (consumes the response).
    async fn read_json_body(resp: Response) -> (StatusCode, Value) {
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, parsed)
    }

    /// Assert RFC 6749 §5.1 / Decision 15 cache-suppression headers.
    fn assert_no_store_cache_headers(resp: &Response) {
        let cc = resp.headers().get(axum::http::header::CACHE_CONTROL);
        assert_eq!(
            cc.and_then(|h| h.to_str().ok()),
            Some("no-store"),
            "/oauth/token response must carry Cache-Control: no-store"
        );
        let pragma = resp.headers().get(axum::http::header::PRAGMA);
        assert_eq!(
            pragma.and_then(|h| h.to_str().ok()),
            Some("no-cache"),
            "/oauth/token response must carry Pragma: no-cache"
        );
    }

    /// TDD anchor: POST grant_type=refresh_token with no refresh_token
    /// field → 400 invalid_request (Decision 11 / AC13).
    #[tokio::test]
    async fn missing_refresh_token_field_returns_invalid_request() {
        let st = fresh_state();
        let app = build_token_router(st);
        let resp = post_token_form_full(app, "grant_type=refresh_token").await;
        let status = resp.status();
        assert_no_store_cache_headers(&resp);
        let (_, body) = read_json_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        let err = body["error"].as_str().unwrap_or("");
        assert!(
            err.starts_with("invalid_request"),
            "expected invalid_request, got {err}"
        );
    }

    /// TDD anchor: unknown grant_type → 400 unsupported_grant_type
    /// (RFC 6749 §5.2 / Decision 11 — closes the silent fall-through to
    /// authorization_code).
    #[tokio::test]
    async fn unknown_grant_type_returns_unsupported_grant_type() {
        let st = fresh_state();
        let app = build_token_router(st);
        let resp = post_token_form_full(app, "grant_type=client_credentials").await;
        let status = resp.status();
        assert_no_store_cache_headers(&resp);
        let (_, body) = read_json_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        let err = body["error"].as_str().unwrap_or("");
        assert!(
            err.starts_with("unsupported_grant_type"),
            "expected unsupported_grant_type, got {err}"
        );
    }

    /// TDD anchor: refresh_token > 4096 bytes → 400 invalid_request,
    /// rejected BEFORE any hashing or DB lookup (Decision 16). Asserts:
    /// 1. the response is invalid_request (not invalid_grant — the gate
    ///    fires before `refresh::rotate` is reached);
    /// 2. no row exists in `refresh_tokens` referencing the hash of the
    ///    oversized plaintext (which proves the DB lookup never ran).
    #[tokio::test]
    async fn refresh_token_too_long_returns_invalid_request() {
        let st = fresh_state();
        let app = build_token_router(st.clone());
        // 4097 bytes — one over the cap.
        let oversized = "a".repeat(REFRESH_TOKEN_MAX_LEN + 1);
        let form = format!(
            "grant_type=refresh_token&refresh_token={}",
            urlencoding_encode(&oversized)
        );
        let resp = post_token_form_full(app, &form).await;
        let status = resp.status();
        assert_no_store_cache_headers(&resp);
        let (_, body) = read_json_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        let err = body["error"].as_str().unwrap_or("");
        assert!(
            err.starts_with("invalid_request"),
            "must be invalid_request (length cap), not invalid_grant; got {err}"
        );
        // Assert no DB work occurred — the row count is still 0 and no
        // hash of the oversized plaintext could possibly be present.
        let conn = st.refresh_store.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM refresh_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "length cap MUST fire before any DB insert");
    }

    /// TDD anchor: every /oauth/token response (success AND error) carries
    /// `Cache-Control: no-store` + `Pragma: no-cache` (Decision 15).
    /// Iterates the three error-path branches PLUS a success path.
    #[tokio::test]
    async fn token_response_emits_no_store_cache_headers() {
        let st = fresh_state();

        // Error path 1: invalid_request (missing refresh_token).
        let resp1 =
            post_token_form_full(build_token_router(st.clone()), "grant_type=refresh_token").await;
        assert_no_store_cache_headers(&resp1);

        // Error path 2: unsupported_grant_type.
        let resp2 = post_token_form_full(build_token_router(st.clone()), "grant_type=foo").await;
        assert_no_store_cache_headers(&resp2);

        // Error path 3: invalid_grant (unknown refresh token).
        let resp3 = post_token_form_full(
            build_token_router(st.clone()),
            "grant_type=refresh_token&refresh_token=never-issued-plaintext",
        )
        .await;
        assert_no_store_cache_headers(&resp3);

        // Error path 4: authorization_code with missing code.
        let resp4 = post_token_form_full(
            build_token_router(st.clone()),
            "grant_type=authorization_code&code_verifier=v",
        )
        .await;
        assert_no_store_cache_headers(&resp4);

        // Success path: drive a complete authorization_code exchange so
        // we can assert the cache headers ride the 200 too. The webapp /
        // tests path: insert an IssuedCode directly via mint_issued_code,
        // then redeem at /oauth/token.
        let verifier = "a-test-verifier-min-43-chars-long-aaaaaaaaaa";
        let challenge = pkce_challenge(verifier);
        let code = st.mint_issued_code(
            "test-sub".to_string(),
            challenge,
            "https://app/callback".to_string(),
            None,
        );
        let form = format!(
            "grant_type=authorization_code&code={code}&code_verifier={}",
            urlencoding_encode(verifier)
        );
        let resp5 = post_token_form_full(build_token_router(st.clone()), &form).await;
        assert_eq!(resp5.status(), StatusCode::OK);
        assert_no_store_cache_headers(&resp5);
    }

    /// TDD anchor: `validate_refresh_salt(None)` returns Err (Decision 2).
    #[test]
    fn salt_missing_aborts_boot() {
        let result = validate_refresh_salt(None);
        assert!(
            result.is_err(),
            "validate_refresh_salt(None) MUST error — required env var"
        );
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("MCP_REFRESH_SALT"),
            "error message must name the env var; got {msg}"
        );
    }

    /// TDD anchor: salt that base64-decodes to < 32 bytes returns Err
    /// (Decision 2 — closes the 32-ASCII-chars / ~5-bytes-of-entropy
    /// footgun). Also covers a positive case to prove the happy path
    /// returns the decoded bytes.
    #[test]
    fn salt_under_32_bytes_aborts_boot() {
        // "YQ==" — standard-base64 of `b"a"` — decodes to 1 byte.
        let result = validate_refresh_salt(Some("YQ=="));
        assert!(result.is_err(), "1-byte salt MUST be rejected (< 32 bytes)");
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("32") || msg.contains(">= 32"),
            "error must explain the 32-byte minimum; got {msg}"
        );

        // Positive case: 32 random bytes base64-encoded (matches the
        // `openssl rand -base64 32` output shape).
        let bytes32 = [0xCDu8; 32];
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes32);
        let result_ok = validate_refresh_salt(Some(&encoded));
        let decoded = result_ok.expect("32-byte salt must decode");
        assert!(decoded.len() >= 32, "decoded len must be >= 32");
    }

    /// TDD anchor (round-2 m6): the `reuse_cache_cap` argument to
    /// `OAuthState::new` IS wired through to the LRU constructor — NOT
    /// silently defaulted to `DEFAULT_REUSE_CACHE_CAP`. Construct an
    /// OAuthState with cap=2, drive 3 rotations, assert the LRU evicts
    /// the oldest entry (the first plaintext is no longer cached). Using
    /// the in-memory `Connection` via tempfile so we can drive `new`
    /// directly with a custom cap.
    #[tokio::test]
    async fn reuse_cache_cap_respects_oauth_state_field() {
        // Cap = 2.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let salt = vec![0xABu8; 32];
        let st = OAuthState::new(
            TEST_SECRET,
            &path,
            salt,
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(60),
            2,
        )
        .expect("OAuthState::new with cap=2");
        // Publish 3 distinct entries; LRU cap=2 must evict the oldest.
        st.reuse_cache
            .put("k1", "j1".into(), "p1".into(), "fam".into());
        st.reuse_cache
            .put("k2", "j2".into(), "p2".into(), "fam".into());
        st.reuse_cache
            .put("k3", "j3".into(), "p3".into(), "fam".into());
        assert_eq!(
            st.reuse_cache.len(),
            2,
            "cap=2 must hold at 2 entries after 3 puts"
        );
        assert!(
            st.reuse_cache.get("k1").is_none(),
            "oldest entry k1 must be evicted under cap=2"
        );
        assert!(st.reuse_cache.get("k2").is_some());
        assert!(st.reuse_cache.get("k3").is_some());
    }

    /// POST `/oauth/token` form helper that injects `X-Forwarded-For` so
    /// the handler's D14 forensic-field extraction (`remote_addr`) has
    /// non-`-` input to log. Used by the round-2 logging-policy anchor
    /// which asserts both ABSENCE of secrets AND PRESENCE of forensic
    /// fields per tech-spec line 628.
    async fn post_token_form_with_xff(app: Router, form: &str, xff: &str) -> Response {
        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("x-forwarded-for", xff)
            .body(Body::from(form.to_string()))
            .unwrap();
        app.oneshot(req).await.unwrap()
    }

    /// TDD anchor (D14): per tech-spec line 628 the test asserts BOTH
    /// (a) NO plaintext, at-rest hash, SALT BYTES, or full ACCESS_TOKEN
    /// appears in any tracing log emitted from `refresh::rotate` or
    /// `token_handler` AND (b) the forensic fields the policy mandates
    /// (`family_id`, `sub`, `remote_addr`, `request_id`) ARE present
    /// where applicable. Iterates each of the 6 rotate branches
    /// individually (A, B, B', C, D, E) — Branch B' is an INSIDE-window
    /// cache-MISS, distinct from Branch C's OUTSIDE-window replay, so
    /// the test forces a cache flush after Branch A and re-submits the
    /// OLD plaintext to exercise B' explicitly. Branch D (expired) is
    /// pre-seeded with `expires_at < now`; Branch E (unknown) is a
    /// never-issued plaintext.
    #[tokio::test]
    #[tracing_test::traced_test]
    async fn logging_policy_no_plaintext_no_hash_across_branches() {
        let st = fresh_state();
        let app_factory = || build_token_router(st.clone());

        // Per-branch X-Forwarded-For sentinels so we can prove
        // `remote_addr` IS captured in the resulting log lines.
        let xff_a = "203.0.113.10";
        let xff_b = "203.0.113.11";
        let xff_b_prime = "203.0.113.12";
        let xff_c = "203.0.113.13";
        let xff_d = "203.0.113.14";
        let xff_e = "203.0.113.15";

        // ── Branch E — too-long pre-hash ─────────────────────────────
        let too_long_pt = "BRANCHE_PLAINTEXT_a".repeat(500);
        let _ = post_token_form_with_xff(
            app_factory(),
            &format!(
                "grant_type=refresh_token&refresh_token={}",
                urlencoding_encode(&too_long_pt)
            ),
            xff_e,
        )
        .await;

        // ── Branch D — expired refresh-token (pre-seeded row with
        // `expires_at < now`, revoked=0 — that is refresh.rs's Branch D) ──
        let expired_plaintext = "BRANCHD_EXPIRED_PLAINTEXT";
        let expired_hash = refresh::hash_refresh_token(&st.refresh_salt, expired_plaintext);
        let expired_family = uuid::Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        {
            let conn = st.refresh_store.lock().unwrap();
            conn.execute(
                "INSERT INTO refresh_tokens
                    (token_hash, sub, google_sub, issued_at, expires_at,
                     revoked, family_id)
                 VALUES (?1, 'BRANCHD_SUB', NULL, ?2, ?3, 0, ?4)",
                rusqlite::params![expired_hash, now - 10, now - 1, expired_family],
            )
            .unwrap();
        }
        let _ = post_token_form_with_xff(
            app_factory(),
            &format!("grant_type=refresh_token&refresh_token={expired_plaintext}"),
            xff_d,
        )
        .await;

        // ── Branches A / B / B' / C ──────────────────────────────────
        // Mint a refresh row via authorization_code redeem, then drive
        // the rotation flow.
        let verifier_a = "branchA-verifier-min-43-chars-long-aaaaaaaaaaa";
        let challenge_a = pkce_challenge(verifier_a);
        let code_a = st.mint_issued_code(
            "BRANCHA_SUB".to_string(),
            challenge_a,
            "https://app/callback".to_string(),
            None,
        );
        let form_a = format!(
            "grant_type=authorization_code&code={code_a}&code_verifier={}",
            urlencoding_encode(verifier_a)
        );
        // The authorization_code redeem path doesn't carry remote_addr
        // into the log; we only need its response body to extract the
        // refresh-token plaintext and access-token to assert against.
        let resp_a_init = post_token_form_with_xff(app_factory(), &form_a, xff_a).await;
        let (_, body_a) = read_json_body(resp_a_init).await;
        let rt_a = body_a["refresh_token"]
            .as_str()
            .expect("authorization_code mint should yield refresh_token")
            .to_string();
        let access_token_a = body_a["access_token"]
            .as_str()
            .expect("authorization_code redeem should yield access_token")
            .to_string();
        let form_refresh = format!(
            "grant_type=refresh_token&refresh_token={}",
            urlencoding_encode(&rt_a)
        );

        // Branch A — first rotate of the just-minted plaintext.
        let resp_branch_a = post_token_form_with_xff(app_factory(), &form_refresh, xff_a).await;
        let (_, body_branch_a) = read_json_body(resp_branch_a).await;
        let access_token_branch_a = body_branch_a["access_token"]
            .as_str()
            .expect("Branch A should yield an access_token")
            .to_string();

        // Branch B — second call within reuse-interval against the SAME
        // (now revoked) plaintext, cache HIT.
        let _ = post_token_form_with_xff(app_factory(), &form_refresh, xff_b).await;

        // Branch B' — INSIDE the reuse window but cache MISS. Wipe the
        // reuse_cache directly so the next rotate of the SAME old
        // plaintext sees revoked=1, rotated_at recent → cache.get None →
        // fail-closed. This is the actual B' branch (distinct from C).
        st.reuse_cache.clear();
        let _ = post_token_form_with_xff(app_factory(), &form_refresh, xff_b_prime).await;

        // Branch C — replay OUTSIDE reuse-interval. The with_defaults
        // reuse_interval is 100 ms (sub-second flake-resistant: rusqlite
        // stores integer seconds, so we wait long enough that
        // `now > rotated_at + reuse_interval.as_secs()` reliably).
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let _ = post_token_form_with_xff(app_factory(), &form_refresh, xff_c).await;

        // ─────────────────────────────────────────────────────────────
        // (a) ABSENCE assertions — no plaintext / no hash / no salt /
        // no full access_token leaks into any log line.
        // ─────────────────────────────────────────────────────────────
        assert!(
            !logs_contain("BRANCHE_PLAINTEXT_a"),
            "D14: Branch E must not log the oversized plaintext"
        );
        assert!(
            !logs_contain(expired_plaintext),
            "D14: Branch D must not log the expired plaintext"
        );
        assert!(
            !logs_contain(&rt_a),
            "D14: rotate must never log the legitimate refresh-token plaintext"
        );
        let hash_rt = refresh::hash_refresh_token(&st.refresh_salt, &rt_a);
        assert!(
            !logs_contain(&hash_rt),
            "D14: rotate must never log the at-rest hash"
        );
        let salt_hex = hex::encode(&st.refresh_salt);
        assert!(
            !logs_contain(&salt_hex),
            "D14: rotate must never log the per-deploy salt bytes"
        );
        assert!(
            !logs_contain(&access_token_a),
            "D14: rotate must never log the full authorization_code access_token"
        );
        assert!(
            !logs_contain(&access_token_branch_a),
            "D14: rotate must never log the full Branch-A access_token"
        );

        // ─────────────────────────────────────────────────────────────
        // (b) PRESENCE assertions — forensic fields IS captured per
        // tech-spec line 628.
        // ─────────────────────────────────────────────────────────────
        // Sub is logged on Branch A (success) — refresh.rs:795-803.
        assert!(
            logs_contain("BRANCHA_SUB"),
            "D14: Branch A must log sub for forensic correlation"
        );
        // Family_id is logged on Branch A success — emitted as
        // `family_id=<uuid>`; presence of the key is sufficient since
        // the uuid is randomly generated per mint.
        assert!(
            logs_contain("family_id"),
            "D14: Branch A/B/C/D log lines must carry family_id"
        );
        // Remote_addr is threaded into the rotate ctx from the handler,
        // surfaces in log lines for B'/C/D/E (the branches that log
        // remote_addr per refresh.rs). Each branch uses a distinct
        // X-Forwarded-For so we can assert presence.
        assert!(
            logs_contain(xff_b_prime),
            "D14: Branch B' must log remote_addr (X-Forwarded-For first hop)"
        );
        assert!(logs_contain(xff_c), "D14: Branch C must log remote_addr");
        assert!(logs_contain(xff_d), "D14: Branch D must log remote_addr");
        assert!(logs_contain(xff_e), "D14: Branch E must log remote_addr");
        // request_id is a UUIDv4 minted per /oauth/token request — assert
        // the key is present in the log buffer (the value itself rotates
        // per branch / per request).
        assert!(
            logs_contain("request_id"),
            "D14: log lines must carry request_id for cross-call correlation"
        );
    }
}
