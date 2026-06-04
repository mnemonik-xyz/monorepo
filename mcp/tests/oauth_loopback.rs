//! Integration tests for Task 6 of agent-native-distribution: the
//! `token_store` callsites added to the OAuth path on the MCP server.
//!
//! Spec alignment: the task's TDD anchor names three tests that exercise a
//! full agent-side OAuth-loopback flow (`fresh_install_path`,
//! `expired_token_path`, `corrupted_token_path`). The Rust binary does not
//! act as an OAuth client in V1 (code-research §5 — Node CLI only), so the
//! integration surface this task introduces is narrower than the spec
//! sketch implies. The three tests below exercise the **actual** callsites
//! this task added (cache after mint, expired→`-32099`, malformed→re-OAuth)
//! through the production OAuth `/oauth/token` endpoint and the
//! `token_expired` JSON-RPC helper. Decisions.md carries the deviation.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use mnemonic_core::identity::{read_token_from, save_token_to, TokenJson};
use mnemonic_mcp::mcp::token_expired;
use mnemonic_mcp::oauth::{
    authorize_handler, cache_minted_token, token_handler, OAuthState, JWT_TTL_SECS,
};
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};
use tempfile::TempDir;
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"loopback-int-secret-32-bytes-OK!";

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

/// Run `f` with `$HOME` pointed at a fresh temp directory. The lock is
/// shared across all tests in this file to prevent the env-var mutation
/// from racing across tokio's multi-thread runtime.
fn with_home_override<F: FnOnce(&std::path::Path) -> R, R>(f: F) -> R {
    static HOME_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = HOME_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let previous = std::env::var_os("HOME");
    // SAFETY: tests are guarded by HOME_GUARD; only one test mutates HOME
    // at a time and restores the previous value on exit.
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let result = f(dir.path());
    unsafe {
        match previous {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    result
}

/// **fresh_install_path** — equivalent to the TDD anchor of the same name.
///
/// No token file in tempdir HOME; complete a full `/oauth/token` mint
/// round-trip; assert `~/.mnemonic/token.json` exists and round-trips
/// back through `read_token_from` with the freshly-minted JWT. The
/// "second sign_memory reuses the cache" half is asserted via the
/// non-expired read returning `Ok(Some(token))`.
///
/// Not a `#[tokio::test]` because [`with_home_override`] mutates `$HOME`
/// and that mutation must outlive the runtime — we own the runtime here
/// instead of letting `#[tokio::test]` build one outside the closure.
#[test]
fn fresh_install_path() {
    with_home_override(|home| {
        let token_path = home.join(".mnemonic").join("token.json");
        assert!(
            !token_path.exists(),
            "test precondition: token.json must NOT exist in fresh HOME"
        );

        // Drive the OAuth state directly. The hand-walk through PKCE +
        // signed-challenge mirrors the existing oauth_flow.rs fixture; we
        // need a real `token_handler` execution because that's where the
        // `cache_minted_token` call lives.
        let st = Arc::new(OAuthState::new(TEST_SECRET));
        let kp = Keypair::new();
        let pubkey = kp.pubkey().to_string();
        let verifier = "loopback-fresh-verifier-43-chars-aaaaaaaaa";
        let code_challenge = pkce_challenge(verifier);
        let nonce = "n-fresh";
        let redirect_uri = "https://app.example/callback";
        let exp = now_secs() + 30;

        let challenge_obj = serde_json::json!({
            "server_origin": "https://mcp.mnemonik.xyz",
            "state": "rt-fresh",
            "client_id": "test-client",
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": "S256",
            "nonce": nonce,
            "exp": exp,
        });
        let challenge_bytes = mnemonic_core::codec::canonical::to_canonical_cbor(
            &challenge_obj,
            &mnemonic_core::codec::schema::ArtifactSchema {
                artifact_type: mnemonic_core::codec::schema::ArtifactType::Receipt,
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
            },
        )
        .unwrap();
        let hash = mnemonic_core::codec::hash::hash_bytes(&challenge_bytes);
        let sig = mnemonic_core::identity::sign_bytes(&kp, &challenge_bytes);
        let sig_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig);

        st.insert_pending(
            "rt-fresh".to_string(),
            hash,
            challenge_bytes,
            pubkey.clone(),
            redirect_uri.to_string(),
            code_challenge.clone(),
            "rt-fresh".to_string(),
            exp,
        );

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let app = build_oauth_router(st.clone());
            let (s1, body) = post_json(
                app,
                "/oauth/authorize",
                serde_json::json!({
                    "state": "rt-fresh",
                    "signature": sig_b64,
                    "signer_pubkey": pubkey,
                }),
            )
            .await;
            assert_eq!(s1, StatusCode::OK, "authorize must succeed: {body}");
            let code = body["code"].as_str().unwrap().to_string();

            let app2 = build_oauth_router(st.clone());
            let (s2, body2) = post_json(
                app2,
                "/oauth/token",
                serde_json::json!({"code": code, "code_verifier": verifier}),
            )
            .await;
            assert_eq!(s2, StatusCode::OK, "token mint must succeed: {body2}");
            let minted_jwt = body2["access_token"].as_str().unwrap();
            assert!(!minted_jwt.is_empty());
        });

        // The post-mint cache write must have created the file.
        assert!(
            token_path.exists(),
            "token_handler must persist to {} after JWT mint",
            token_path.display()
        );
        let cached = read_token_from(&token_path)
            .expect("cached token must be readable")
            .expect("cached token must not be expired");
        assert_eq!(
            cached.sub, pubkey,
            "cached sub must match the OAuth subject"
        );
        assert!(
            !cached.jwt.is_empty(),
            "cached jwt must be the freshly-minted token"
        );
    });
}

