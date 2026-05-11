//! Integration test: full `/oauth/authorize` -> `/oauth/token` round-trip.
//!
//! Boots an in-process Axum app with the OAuth routes, walks through:
//!   1. Generate Ed25519 keypair (the user's localStorage key, modeled in-test).
//!   2. Insert a pending authorize record matching the canonical-CBOR
//!      challenge fields (Decision 10).
//!   3. POST `/oauth/authorize` with the COSE_Sign1-signed challenge.
//!   4. POST `/oauth/token` with the matching `code_verifier`.
//!   5. Decode the returned JWT and assert `iss/aud/sub` match Decision 11.
//!
//! Task 8 owns the bulk of integration-test coverage; this stub is the
//! minimum closing the loop for Task 4's smoke gate.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::identity::sign_bytes;
use mnemonic_mcp::oauth::{
    self, authorize_handler, build_challenge_hash, token_handler, Claims, OAuthState, JWT_AUDIENCE,
    JWT_ISSUER, SERVER_ORIGIN,
};
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use std::collections::HashSet;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"integration-test-secret-32-bytes";

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest)
}

fn build_oauth_router(state: Arc<OAuthState>) -> Router {
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
    let parsed: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::String(
        String::from_utf8_lossy(&body_bytes).into_owned(),
    ));
    (status, parsed)
}

#[tokio::test]
async fn full_authorize_token_jwt_roundtrip() {
    let st = Arc::new(OAuthState::new(TEST_SECRET));
    let kp = Keypair::new();
    let pubkey = kp.pubkey().to_string();

    // PKCE setup.
    let verifier = "integration-test-verifier-43-chars-aaaaaaaa";
    let code_challenge = pkce_challenge(verifier);

    // Build the canonical-CBOR challenge fields and sign with the user's keypair.
    let client_state = "csrf-integration";
    let redirect_uri = "https://app.example/callback";
    let nonce = "nonce-int";
    let exp = now_secs() + 30;

    let challenge_hash = build_challenge_hash(
        SERVER_ORIGIN,
        client_state,
        "test-client",
        redirect_uri,
        &code_challenge,
        "S256",
        nonce,
        exp,
    )
    .unwrap();

    // Sign the same canonical CBOR via the same primitive the WASM signer
    // would call in production.
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
    // Reuse the same schema the server uses for hash computation.
    use mnemonic_core::codec::schema::{ArtifactSchema, ArtifactType};
    let schema = ArtifactSchema {
        artifact_type: ArtifactType::Receipt,
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
    let cbor = to_canonical_cbor(&challenge, &schema).unwrap();
    // Raw Ed25519 over the canonical CBOR — the post-refactor wire contract
    // (the handler validates `signature` is exactly 64 bytes and calls
    // `identity::verify_signature` against `pending.challenge_bytes`).
    let sig = sign_bytes(&kp, &cbor);
    let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig);

    // Insert the pending record on the server side. `challenge_bytes` must
    // be the canonical-CBOR bytes the user signed — the handler verifies
    // the signature against those exact bytes.
    st.insert_pending(
        client_state.to_string(),
        challenge_hash,
        cbor,
        pubkey.clone(),
        redirect_uri.to_string(),
        code_challenge.clone(),
        client_state.to_string(),
        exp,
    );

    // Step 1: /oauth/authorize.
    let app = build_oauth_router(st.clone());
    let (s1, body1) = post_json(
        app,
        "/oauth/authorize",
        serde_json::json!({
            "state": client_state,
            "signature": sig_b64,
            "signer_pubkey": pubkey,
        }),
    )
    .await;
    assert_eq!(s1, StatusCode::OK, "authorize body: {body1}");
    let code = body1["code"].as_str().expect("code field").to_string();
    assert!(code.len() >= 8, "code looks short: {code}");
    assert_eq!(body1["state"], client_state);
    assert_eq!(body1["redirect_uri"], redirect_uri);

    // Step 2: /oauth/token.
    let app = build_oauth_router(st.clone());
    let (s2, body2) = post_json(
        app,
        "/oauth/token",
        serde_json::json!({"code": code, "code_verifier": verifier}),
    )
    .await;
    assert_eq!(s2, StatusCode::OK, "token body: {body2}");
    let token = body2["access_token"]
        .as_str()
        .expect("access_token")
        .to_string();
    assert_eq!(body2["token_type"], "Bearer");
    assert_eq!(body2["expires_in"].as_u64(), Some(3600));

    // Step 3: decode + verify the JWT — independent of OAuthState's
    // verify_jwt to prove the externally-observable token is well-formed.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[JWT_AUDIENCE]);
    let mut iss_set = HashSet::new();
    iss_set.insert(JWT_ISSUER.to_string());
    validation.iss = Some(iss_set);
    let data = decode::<Claims>(&token, &DecodingKey::from_secret(TEST_SECRET), &validation)
        .expect("JWT decodes against TEST_SECRET");
    assert_eq!(data.claims.iss, JWT_ISSUER);
    assert_eq!(data.claims.aud, JWT_AUDIENCE);
    assert_eq!(data.claims.sub, pubkey);
    assert!(data.claims.exp > data.claims.iat);
    let _ = uuid::Uuid::parse_str(&data.claims.jti).expect("jti is uuid");

    // Sanity: also pass through OAuthState's validator.
    let claims = oauth::verify_jwt(&st, &token).expect("verify_jwt round-trip");
    assert_eq!(claims.sub, pubkey);
}
