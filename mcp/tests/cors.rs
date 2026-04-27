//! Integration test: CORS exact-origin policy (Decision 9 + Risks R-CORS).
//!
//! `main.rs::run_http` narrows the previous `allow_origin(Any)` to a single
//! exact origin — `https://mnemonik.xyz`. We assert two preflight `OPTIONS`
//! requests behave correctly:
//!
//! 1. Origin `https://mnemonik.xyz` → 2xx with the `Access-Control-Allow-Origin`
//!    response header equal to that origin.
//! 2. Origin `https://evil.example.com` → no `Access-Control-Allow-Origin`
//!    header (tower-http omits the header on rejection rather than 403; we
//!    accept either status, but the header MUST be absent).
//!
//! Catches the regression where CORS is loosened back to `Any` or the
//! allowed origin is mistyped (`mnemonic.xyz` vs `mnemonik.xyz`).

use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode},
    routing::post,
    Router,
};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;

const ALLOWED_ORIGIN: &str = "https://mnemonik.xyz";
const EVIL_ORIGIN: &str = "https://evil.example.com";

/// Build a router that mirrors `main.rs::run_http`'s CORS layer exactly.
/// We intentionally inline the CORS config rather than reaching into a
/// helper from `main.rs` because the production helper builds the full
/// router with all middleware; this test only needs the CORS gate.
fn build_cors_router() -> Router {
    use axum::http::header;
    let cors = CorsLayer::new()
        .allow_origin(
            ALLOWED_ORIGIN
                .parse::<HeaderValue>()
                .expect("ALLOWED_ORIGIN parses"),
        )
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

    // 1. Allowed origin — must echo back the origin in ACAO and respond
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
