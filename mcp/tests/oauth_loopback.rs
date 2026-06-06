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
//!
//! Round-1 test-review F3/F4: the "OAuth mock invocation count == 1"
//! invariant (TDD anchor for `fresh_install_path`) and the "OAuth re-fires
//! on corrupted token" invariant (TDD anchor for `corrupted_token_path`)
//! are intentionally deferred to Task 5 of agent-native-distribution,
//! which wires the outbound participate-mode proxy. The tests below
//! cover the V1-scope library and server-side contracts and add a
//! second-read assertion in `fresh_install_path` to prove the cache reuse
//! property a future outbound caller would rely on.

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
    authorize_handler, cache_minted_token, jwt_ttl_secs, token_handler, OAuthState,
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

/// Run `f` with `MNEMONIC_CONFIG_DIR` pointed at a fresh temp directory.
/// Replaces an earlier HOME-mutation pattern (round-1 code review
/// R1-MAJOR-2): the override is the Node CLI's existing test seam
/// (`packages/cli/src/config.ts:48-52`), supported by `token_path()` since
/// the round-1 fixes, and requires no `unsafe` env mutation. The lock is
/// shared across all tests in this file to prevent the env mutation from
/// racing across tokio runtimes.
///
/// `into_inner()` on poisoning is intentional: a panic inside one test
/// leaves the env var possibly stale, but the next test explicitly sets
/// it again before reading, so a poisoned lock does not corrupt the
/// observed environment.
fn with_config_dir_override<F: FnOnce(&std::path::Path) -> R, R>(f: F) -> R {
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let previous = std::env::var_os("MNEMONIC_CONFIG_DIR");
    // SAFETY: tests are guarded by ENV_GUARD; only one test mutates this
    // env var at a time and restores the previous value on exit. The
    // mutation is necessary because token_path() reads the env var from
    // global process state — there is no in-process configuration knob.
    unsafe {
        std::env::set_var("MNEMONIC_CONFIG_DIR", dir.path());
    }
    let result = f(dir.path());
    unsafe {
        match previous {
            Some(v) => std::env::set_var("MNEMONIC_CONFIG_DIR", v),
            None => std::env::remove_var("MNEMONIC_CONFIG_DIR"),
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
/// Not a `#[tokio::test]` because [`with_config_dir_override`] mutates
/// `MNEMONIC_CONFIG_DIR` and that mutation must outlive the runtime — we
/// own the runtime here instead of letting `#[tokio::test]` build one
/// outside the closure.
#[test]
fn fresh_install_path() {
    with_config_dir_override(|cfg_dir| {
        let token_path = cfg_dir.join("token.json");
        assert!(
            !token_path.exists(),
            "test precondition: token.json must NOT exist under fresh MNEMONIC_CONFIG_DIR"
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

        // Round-1 test review F3 — the TDD anchor says "second sign_memory
        // within TTL reuses cached token without invoking the OAuth mock".
        // The Rust binary is not an OAuth client in V1, so we cannot model
        // a full "mock call count == 1" assertion. We CAN prove the
        // surrogate property: a second read of the cached token returns
        // the same JWT, so any future outbound caller would reuse it
        // rather than re-mint. The "OAuth mock not invoked" half of the
        // contract is deferred to Task 5 (`MNEMONIC_HOSTED_ENDPOINT`
        // proxy wiring) — see decisions.md Task 6 entry.
        let cached_again = read_token_from(&token_path)
            .expect("second cache read must succeed")
            .expect("second cache read must not be expired");
        assert_eq!(
            cached_again.jwt, cached.jwt,
            "second read must return byte-identical jwt — cache reuse property"
        );
        assert_eq!(cached_again.sub, cached.sub);
        assert_eq!(cached_again.expires_at, cached.expires_at);
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
/// Uses real JWTs minted via [`issue_jwt`] so the exp-extraction path in
/// [`cache_minted_token`] is exercised — round-2 code review R2-NOTE-2.
/// A hardcoded string would silently bypass `extract_exp_unix_no_verify`
/// and fall back to the conservative `now + TTL` estimate, leaving the
/// production exp-from-JWT path untested.
#[test]
fn cache_minted_token_overwrites_existing() {
    use mnemonic_mcp::oauth::issue_jwt;
    with_config_dir_override(|cfg_dir| {
        let path = cfg_dir.join("token.json");
        let st = Arc::new(OAuthState::new(TEST_SECRET));
        let owner_a = "OwnerA111111111111111111111111111111111111";
        let owner_b = "OwnerB111111111111111111111111111111111111";

        let jwt_a = issue_jwt(&st, owner_a).expect("issue first JWT");
        cache_minted_token(&jwt_a, owner_a);
        let first = read_token_from(&path).unwrap().unwrap();
        assert_eq!(first.jwt, jwt_a);
        assert_eq!(first.sub, owner_a);
        // expires_at is derived from the JWT's own `exp` claim — assert it
        // parses and is within ~`jwt_ttl_secs()` of now (the exp claim is
        // `iat + jwt_ttl_secs()`, where iat is the JWT mint timestamp).
        let parsed = chrono::DateTime::parse_from_rfc3339(&first.expires_at)
            .expect("cached expires_at must be RFC3339");
        let delta = parsed.with_timezone(&chrono::Utc) - chrono::Utc::now();
        assert!(
            delta.num_seconds() > 0 && delta.num_seconds() <= jwt_ttl_secs() as i64,
            "expires_at must be in the future, within jwt_ttl_secs(): delta={}s",
            delta.num_seconds()
        );

        let jwt_b = issue_jwt(&st, owner_b).expect("issue second JWT");
        cache_minted_token(&jwt_b, owner_b);
        let second = read_token_from(&path).unwrap().unwrap();
        assert_eq!(second.jwt, jwt_b);
        assert_eq!(second.sub, owner_b);
        // Each freshly-minted JWT must overwrite the previous cache and
        // the second exp must reflect the second mint, not the first.
        assert_ne!(
            first.jwt, second.jwt,
            "second JWT must differ from first (jti claim is fresh per mint)"
        );
    });
}
