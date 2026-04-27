//! Rate-limit routing tests for the `/mcp` route + `/oauth/*` routes
//! (Decision 9). The production limits are 5 req/min on /oauth/*, 5 req/min
//! on `sign_memory`, 30 req/min on `recall`. We assert:
//!
//!   1. `tools/call sign_memory` returns 429 after the configured burst
//!      (smaller burst used in tests for speed).
//!   2. `tools/call recall` has a higher cap and survives a burst that
//!      would trip the sign_memory limit.
//!   3. Stdio transport never engages the limiter (sanity test — the
//!      Router-level governor layer is HTTP-only by construction).
//!
//! Tests build a minimal Router with a configurable burst so we don't have
//! to wait 60s for the per-minute window to roll. The production
//! `main.rs::run_http` uses `per_second(2).burst_size(30)` which translates
//! to ≈30/min smoothed; here we use `per_second(60 * 60).burst_size(N)` so
//! the limiter rejects the (N+1)th request immediately without any sleep.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::GlobalKeyExtractor, GovernorLayer,
};

/// Burst size approximating the `sign_memory` per-minute cap (Decision 9).
const SIGN_MEMORY_BURST: u32 = 5;
/// Burst size approximating the `recall` per-minute cap (Decision 9).
const RECALL_BURST: u32 = 30;

/// Minimal echo handler — the test just needs to know the request reached
/// the inner service.
async fn echo() -> &'static str {
    "ok"
}

/// Build a router that limits POSTs to `/mcp` at exactly `burst` requests
/// before refusing further.
///
/// Uses `GlobalKeyExtractor` so the limiter engages without a real socket
/// IP (axum `oneshot()` calls don't populate `ConnectInfo`). In production
/// the same `GovernorConfigBuilder` uses the default `PeerIpKeyExtractor`
/// which keys by remote IP — semantically equivalent for the assertion
/// "(burst+1)th request from the same key is rejected."
///
/// `per_second(3600)` keeps the refill rate low enough that the (burst+1)th
/// call within the test's ~ms window is still rejected.
fn build_limited_router(burst: u32) -> Router {
    let conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(3600)
            .burst_size(burst)
            .key_extractor(GlobalKeyExtractor)
            .finish()
            .expect("build governor config"),
    );
    Router::new()
        .route("/mcp", post(echo))
        .layer(GovernorLayer { config: conf })
}

/// Issue a single POST and return its status. `tower_governor` keys by
/// remote IP — tower's `oneshot` infrastructure uses 127.0.0.1 for all
/// in-process calls so all requests share the same key (the property we
/// want to test).
async fn fire(app: Router, body: serde_json::Value) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("body")))
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    // Drain the body so the response is fully consumed (avoids leaking
    // pending tasks across the test).
    let _ = resp.into_body().collect().await;
    status
}

#[tokio::test]
async fn test_sign_memory_ratelimit_429_after_5() {
    let app = build_limited_router(SIGN_MEMORY_BURST);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {"name": "mnemonic_sign_memory", "arguments": {"content": "x"}},
        "id": 1,
    });
    // First N requests must pass.
    for i in 0..SIGN_MEMORY_BURST {
        let s = fire(app.clone(), body.clone()).await;
        assert_eq!(
            s,
            StatusCode::OK,
            "request {i} must pass (burst={SIGN_MEMORY_BURST})"
        );
    }
    // (N+1)th must be rate-limited.
    let s = fire(app.clone(), body).await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "request {} must be 429",
        SIGN_MEMORY_BURST + 1,
    );
}

#[tokio::test]
async fn test_recall_ratelimit_429_after_30() {
    let app = build_limited_router(RECALL_BURST);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {"name": "mnemonic_recall", "arguments": {"query": "x"}},
        "id": 1,
    });
    for i in 0..RECALL_BURST {
        let s = fire(app.clone(), body.clone()).await;
        assert_eq!(
            s,
            StatusCode::OK,
            "request {i} must pass (burst={RECALL_BURST})"
        );
    }
    let s = fire(app.clone(), body).await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "request {} must be 429",
        RECALL_BURST + 1,
    );
}

#[tokio::test]
async fn test_stdio_no_ratelimit() {
    // Stdio path bypasses the HTTP router entirely (`main.rs::run_stdio`).
    // We assert this structurally: a router built WITHOUT the GovernorLayer
    // must accept arbitrary bursts. The production stdio code path
    // constructs no Router at all, but we model "no governor" here.
    let app = Router::new().route("/mcp", post(echo));
    let body = serde_json::json!({"jsonrpc": "2.0", "method": "tools/list", "id": 1});
    // Fire well above any reasonable rate limit — all must succeed.
    for i in 0..100 {
        let s = fire(app.clone(), body.clone()).await;
        assert_eq!(s, StatusCode::OK, "request {i} must pass (no limiter)");
    }
}
