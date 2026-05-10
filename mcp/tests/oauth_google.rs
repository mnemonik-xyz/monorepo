//! Integration tests for the Google OAuth provider (T14 of
//! `work/chrome-extension/`). Drives Decision 5 + Decision 9 end-to-end:
//!
//! - **Full PKCE round-trip:** start → callback (mocked Google) → lookup →
//!   link → exchange code at `/oauth/token` for a server JWT carrying
//!   `google_sub`.
//! - **Possession proof:** `/oauth/google/link` rejects requests whose
//!   `possession_proof_base64` doesn't sign the nonce returned by `/lookup`.
//! - **id_token bad audience:** the JWKS verifier rejects an `id_token`
//!   whose `aud` claim doesn't match `GOOGLE_OAUTH_CLIENT_ID`.
//! - **id_token bad signature:** a token signed with a different RSA key
//!   (kid mismatch / signature fail) is rejected.
//!
//! The mock Google server is an axum sub-router that mints an RS256
//! keypair at test setup, exposes the JWK at `/oauth2/v3/certs`, and signs
//! tokens at `/token`. No network access required.
//!
//! Run with: `cargo test -p mnemonic-mcp --features test-support --test oauth_google -- --nocapture`

#![cfg(feature = "test-support")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine as _;
use http_body_util::BodyExt;
use jsonwebtoken::{encode as jwt_encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use mnemonic_core::identity::sign_bytes;
use mnemonic_mcp::{
    oauth::{
        self,
        google::{
            self, google_callback_handler, google_link_handler, google_lookup_handler,
            google_start_handler, GoogleOAuthState,
        },
        google_jwks::GoogleJwksCache,
        OAuthState,
    },
    test_support::{mint_jwt, mock_state},
};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_sdk::signature::{Keypair, Signer};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"oauth-google-test-secret-32bytes";
const TEST_CLIENT_ID: &str = "test-google-client-id.apps.googleusercontent.com";
const TEST_CLIENT_SECRET: &str = "test-google-client-secret";
const TEST_KID: &str = "test-kid-2026";

// ── Mock Google server ───────────────────────────────────────────────────────

struct MockGoogle {
    private_key: RsaPrivateKey,
    /// id_token the next /token call should return (set per-test).
    next_id_token: Mutex<Option<String>>,
    /// kid embedded in JWKs we serve.
    kid: String,
}

impl MockGoogle {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA keygen");
        Self {
            private_key,
            next_id_token: Mutex::new(None),
            kid: TEST_KID.to_string(),
        }
    }

    fn public_key(&self) -> RsaPublicKey {
        RsaPublicKey::from(&self.private_key)
    }

    fn jwk(&self) -> Value {
        let pk = self.public_key();
        let n_bytes = pk.n().to_bytes_be();
        let e_bytes = pk.e().to_bytes_be();
        serde_json::json!({
            "kid": self.kid,
            "use": "sig",
            "kty": "RSA",
            "alg": "RS256",
            "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(n_bytes),
            "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(e_bytes),
        })
    }

    fn jwks(&self) -> Value {
        serde_json::json!({ "keys": [self.jwk()] })
    }

    /// Sign an id_token with this mock's RSA key + kid.
    fn sign_id_token(&self, claims: &Value, kid_override: Option<&str>) -> String {
        let pem = self
            .private_key
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid_override.unwrap_or(&self.kid).to_string());
        jwt_encode(&header, claims, &key).unwrap()
    }
}

#[derive(Clone)]
struct MockState(Arc<MockGoogle>);

async fn handler_jwks(State(s): State<MockState>) -> Response {
    Json(s.0.jwks()).into_response()
}

#[derive(serde::Deserialize)]
struct TokenForm {
    #[allow(dead_code)]
    code: String,
    client_id: String,
    client_secret: String,
    #[allow(dead_code)]
    redirect_uri: String,
    #[allow(dead_code)]
    grant_type: String,
}

