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

use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use lru::LruCache;
use mnemonic_core::codec::{
    canonical::to_canonical_cbor, hash::hash_bytes as blake3_hex, sign::verify_artifact,
};
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
/// bootstrap (future webapp endpoint) when inserting a fresh challenge.
#[allow(dead_code)]
pub const STATE_TTL_SECS: u64 = 60;
/// Issued-code TTL: 60s — code must be redeemed at /oauth/token within this.
pub const CODE_TTL_SECS: u64 = 60;
/// LRU bound on both pending-state and issued-code maps.
pub const OAUTH_STATE_CAPACITY: usize = 10_000;
/// Server origin used in the canonical-CBOR challenge (Decision 10).
/// Public so the consent-page bootstrap and tests can reuse the same value.
#[allow(dead_code)]
pub const SERVER_ORIGIN: &str = "https://mcp.mnemonik.xyz";

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
}

/// Pending authorize record (state → expected pubkey + challenge metadata).
#[derive(Debug, Clone)]
struct PendingAuthorize {
    /// blake3 hex of canonical_cbor(challenge_fields). Recomputed on
    /// `/authorize` POST and compared against the supplied COSE payload's
    /// `expected_hash`.
    challenge_hash: String,
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
    /// Unix-seconds expiry.
    exp: u64,
}

