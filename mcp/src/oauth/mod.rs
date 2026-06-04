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

use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// JWT issuer — must match the production server origin.
pub const JWT_ISSUER: &str = "mcp.mnemonik.xyz";
/// JWT audience — fixed, validated on every verify.
pub const JWT_AUDIENCE: &str = "mcp";
/// JWT TTL: 1 hour per Decision 11.
pub const JWT_TTL_SECS: u64 = 3600;
/// Pending-state TTL: 60s per Decision 10. Consumed by the consent-page
/// bootstrap endpoint (`GET /oauth/authorize`) when inserting a fresh challenge.
pub const STATE_TTL_SECS: u64 = 60;
/// Issued-code TTL: 60s — code must be redeemed at /oauth/token within this.
pub const CODE_TTL_SECS: u64 = 60;
/// LRU bound on both pending-state and issued-code maps.
pub const OAUTH_STATE_CAPACITY: usize = 10_000;
/// Server origin used in the canonical-CBOR challenge (Decision 10).
/// Public so the consent-page bootstrap and tests can reuse the same value.
pub const SERVER_ORIGIN: &str = "https://mcp.mnemonik.xyz";
/// Frontend webapp origin — the consent page lives here. The bootstrap
/// endpoint `GET /oauth/authorize` redirects the user-agent (browser) to
/// `WEBAPP_CONSENT_URL?challenge=<base64-cbor>&state=<state>` so the WASM
/// signer can produce the COSE_Sign1 over the canonical-CBOR challenge.
pub const WEBAPP_CONSENT_URL: &str = "https://mnemonik.xyz/oauth/consent";

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
pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
    clients: Mutex<LruCache<String, RegisteredClient>>,
    /// HS256 signing key. Constructed once from `MCP_JWT_SECRET` at startup.
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
}

