//! Integration test: CORS allow-origin policy.
//!
//! Two regimes:
//!
//! 1. **Legacy narrow policy (Decision 9)** — exact-match `mnemonik.xyz`.
//!    Kept as a sanity test that the predicate accepts the first-party
//!    webapp identically to the previous static configuration.
//!
//! 2. **Hotfix predicate (post-T15)** — `AllowOrigin::predicate` driven by
//!    [`mnemonic_mcp::cors_policy::is_allowed_cors_origin`]. Anthropic /
//!    Cursor / ChatGPT connectors run inside the browser and originate
//!    requests from their own first-party domains (`https://claude.ai`,
//!    `https://cursor.sh`, …). The previous policy returned
//!    `Access-Control-Allow-Origin: https://mnemonik.xyz` for ALL preflights,
//!    which browsers reject. The predicate echoes back the request origin
//!    when it matches a trusted MCP-client root.
//!
//! Catches:
//!   - regression where someone re-narrows CORS to a single origin and
//!     breaks claude.ai / cursor.sh connector reachability;
//!   - lookalike-domain mistakes (`evilclaude.ai` must not pass);
//!   - subdomain handling regressions for `*.claude.ai` etc.

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::post,
    Router,
};
use mnemonic_mcp::cors_policy::is_allowed_cors_origin;
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};

const ALLOWED_ORIGIN: &str = "https://mnemonik.xyz";
const CLAUDE_AI_ORIGIN: &str = "https://claude.ai";
const CLAUDE_AI_SUBDOMAIN_ORIGIN: &str = "https://console.claude.ai";
const CURSOR_ORIGIN: &str = "https://cursor.sh";
const CHATGPT_ORIGIN: &str = "https://chatgpt.com";
const PAYWALL_ORIGIN: &str = "https://mnemonik-dev.github.io";
const EVIL_ORIGIN: &str = "https://evil.example.com";
const LOOKALIKE_ORIGIN: &str = "https://evilclaude.ai";

/// Build a router that mirrors `main.rs::run_http`'s CORS layer exactly.
/// Predicate-driven: echoes back the request origin only if it passes
/// `is_allowed_cors_origin`.
fn build_cors_router() -> Router {
    use axum::http::header;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            is_allowed_cors_origin(origin.as_bytes())
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    Router::new()
        .route("/mcp", post(|| async { "ok" }))
        .layer(cors)
}

async fn preflight(app: &Router, origin: &str) -> (StatusCode, Option<String>) {
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/mcp")
        .header("origin", origin)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let allow_origin = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    (status, allow_origin)
}

#[tokio::test]
async fn test_preflight_allows_mnemonik_xyz_rejects_evil_example_com() {
    let app = build_cors_router();

    // 1. First-party webapp — must echo back the origin in ACAO and respond
    //    with 200 or 204 (axum's CORS layer can do either depending on
    //    middleware composition; both are CORS-spec-compliant).
    let (s1, allow1) = preflight(&app, ALLOWED_ORIGIN).await;
    assert!(
        s1 == StatusCode::OK || s1 == StatusCode::NO_CONTENT,
        "preflight from allowed origin should be 2xx, got {s1}"
    );
    assert_eq!(
        allow1.as_deref(),
        Some(ALLOWED_ORIGIN),
        "Access-Control-Allow-Origin must echo allowed origin"
    );

    // 2. Disallowed origin — `tower-http` omits the ACAO header rather than
    //    returning 403. Asserting only header absence keeps the test stable
    //    across tower-http minor versions.
    let (_s2, allow2) = preflight(&app, EVIL_ORIGIN).await;
    assert!(
        allow2.is_none() || allow2.as_deref() != Some(EVIL_ORIGIN),
        "evil origin must NOT receive an ACAO header echoing itself; got {allow2:?}"
    );
}

#[tokio::test]
async fn test_preflight_allows_claude_ai_and_subdomain() {
    let app = build_cors_router();

    // claude.ai apex — Anthropic's MCP connector probes from this origin.
    let (status, allow) = preflight(&app, CLAUDE_AI_ORIGIN).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NO_CONTENT,
        "claude.ai preflight should be 2xx, got {status}"
    );
    assert_eq!(
        allow.as_deref(),
        Some(CLAUDE_AI_ORIGIN),
        "ACAO must echo claude.ai (NOT mnemonik.xyz) — connector requirement"
    );

    // *.claude.ai — wildcard subdomain support via predicate.
    let (status2, allow2) = preflight(&app, CLAUDE_AI_SUBDOMAIN_ORIGIN).await;
    assert!(status2 == StatusCode::OK || status2 == StatusCode::NO_CONTENT);
    assert_eq!(
        allow2.as_deref(),
        Some(CLAUDE_AI_SUBDOMAIN_ORIGIN),
        "console.claude.ai must be echoed back when present"
    );
}

#[tokio::test]
async fn test_preflight_allows_cursor_and_chatgpt() {
    let app = build_cors_router();

    let (s_cursor, allow_cursor) = preflight(&app, CURSOR_ORIGIN).await;
    assert!(s_cursor == StatusCode::OK || s_cursor == StatusCode::NO_CONTENT);
    assert_eq!(allow_cursor.as_deref(), Some(CURSOR_ORIGIN));

    let (s_chatgpt, allow_chatgpt) = preflight(&app, CHATGPT_ORIGIN).await;
    assert!(s_chatgpt == StatusCode::OK || s_chatgpt == StatusCode::NO_CONTENT);
    assert_eq!(allow_chatgpt.as_deref(), Some(CHATGPT_ORIGIN));
}

#[tokio::test]
async fn test_preflight_allows_exact_hosted_paywall_origin() {
    let app = build_cors_router();
    let (status, allow) = preflight(&app, PAYWALL_ORIGIN).await;
    assert!(status == StatusCode::OK || status == StatusCode::NO_CONTENT);
    assert_eq!(allow.as_deref(), Some(PAYWALL_ORIGIN));

    let (_, unrelated) = preflight(&app, "https://evil.github.io").await;
    assert!(unrelated.is_none());
}

#[tokio::test]
async fn test_preflight_rejects_lookalike_attack() {
    // `evilclaude.ai` ends with `claude.ai` as a substring but NOT as a
    // dot-separated suffix — the leading-dot guard in `is_allowed_cors_origin`
    // rejects it. ACAO must NOT be echoed.
    let app = build_cors_router();
    let (_status, allow) = preflight(&app, LOOKALIKE_ORIGIN).await;
    assert!(
        allow.is_none() || allow.as_deref() != Some(LOOKALIKE_ORIGIN),
        "lookalike `evilclaude.ai` must be rejected; got {allow:?}"
    );
}