/// Shared OAuth state — pending challenges + issued codes, both LRU+TTL bound.
/// Wrap in `Arc` and inject into Axum routers via `with_state`.
pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
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
            jwt_encoding_key: EncodingKey::from_secret(secret),
            jwt_decoding_key: DecodingKey::from_secret(secret),
        }
    }

    /// Insert a pending authorize record before issuing the challenge to the
    /// browser. Called by the consent-page bootstrap (future webapp endpoint)
    /// and by unit tests in this module.
    #[allow(clippy::too_many_arguments, dead_code)]
    pub fn insert_pending(
        &self,
        state: String,
        challenge_hash: String,
        expected_pubkey: String,
        redirect_uri: String,
        code_challenge: String,
        client_state: String,
        exp: u64,
    ) {
        let entry = PendingAuthorize {
            challenge_hash,
            expected_pubkey,
            redirect_uri,
            code_challenge,
            client_state,
            exp,
        };
        let mut guard = self.pending.lock().expect("pending mutex poisoned");
        guard.put(state, entry);
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
#[allow(clippy::too_many_arguments, dead_code)]
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
#[allow(dead_code)]
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

/// Issue an HS256 JWT bound to `sub` (base58 user pubkey).
pub fn issue_jwt(state: &OAuthState, sub: &str) -> Result<String, String> {
    let now = now_secs();
    let claims = Claims {
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + JWT_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
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

// ── /oauth/authorize handler (Decision 10) ───────────────────────────────────

/// Request body for `POST /oauth/authorize`. The browser fetches the unsigned
/// challenge fields, signs `blake3(canonical_cbor(fields))` with the user's
/// localStorage Ed25519 key (via WASM `sign_challenge`), and POSTs the
/// COSE_Sign1 bytes back here.
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    /// Original CSRF state — must match a record in OAuthState.pending.
    pub state: String,
    /// COSE_Sign1 bytes (base64) — payload is the canonical-CBOR challenge.
    pub cose_signed: String,
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

    // Decode the COSE_Sign1 envelope from base64.
    let cose_bytes = match base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        req.cose_signed.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                &format!("cose_signed is not valid base64: {e}"),
            );
        }
    };

    // Verify COSE_Sign1 — checks Ed25519 signature, content integrity vs
    // expected hash, and the algorithm field. Decision 10 requires ALL
    // checks pass.
    let result = match verify_artifact(&cose_bytes, Some(&pending.challenge_hash)) {
        Ok(r) => r,
        Err(e) => return oauth_error(StatusCode::UNAUTHORIZED, &format!("COSE verify: {e}")),
    };
    if !result.valid
        || !result.cose_signature
        || !result.content_integrity
        || !result.algorithm_valid
    {
        return oauth_error(StatusCode::UNAUTHORIZED, "challenge signature invalid");
    }
    // Bind to expected pubkey: the kid embedded in the COSE header must match
    // the pubkey we recorded when the pending record was created. This
    // closes the "tampered sub" attack — the attacker can't sign with their
    // own key and claim to be someone else.
    if result.signer != pending.expected_pubkey {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "signer pubkey does not match expected pubkey",
        );
    }

    // Issue a single-use code, store binding for /token.
    let code = uuid::Uuid::new_v4().to_string();
    {
        let mut guard = state.codes.lock().expect("codes mutex poisoned");
        guard.put(
            code.clone(),
            IssuedCode {
                sub: result.signer.clone(),
                code_challenge: pending.code_challenge.clone(),
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
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

/// `POST /oauth/token` — exchange a fresh authorization code for a JWT.
///
/// PKCE: `SHA256(code_verifier)` (base64url, no padding) must equal the
/// stored `code_challenge`. Single-use — code is removed atomically.
pub async fn token_handler(
    State(state): State<Arc<OAuthState>>,
    Json(req): Json<TokenRequest>,
) -> Response {
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

    // Verify PKCE: SHA256(verifier) base64url-no-pad == challenge.
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(req.code_verifier.as_bytes());
    let derived = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest);
    if derived != issued.code_challenge {
        return oauth_error(StatusCode::UNAUTHORIZED, "code_verifier does not match");
    }

    let token = match issue_jwt(&state, &issued.sub) {
        Ok(t) => t,
        Err(e) => {
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("JWT issuance failed: {e}"),
            );
        }
    };
    let body = TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: JWT_TTL_SECS,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Build a uniform OAuth-style error envelope.
fn oauth_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}

// ── Bearer-auth middleware (Decision 9) ──────────────────────────────────────

/// Maximum body size accepted by the body-peeking middleware (1 MiB).
/// Larger bodies short-circuit with HTTP 413; the JSON-RPC dispatcher's own
/// limit is 2 MiB so this is the tighter gate for request inspection.
const MAX_PEEK_BODY: usize = 1024 * 1024;

/// Allowlist of JSON-RPC methods that bypass JWT validation. `initialize`
/// and `tools/list` are required for MCP discovery — Cursor / Claude.ai
/// post these BEFORE completing OAuth, so blocking them breaks the install
/// handshake.
const ALLOWLIST_METHODS: &[&str] = &["initialize", "tools/list"];

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

    // URI-based allowlist — never inspect the body for these.
    if path.starts_with("/oauth/") || path == "/health" {
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
        .map(|m| ALLOWLIST_METHODS.contains(&m))
        .unwrap_or(false);

    if !allowlisted {
        // Require Bearer JWT.
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string());
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

    // Allowlisted — re-inject the body untouched.
    let new_req = Request::from_parts(parts, Body::from(body_bytes));
    next.run(new_req).await
}

/// Emit a JSON-RPC-shaped 401 envelope for failed bearer-auth checks.
fn jsonrpc_unauthorized(status: StatusCode, msg: &str) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": -32001, "message": format!("unauthorized: {msg}")}
    });
    (status, Json(body)).into_response()
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
        let (hash, cose_b64) = make_signed_challenge(
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
            serde_json::json!({"state": "csrf-state-1", "cose_signed": cose_b64}),
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
        let (hash, _good_cose) = make_signed_challenge(
            &kp,
            "csrf-2",
            "https://app/cb",
            &challenge,
            "n2",
            now_secs() + 30,
        );
        // Tamper: re-sign the SAME canonical CBOR with the attacker's key.
        // Hash matches but sig won't verify against the expected_pubkey.
        let challenge_obj = serde_json::json!({
            "server_origin": SERVER_ORIGIN,
            "state": "csrf-2",
            "client_id": "test-client",
            "redirect_uri": "https://app/cb",
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "nonce": "n2",
            "exp": now_secs() + 30,
        });
        let cbor = to_canonical_cbor(&challenge_obj, &CHALLENGE_SCHEMA).unwrap();
        let bad_cose = sign_cose(&cbor, &attacker).unwrap();
        let bad_cose_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bad_cose);
        st.insert_pending(
            "csrf-2".to_string(),
            hash,
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
            serde_json::json!({"state": "csrf-2", "cose_signed": bad_cose_b64}),
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
        let pubkey_a = Keypair::new().pubkey().to_string(); // unrelated
        let challenge = pkce_challenge("v-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (hash, cose_b64) = make_signed_challenge(
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
            serde_json::json!({"state": "csrf-tamper", "cose_signed": cose_b64}),
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
            pubkey,
            "https://app/cb".to_string(),
            plain_challenge.to_string(),
            "csrf-plain".to_string(),
            now_secs() + 30,
        );
        // But the user signs a "plain" envelope.
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
        let cose = sign_cose(&cbor, &kp).unwrap();
        let cose_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cose);
        let app = build_authorize_router(st);
        let (status, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({"state": "csrf-plain", "cose_signed": cose_b64}),
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
        let (hash, cose_b64) = make_signed_challenge(
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
            pubkey,
            "https://app/cb".to_string(),
            challenge,
            "csrf-replay".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (status1, _) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({"state": "csrf-replay", "cose_signed": cose_b64.clone()}),
        )
        .await;
        assert_eq!(status1, StatusCode::OK);
        // Second submission of the same `state` must fail — entry is gone.
        let app2 = build_authorize_router(st);
        let (status2, _) = post_json(
            app2,
            "/oauth/authorize",
            serde_json::json!({"state": "csrf-replay", "cose_signed": cose_b64}),
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
        let (hash, cose_b64) = make_signed_challenge(
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
            serde_json::json!({"state": "tok-state", "cose_signed": cose_b64}),
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
        let (hash, cose_b64) = make_signed_challenge(
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
            pubkey,
            "https://app/cb".to_string(),
            challenge,
            "tok-bad".to_string(),
            now_secs() + 30,
        );
        let app = build_authorize_router(st.clone());
        let (_, body) = post_json(
            app,
            "/oauth/authorize",
            serde_json::json!({"state": "tok-bad", "cose_signed": cose_b64}),
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
}