impl OAuthState {
    /// Build OAuthState from a 32-byte base64-decoded secret. Panics if the
    /// secret is shorter than 32 bytes — caller (main.rs) must reject startup
    /// in that case rather than allowing a weak HMAC key.
    pub fn new(secret: &[u8]) -> Self {
        assert!(
            secret.len() >= 32,
            "MCP_JWT_SECRET must decode to >= 32 bytes (got {})",
            secret.len()
        );
        let cap = NonZeroUsize::new(OAUTH_STATE_CAPACITY).expect("nonzero capacity");
        Self {
            pending: Mutex::new(LruCache::new(cap)),
            codes: Mutex::new(LruCache::new(cap)),
            clients: Mutex::new(LruCache::new(cap)),
            jwt_encoding_key: EncodingKey::from_secret(secret),
            jwt_decoding_key: DecodingKey::from_secret(secret),
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
        exp: now + JWT_TTL_SECS,
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

    let challenge_obj = serde_json::json!({
        "server_origin": SERVER_ORIGIN,
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
        SERVER_ORIGIN,
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
    let location = format!("{WEBAPP_CONSENT_URL}?challenge={challenge_param}&state={state_param}");
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
const REDIRECT_EXACT: &[&str] = &["https://mnemonik.xyz/oauth/consent"];

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
/// 3. Loopback callback `http://127.0.0.1:<port>[/<path>]` or
///    `http://[::1]:<port>[/<path>]` for any client (RFC 8252 §7.3).
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
/// `http://127.0.0.1.evil.com`) → false.
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
        return None;
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

/// Match `^http://127\.0\.0\.1:\d+(/.*)?$` and `^http://\[::1\]:\d+(/.*)?$`
/// per RFC 8252 §7.3 without pulling in a `regex` crate. Hand-rolled is
/// cheaper and fully covered by unit tests.
///
/// Rules:
///   - host MUST be exactly `127.0.0.1` or `[::1]` (the leading-`http://`
///     + exact-host strip rejects e.g. `http://127.0.0.1.evil.com:80/`)
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

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub code: String,
    pub code_verifier: String,
    /// `redirect_uri` from the original authorize request. Optional for
    /// backward compatibility with clients that omit it (legacy webapp,
    /// integration tests). When present, MUST equal the value bound at
    /// `/oauth/authorize` time — RFC 6749 §4.1.3 + RFC 7636 §4.4 require this
    /// equality check to defeat a swap-redirect attack on a leaked code.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    /// OAuth 2.1 / RFC 6749 §5.1 — the scope the token was granted.
    /// We only have one scope (`mcp`); echoed for clients that read it.
    pub scope: String,
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
    // Decide between JSON and x-www-form-urlencoded. Default to JSON if
    // the header is missing — keeps existing programmatic clients
    // (`scripts/test-oauth-flow.sh`, integration tests) working.
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_lowercase();

    let req: TokenRequest = if ct.starts_with("application/x-www-form-urlencoded") {
        match serde_urlencoded::from_bytes(&body) {
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
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    &format!("token request JSON parse failed: {e}"),
                );
            }
        }
    };

    let issued = {
        let mut guard = state.codes.lock().expect("codes mutex poisoned");
        guard.pop(&req.code)
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
    let digest = Sha256::digest(req.code_verifier.as_bytes());
    let derived = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest);
    if derived != issued.code_challenge {
        return oauth_error(StatusCode::UNAUTHORIZED, "code_verifier does not match");
    }

    let token = match issue_jwt_with_google_sub(&state, &issued.sub, issued.google_sub.clone()) {
        Ok(t) => t,
        Err(e) => {
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("JWT issuance failed: {e}"),
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
    let body = TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: JWT_TTL_SECS,
        scope: "mcp".to_string(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Persist `token` to `~/.mnemonic/token.json` as a best-effort cache. The
/// `expires_at` field is derived from [`JWT_TTL_SECS`] so the on-disk
/// timestamp matches the JWT's own `exp` claim within ~1s. Errors are
/// logged at `warn` and swallowed — a cache failure must never break the
/// OAuth response (the calling agent has already received its token).
///
/// Public so future binary-side outbound flows can persist tokens
/// minted through alternate paths (Google OAuth, extension key-escrow).
pub fn cache_minted_token(jwt: &str, sub: &str) {
    let expires_at =
        (chrono::Utc::now() + chrono::Duration::seconds(JWT_TTL_SECS as i64)).to_rfc3339();
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

/// Build a uniform OAuth-style error envelope.
fn oauth_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
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
    let body = serde_json::json!({
        "issuer": SERVER_ORIGIN,
        "authorization_endpoint": format!("{SERVER_ORIGIN}/oauth/authorize"),
        "token_endpoint": format!("{SERVER_ORIGIN}/oauth/token"),
        "registration_endpoint": format!("{SERVER_ORIGIN}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /.well-known/oauth-protected-resource` — MCP spec metadata document
/// for the server origin as a whole.
pub async fn oauth_protected_resource_metadata() -> Response {
    let body = serde_json::json!({
        "resource": SERVER_ORIGIN,
        "authorization_servers": [SERVER_ORIGIN],
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
    let body = serde_json::json!({
        "resource": format!("{SERVER_ORIGIN}/mcp"),
        "authorization_servers": [SERVER_ORIGIN],
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
    let allowlisted = method
        .as_deref()
        .map(|m| {
            // Explicit allowlist: discovery + lifecycle methods.
            // Notifications (per JSON-RPC 2.0 spec — no `id` field) and MCP
            // `notifications/*` methods are also pass-through. Without this
            // Cursor's post-initialize `notifications/initialized` is rejected
            // with 401, breaking the handshake.
            ALLOWLIST_METHODS.contains(&m) || m.starts_with("notifications/")
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
        let www_auth = format!(
            "Bearer realm=\"{issuer}\", error=\"invalid_token\", \
             error_description=\"{esc_msg}\", \
             resource_metadata=\"{issuer}/.well-known/oauth-protected-resource\"",
            issuer = "https://mcp.mnemonik.xyz",
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
        Arc::new(OAuthState::new(TEST_SECRET))
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
            "server_origin": SERVER_ORIGIN,
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
            "server_origin": SERVER_ORIGIN,
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
            SERVER_ORIGIN,
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
            "server_origin": SERVER_ORIGIN,
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
        assert_eq!(parsed["issuer"], SERVER_ORIGIN);
        assert_eq!(
            parsed["authorization_endpoint"],
            format!("{SERVER_ORIGIN}/oauth/authorize")
        );
        assert_eq!(
            parsed["token_endpoint"],
            format!("{SERVER_ORIGIN}/oauth/token")
        );
        assert_eq!(parsed["response_types_supported"][0], "code");
        assert_eq!(parsed["grant_types_supported"][0], "authorization_code");
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
        assert_eq!(parsed["resource"], SERVER_ORIGIN);
        assert_eq!(parsed["authorization_servers"][0], SERVER_ORIGIN);
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
        assert_eq!(
            parsed["resource"],
            format!("{SERVER_ORIGIN}/mcp"),
            "resource MUST equal the URL the MCP client connects to (path-specific)"
        );
        assert_eq!(parsed["authorization_servers"][0], SERVER_ORIGIN);
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
                allowed_redirect("http://[::1]:33418/", client),
                "loopback v6 / must accept client_id={client:?}"
            );
            // Path "/callback" — mnemonic-cli's path.
            assert!(allowed_redirect("http://127.0.0.1:1234/callback", client));
            // Path with extra segments — also allowed per RFC 8252.
            assert!(allowed_redirect(
                "http://127.0.0.1:1234/callback/extra",
                client
            ));
            // No path at all (just port).
            assert!(allowed_redirect("http://127.0.0.1:1234", client));
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
                "http://[::1]:33418/".to_string(),
            ],
        );

        assert!(st.allows_redirect("https://vscode.dev/redirect", &client_id));
        assert!(st.allows_redirect("http://127.0.0.1:59656/", &client_id));
        assert!(st.allows_redirect("http://[::1]:59656/", &client_id));
        assert!(!st.allows_redirect("https://evil.com/redirect", &client_id));
        assert!(!st.allows_redirect("http://127.0.0.1.evil.com:59656/", &client_id));
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
            "server_origin": SERVER_ORIGIN,
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
}
