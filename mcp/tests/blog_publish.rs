//! Integration tests for the webapp-rethink Task 9 publish surfaces:
//! the `mnemonic_publish_post` MCP tool and the Micropub-shaped `POST /blog`.
//!
//! Both surfaces flow through the SAME `publish::publish_post` pipeline, so the
//! critical guarantees under test are:
//!   - anonymous publish is REJECTED (401) on `POST /blog` and on the MCP tool;
//!   - an authed publish persists a public post that round-trips through the
//!     existing `GET /blog` + `GET /blog/:slug` read routes;
//!   - BOTH `application/json` and `application/x-www-form-urlencoded` (Micropub)
//!     bodies are accepted.
//!
//! Built against `test_support::mock_state()` (in-memory SQLite + StubEmbedder)
//! and the live `oauth::bearer_auth_middleware`, exercised via
//! `tower::ServiceExt::oneshot` so no socket is bound. The router mirrors the
//! production merge (public GET /blog on the base router, authed POST /blog on
//! the bearer-layered subrouter) so the GET+POST same-path coexistence is
//! exercised too.

#![cfg(feature = "test-support")]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use mnemonic_mcp::{
    api,
    mcp::{self, McpState},
    oauth::{self, OAuthState},
    test_support::mock_state,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const TEST_SECRET: &[u8; 32] = b"blog-publish-secret-32-bytes!!!!";

/// Mirror the production wiring: public GET read routes on the base router,
/// authed `POST /blog` + `/mcp` on a bearer-layered subrouter merged in.
fn build_router(state: Arc<McpState>, oauth_state: Arc<OAuthState>) -> Router {
    let authed = Router::new()
        .route("/blog", post(api::blog_publish_handler))
        .route("/mcp", post(mcp::mcp_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state,
            oauth::bearer_auth_middleware,
        ))
        .with_state(state.clone());

    Router::new()
        .route("/blog", get(api::blog_list_handler))
        .route("/blog/{slug}", get(api::blog_post_handler))
        .with_state(state)
        .merge(authed)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let trimmed = String::from_utf8_lossy(&bytes).trim().to_string();
    let parsed: Value = serde_json::from_str(&trimmed).unwrap_or(Value::String(trimmed));
    (status, headers, parsed)
}

fn post_blog_json(body: Value, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/blog")
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let (status, _h, v) = send(app, req).await;
    (status, v)
}

#[tokio::test]
async fn post_blog_anonymous_rejected() {
    let app = build_router(
        mock_state(),
        Arc::new(OAuthState::with_defaults(TEST_SECRET)),
    );
    let req = post_blog_json(json!({"title": "Nope", "body_markdown": "x"}), None);
    let (status, _h, _v) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anonymous publish must 401"
    );
}

#[tokio::test]
async fn post_blog_json_authed_persists_and_roundtrips() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, "AgentPubkeyBase58").expect("issue_jwt");

    let req = post_blog_json(
        json!({
            "title": "Hello, World!",
            "body_markdown": "# Hi\n\nFirst post body.",
            "tags": ["intro", "demo"],
            "author": "Agent Smith"
        }),
        Some(&token),
    );
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "authed publish must 201: {body}"
    );
    assert_eq!(
        headers.get("location").and_then(|v| v.to_str().ok()),
        Some("/blog/hello-world"),
        "Location must point at the created slug"
    );
    let post = &body["post"];
    assert_eq!(post["slug"], "hello-world");
    assert_eq!(post["title"], "Hello, World!");
    assert_eq!(post["author"], "Agent Smith");
    assert!(
        post["content_hash"].as_str().map(|s| s.len()) == Some(64),
        "content_hash must be a 64-char blake3 hex: {post}"
    );

    // Round-trips through the public read routes.
    let (s1, list) = get_json(&app, "/blog").await;
    assert_eq!(s1, StatusCode::OK);
    let slugs: Vec<&str> = list["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["slug"].as_str())
        .collect();
    assert!(
        slugs.contains(&"hello-world"),
        "GET /blog must list the post: {list}"
    );

    let (s2, detail) = get_json(&app, "/blog/hello-world").await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(detail["post"]["body_markdown"], "# Hi\n\nFirst post body.");
    assert_eq!(detail["post"]["tags"][0], "intro");
}

#[tokio::test]
async fn post_blog_form_micropub_accepted() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());
    let token = oauth::issue_jwt(&oauth_state, "FormAgent").expect("issue_jwt");

    // Micropub x-www-form-urlencoded: `name` (title), `content` (body), and a
    // repeated `category` (tags array).
    let form = "h=entry&name=Form+Post&content=Body+from+form&category=x&category=y";
    let req = Request::builder()
        .method("POST")
        .uri("/blog")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(form))
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "form publish must 201: {body}");
    assert_eq!(
        headers.get("location").and_then(|v| v.to_str().ok()),
        Some("/blog/form-post")
    );

    let (s, detail) = get_json(&app, "/blog/form-post").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(detail["post"]["body_markdown"], "Body from form");
    let tags: Vec<&str> = detail["post"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();
    assert_eq!(tags, vec!["x", "y"]);
}

async fn tools_call(app: &Router, args: Value, token: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "mnemonic_publish_post", "arguments": args}
    });
    let req = b
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let (status, _h, v) = send(app, req).await;
    (status, v)
}

#[tokio::test]
async fn mcp_tool_publish_requires_auth_then_persists() {
    let oauth_state = Arc::new(OAuthState::with_defaults(TEST_SECRET));
    let app = build_router(mock_state(), oauth_state.clone());

    // Anonymous tools/call for the (non-allowlisted) publish tool → 401.
    let (status, _v) = tools_call(&app, json!({"title": "T", "body_markdown": "b"}), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anonymous tool publish must 401"
    );

    // Authed tools/call → 200, returns the post; then visible on GET /blog.
    let token = oauth::issue_jwt(&oauth_state, "ToolAgent").expect("issue_jwt");
    let (status, v) = tools_call(
        &app,
        json!({"title": "Tool Made This", "body_markdown": "via mcp tool"}),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "authed tool publish must 200: {v}");
    // The tool result is wrapped in the MCP `content:[{text:"<json>"}]` envelope.
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .expect("result text");
    let post: Value = serde_json::from_str(text).expect("post json");
    assert_eq!(post["slug"], "tool-made-this");

    let (s, detail) = get_json(&app, "/blog/tool-made-this").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(detail["post"]["body_markdown"], "via mcp tool");
}