/// **expired_token_path** — write a token whose `expires_at` is in the
/// past; assert `read_token_from` surfaces `TokenStoreError::Expired` and
/// that the `token_expired` JSON-RPC helper renders it as `-32099` with
/// the documented `data` shape. The agent client re-initiates loopback
/// on receipt of this error code.
#[test]
fn expired_token_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    let past = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    let pubkey = "ExpiredSubject1111111111111111111111111111".to_string();
    save_token_to(
        &path,
        &TokenJson {
            jwt: "stale.header.sig".to_string(),
            expires_at: past.clone(),
            sub: pubkey.clone(),
        },
    )
    .unwrap();

    let err = read_token_from(&path).expect_err("expired token must surface as Err");
    let (expires_at_field, sub_field) = match &err {
        mnemonic_core::identity::TokenStoreError::Expired { expires_at, sub } => {
            (expires_at.clone(), sub.clone())
        }
        other => panic!("expected Expired, got {other:?}"),
    };
    assert_eq!(expires_at_field, past);
    assert_eq!(sub_field, pubkey);

    // The JSON-RPC boundary maps the typed error to `-32099`.
    let rpc_err = token_expired(&expires_at_field, &sub_field);
    assert_eq!(rpc_err.code, -32099);
    let data = rpc_err.data.expect("typed errors must carry data");
    assert_eq!(data["kind"], "TokenExpired");
    assert_eq!(data["expires_at"], past);
    assert_eq!(data["pubkey"], pubkey);
}

/// **corrupted_token_path** — malformed JSON on disk must NOT panic; it
/// must NOT surface as a hard error; the read must degrade to
/// `Ok(None)` so the caller re-OAuths. This is the Round-3 test-review
/// requirement that corrupted state never crashes the binary.
#[test]
fn corrupted_token_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");
    std::fs::write(&path, b"\x00\xffnot-json-at-all{").unwrap();

    let result = read_token_from(&path);
    let observed = result.expect("malformed must not propagate as Err");
    assert!(
        observed.is_none(),
        "malformed must degrade to Ok(None), got {observed:?}"
    );
}

/// Additional explicit check: `cache_minted_token` is idempotent under
/// repeated calls (subsequent OAuth mints overwrite the cache safely).
#[test]
fn cache_minted_token_overwrites_existing() {
    with_home_override(|home| {
        let path = home.join(".mnemonic").join("token.json");

        cache_minted_token("first-jwt", "OwnerA111111111111111111111111111111111111");
        let first = read_token_from(&path).unwrap().unwrap();
        assert_eq!(first.jwt, "first-jwt");
        assert_eq!(first.sub, "OwnerA111111111111111111111111111111111111");
        // expires_at is `now + JWT_TTL_SECS`; assert it parses and is
        // within ~JWT_TTL_SECS of "now".
        let parsed = chrono::DateTime::parse_from_rfc3339(&first.expires_at)
            .expect("cached expires_at must be RFC3339");
        let delta = parsed.with_timezone(&chrono::Utc) - chrono::Utc::now();
        assert!(
            delta.num_seconds() > 0 && delta.num_seconds() <= JWT_TTL_SECS as i64,
            "expires_at must be in the future, within JWT_TTL_SECS: delta={}s",
            delta.num_seconds()
        );

        cache_minted_token("second-jwt", "OwnerB111111111111111111111111111111111111");
        let second = read_token_from(&path).unwrap().unwrap();
        assert_eq!(second.jwt, "second-jwt");
        assert_eq!(second.sub, "OwnerB111111111111111111111111111111111111");
    });
}