async fn handler_token(State(s): State<MockState>, body: axum::body::Bytes) -> Response {
    let form: TokenForm = match serde_urlencoded::from_bytes(&body) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("form parse: {e}")})),
            )
                .into_response()
        }
    };
    if form.client_id != TEST_CLIENT_ID || form.client_secret != TEST_CLIENT_SECRET {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "bad client credentials"})),
        )
            .into_response();
    }
    let id_token =
        s.0.next_id_token
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
    Json(serde_json::json!({
        "access_token": "mock-access-token",
        "expires_in": 3600,
        "token_type": "Bearer",
        "id_token": id_token,
    }))
    .into_response()
}

fn build_mock_google(mock: Arc<MockGoogle>) -> Router {
    Router::new()
        .route("/oauth2/v3/certs", get(handler_jwks))
        .route("/token", post(handler_token))
        .with_state(MockState(mock))
}

/// Spawn the mock Google server on an ephemeral port; return its base URL.
async fn spawn_mock_google(mock: Arc<MockGoogle>) -> String {
    let app = build_mock_google(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

// ── Test harness builders ────────────────────────────────────────────────────

fn build_app(
    mcp: Arc<mnemonic_mcp::mcp::McpState>,
    oauth_state: Arc<OAuthState>,
    google: Arc<GoogleOAuthState>,
) -> Router {
    let public = Router::new()
        .route("/oauth/google/start", get(google_start_handler))
        .route("/oauth/google/callback", get(google_callback_handler))
        .route("/oauth/google/lookup", post(google_lookup_handler))
        .route("/oauth/google/link", post(google_link_handler))
        .with_state((mcp, oauth_state.clone(), google));
    let token = Router::new()
        .route("/oauth/token", post(oauth::token_handler))
        .with_state(oauth_state);
    Router::new().merge(public).merge(token)
}

struct Harness {
    app: Router,
    mock: Arc<MockGoogle>,
    google: Arc<GoogleOAuthState>,
    oauth_state: Arc<OAuthState>,
    mcp: Arc<mnemonic_mcp::mcp::McpState>,
}

async fn make_harness() -> Harness {
    let mock = Arc::new(MockGoogle::new());
    let base = spawn_mock_google(mock.clone()).await;
    let jwks_url = format!("{base}/oauth2/v3/certs");
    let token_url = format!("{base}/token");
    let jwks = Arc::new(GoogleJwksCache::new_with_base_url(jwks_url));
    let google = Arc::new(GoogleOAuthState::with_endpoints(
        TEST_CLIENT_ID.to_string(),
        TEST_CLIENT_SECRET.to_string(),
        "https://mc.example.com/oauth/google/callback".to_string(),
        format!("{base}/oauth/google/auth"), // unused in this test (we don't follow the start redirect)
        token_url,
        jwks,
    ));
    let oauth_state = Arc::new(OAuthState::new(TEST_SECRET));
    let mcp = mock_state();
    google::migrate_google_identity_links(mcp.store.lock().unwrap().conn()).unwrap();
    let app = build_app(mcp.clone(), oauth_state.clone(), google.clone());
    Harness {
        app,
        mock,
        google,
        oauth_state,
        mcp,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn pkce_pair() -> (String, String) {
    let verifier = "verifier-deterministic-43chars-aaaaaaaaaaa".to_string();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

async fn get_redirect(app: &Router, uri: &str) -> (StatusCode, Option<String>, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::String(
            String::from_utf8_lossy(&body_bytes).into_owned(),
        ))
    };
    (status, location, parsed)
}

async fn post_json_auth(
    app: &Router,
    uri: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, parsed)
}

fn extract_query(url: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(q) = url.split_once('?').map(|(_, q)| q) {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                out.insert(url_decode(k), url_decode(v));
            }
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let h = &s[i + 1..i + 3];
                if let Ok(b) = u8::from_str_radix(h, 16) {
                    out.push(b);
                }
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn id_token_claims(sub: &str, aud: &str) -> Value {
    serde_json::json!({
        "iss": "https://accounts.google.com",
        "sub": sub,
        "aud": aud,
        "iat": now_secs(),
        "exp": now_secs() + 3600,
        "email": "user@example.com",
        "email_verified": true,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn full_pkce_roundtrip() {
    let h = make_harness().await;
    let google_sub = "1234567890-fixture";
    // Pre-arm the mock's id_token before triggering the callback.
    let claims = id_token_claims(google_sub, TEST_CLIENT_ID);
    let id_tok = h.mock.sign_id_token(&claims, None);
    *h.mock.next_id_token.lock().unwrap() = Some(id_tok);

    let (verifier, code_challenge) = pkce_pair();
    let ext_state = "ext-state-csrf-token";
    let ext_redirect = "https://abcdef.chromiumapp.org/google";

    // 1. Hit /oauth/google/start — server should issue a 302 to Google
    //    consent with a server-generated `state` (different from ours).
    let start_url = format!(
        "/oauth/google/start?client_id=mnemonic-extension&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding(ext_redirect),
        urlencoding(&code_challenge),
        urlencoding(ext_state),
    );
    let (status, loc, _) = get_redirect(&h.app, &start_url).await;
    assert_eq!(status, StatusCode::FOUND);
    let google_url = loc.expect("start should set Location");
    assert!(
        google_url.starts_with("http://"),
        "start->Google URL: {google_url}"
    );
    let q = extract_query(&google_url);
    let server_state = q.get("state").expect("state param").clone();
    assert_eq!(q.get("client_id").map(|s| s.as_str()), Some(TEST_CLIENT_ID));
    assert_eq!(q.get("response_type").map(|s| s.as_str()), Some("code"));

    // 2. Simulate Google redirecting back to our /callback with the
    //    server_state we just stored.
    let cb_url = format!(
        "/oauth/google/callback?code=mock-google-code&state={}",
        urlencoding(&server_state)
    );
    let (status, loc, _) = get_redirect(&h.app, &cb_url).await;
    assert_eq!(status, StatusCode::FOUND);
    let final_url = loc.expect("callback Location");
    assert!(
        final_url.starts_with(ext_redirect),
        "redirect target: {final_url} (expected prefix {ext_redirect})"
    );
    let q = extract_query(&final_url);
    let server_code = q.get("code").expect("server code in redirect").clone();
    assert_eq!(q.get("state").map(|s| s.as_str()), Some(ext_state));

    // 3. Exchange the server code for a JWT at /oauth/token. The PKCE
    //    code_verifier must match the challenge we sent at /start.
    let body = serde_json::json!({
        "code": server_code,
        "code_verifier": verifier,
        "redirect_uri": ext_redirect,
    });
    let (status, val) = post_json_auth(&h.app, "/oauth/token", body, None).await;
    assert_eq!(status, StatusCode::OK, "token exchange failed: {val:?}");
    let access_token = val
        .get("access_token")
        .and_then(|v| v.as_str())
        .expect("access_token")
        .to_string();

    // 4. Decode the JWT and assert google_sub is set.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[oauth::JWT_AUDIENCE]);
    let mut iss_set = std::collections::HashSet::new();
    iss_set.insert(oauth::JWT_ISSUER.to_string());
    validation.iss = Some(iss_set);
    let data = jsonwebtoken::decode::<serde_json::Value>(
        &access_token,
        &DecodingKey::from_secret(TEST_SECRET),
        &validation,
    )
    .expect("decode server JWT");
    let claim_obj = data.claims.as_object().unwrap();
    assert_eq!(
        claim_obj.get("google_sub").and_then(|v| v.as_str()),
        Some(google_sub),
        "JWT must carry google_sub claim"
    );

    // 5. lookup should now return no existing pubkey + a fresh link challenge.
    let (status, val) = post_json_auth(
        &h.app,
        "/oauth/google/lookup",
        serde_json::json!({}),
        Some(&access_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "lookup failed: {val:?}");
    assert!(val["existing_pubkey"].is_null());
    let challenge = val["link_challenge"]
        .as_str()
        .expect("link_challenge")
        .to_string();

    // 6. Sign the nonce with a fresh Ed25519 key + POST /link.
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let nonce_bytes = base64::engine::general_purpose::STANDARD
        .decode(&challenge)
        .unwrap();
    let sig = sign_bytes(&kp, &nonce_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig);

    let body = serde_json::json!({
        "pubkey_base58": pubkey,
        "possession_proof_base64": sig_b64,
        "challenge": challenge,
    });
    let (status, val) =
        post_json_auth(&h.app, "/oauth/google/link", body, Some(&access_token)).await;
    assert_eq!(status, StatusCode::OK, "link failed: {val:?}");
    assert_eq!(val["status"].as_str(), Some("ok"));
    assert_eq!(val["pubkey_base58"].as_str(), Some(pubkey.as_str()));

    // The freshly minted JWT in /link's response is bound to the user pubkey.
    let fresh_token = val["access_token"].as_str().unwrap();
    let data = jsonwebtoken::decode::<serde_json::Value>(
        fresh_token,
        &DecodingKey::from_secret(TEST_SECRET),
        &validation,
    )
    .expect("decode post-link JWT");
    let claim_obj = data.claims.as_object().unwrap();
    assert_eq!(
        claim_obj.get("sub").and_then(|v| v.as_str()),
        Some(pubkey.as_str())
    );
    assert_eq!(
        claim_obj.get("google_sub").and_then(|v| v.as_str()),
        Some(google_sub)
    );

    // 7. A second lookup should now return existing_pubkey populated and NO
    //    further link_challenge.
    let (status, val) = post_json_auth(
        &h.app,
        "/oauth/google/lookup",
        serde_json::json!({}),
        Some(fresh_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "post-link lookup failed: {val:?}");
    assert_eq!(val["existing_pubkey"].as_str(), Some(pubkey.as_str()));
    assert!(val.get("link_challenge").is_none() || val["link_challenge"].is_null());

    // Silence unused-var warnings from the harness fields not used in this test.
    let _ = (&h.google, &h.oauth_state, &h.mcp);
}

#[tokio::test]
async fn link_requires_possession_proof() {
    let h = make_harness().await;
    let google_sub = "link-proof-test-sub";
    // Mint a JWT that already carries google_sub so we can hit /lookup +
    // /link without re-running the whole start→callback chain.
    let token = mint_google_jwt(google_sub);

    // Acquire a link_challenge via /lookup.
    let (status, val) = post_json_auth(
        &h.app,
        "/oauth/google/lookup",
        serde_json::json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let challenge = val["link_challenge"].as_str().unwrap().to_string();

    // ── Case 1: bad signature (random bytes) → 401 ───────────────────────
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();
    let bad_sig = vec![0xAB; 64];
    let body = serde_json::json!({
        "pubkey_base58": pubkey,
        "possession_proof_base64": base64::engine::general_purpose::STANDARD.encode(&bad_sig),
        "challenge": challenge,
    });
    let (status, val) = post_json_auth(&h.app, "/oauth/google/link", body, Some(&token)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bad sig must be rejected: {val:?}"
    );

    // ── Case 2: link without an outstanding challenge → 401 ──────────────
    // (The bad-sig call above popped the challenge atomically — single-use.)
    let nonce_bytes = base64::engine::general_purpose::STANDARD
        .decode(&challenge)
        .unwrap();
    let real_sig = sign_bytes(&kp, &nonce_bytes);
    let body = serde_json::json!({
        "pubkey_base58": pubkey,
        "possession_proof_base64": base64::engine::general_purpose::STANDARD.encode(real_sig),
        "challenge": challenge,
    });
    let (status, val) = post_json_auth(&h.app, "/oauth/google/link", body, Some(&token)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no challenge must be rejected: {val:?}"
    );

    // ── Case 3: fresh challenge + valid sig → 200 + row inserted ─────────
    let (_, val) = post_json_auth(
        &h.app,
        "/oauth/google/lookup",
        serde_json::json!({}),
        Some(&token),
    )
    .await;
    let challenge2 = val["link_challenge"].as_str().unwrap().to_string();
    let nonce_bytes2 = base64::engine::general_purpose::STANDARD
        .decode(&challenge2)
        .unwrap();
    let real_sig2 = sign_bytes(&kp, &nonce_bytes2);
    let body = serde_json::json!({
        "pubkey_base58": pubkey,
        "possession_proof_base64": base64::engine::general_purpose::STANDARD.encode(real_sig2),
        "challenge": challenge2,
    });
    let (status, val) = post_json_auth(&h.app, "/oauth/google/link", body, Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "valid sig must succeed: {val:?}");
    assert_eq!(val["status"].as_str(), Some("ok"));

    // Verify the row landed in SQLite.
    let store = h.mcp.store.lock().unwrap();
    let row: (String, String) = store
        .conn()
        .query_row(
            "SELECT google_sub, pubkey_base58 FROM google_identity_links WHERE google_sub = ?1",
            rusqlite::params![google_sub],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(row.0, google_sub);
    assert_eq!(row.1, pubkey);
}

#[tokio::test]
async fn id_token_bad_audience_rejected() {
    let h = make_harness().await;
    // Mint an id_token whose `aud` does NOT match our client_id.
    let claims = id_token_claims("aud-fail-sub", "some-other-aud.apps.googleusercontent.com");
    *h.mock.next_id_token.lock().unwrap() = Some(h.mock.sign_id_token(&claims, None));

    let (_, code_challenge) = pkce_pair();
    let ext_state = "aud-fail-state";
    let ext_redirect = "https://abcdef.chromiumapp.org/google";

    // Trigger /start, capture the server-state.
    let start_url = format!(
        "/oauth/google/start?client_id=mnemonic-extension&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding(ext_redirect),
        urlencoding(&code_challenge),
        urlencoding(ext_state),
    );
    let (_, loc, _) = get_redirect(&h.app, &start_url).await;
    let q = extract_query(loc.as_deref().unwrap());
    let server_state = q.get("state").unwrap().clone();

    let cb_url = format!(
        "/oauth/google/callback?code=mock-google-code&state={}",
        urlencoding(&server_state)
    );
    let req = Request::builder()
        .method("GET")
        .uri(&cb_url)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bad aud must be rejected: {val:?}"
    );
}

#[tokio::test]
async fn id_token_bad_signature_rejected() {
    let h = make_harness().await;
    // Build a totally different RSA key + sign the token with IT, but serve
    // the JWKS that announces our mock's kid. Signature verification must
    // fail because the JWK that matches `kid` is for a different key.
    let mut rng = rand::thread_rng();
    let evil_key = RsaPrivateKey::new(&mut rng, 2048).expect("evil keygen");
    let pem = evil_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    let evil_enc = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();

    let claims = id_token_claims("bad-sig-sub", TEST_CLIENT_ID);
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string()); // match the JWKS kid → forces sig check on wrong key.
    let evil_token = jwt_encode(&header, &claims, &evil_enc).unwrap();
    *h.mock.next_id_token.lock().unwrap() = Some(evil_token);

    let (_, code_challenge) = pkce_pair();
    let ext_state = "sig-fail-state";
    let ext_redirect = "https://abcdef.chromiumapp.org/google";
    let start_url = format!(
        "/oauth/google/start?client_id=mnemonic-extension&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding(ext_redirect),
        urlencoding(&code_challenge),
        urlencoding(ext_state),
    );
    let (_, loc, _) = get_redirect(&h.app, &start_url).await;
    let q = extract_query(loc.as_deref().unwrap());
    let server_state = q.get("state").unwrap().clone();

    let cb_url = format!(
        "/oauth/google/callback?code=mock-google-code&state={}",
        urlencoding(&server_state)
    );
    let req = Request::builder()
        .method("GET")
        .uri(&cb_url)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bad signature must be rejected: {val:?}"
    );
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Mint a JWT bound to a placeholder Solana subject but carrying the given
/// Google sub. Used by `link_requires_possession_proof` to skip the
/// callback chain.
fn mint_google_jwt(google_sub: &str) -> String {
    // `mint_jwt` is the test-support helper. We need google_sub set, which
    // the helper doesn't expose. Encode directly with the same shape as
    // `oauth::Claims` so verify_jwt accepts it.
    use serde::Serialize;
    #[derive(Serialize)]
    struct C<'a> {
        iss: &'a str,
        aud: &'a str,
        sub: &'a str,
        iat: u64,
        exp: u64,
        jti: &'a str,
        google_sub: &'a str,
    }
    let _unused = mint_jwt; // silence unused-import warning for the helper.
    let now = now_secs();
    let c = C {
        iss: "mcp.mnemonik.xyz",
        aud: "mcp",
        sub: "test-server-sub",
        iat: now,
        exp: now + 3600,
        jti: "fixture-jti",
        google_sub,
    };
    jwt_encode(
        &Header::new(Algorithm::HS256),
        &c,
        &EncodingKey::from_secret(TEST_SECRET),
    )
    .unwrap()
}

fn urlencoding(s: &str) -> String {
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
