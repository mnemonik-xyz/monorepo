//! HTTP handlers for the chat and download-knowledge endpoints.
//!
//! POST /chat     -- RAG-augmented chat via Ollama (rate-limited).
//! GET  /download-knowledge -- serve the pre-built knowledge artifact .zip.

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::mcp::McpState;
use crate::tools;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: Option<String>,
    /// Optional session identifier (reserved for future use).
    #[allow(dead_code)]
    pub session_id: Option<String>,
}

#[derive(Serialize)]
struct ChatSuccess {
    response: String,
}

#[derive(Serialize)]
struct ChatError {
    error: String,
    code: String,
}

// ── Constants ────────────────────────────────────────────────────────────────

const MAX_MESSAGE_LEN: usize = 2000;
const RECALL_LIMIT: usize = 3;

// ── POST /chat ───────────────────────────────────────────────────────────────

pub async fn chat_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<McpState>>,
    Json(body): Json<ChatRequest>,
) -> Response {
    // -- Rate limiting: 10 req/min per IP --
    let ip_key = addr.ip().to_string();
    if state.chat_limiter.check_key(&ip_key).is_err() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ChatError {
                error: "Rate limit exceeded".into(),
                code: "rate_limited".into(),
            }),
        )
            .into_response();
    }

    // -- Validate input --
    let message = match body.message {
        Some(ref m) if !m.trim().is_empty() => m.trim(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ChatError {
                    error: "message is required and must not be empty".into(),
                    code: "invalid_input".into(),
                }),
            )
                .into_response();
        }
    };

    if message.chars().count() > MAX_MESSAGE_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(ChatError {
                error: format!("message exceeds maximum length of {MAX_MESSAGE_LEN} characters"),
                code: "invalid_input".into(),
            }),
        )
            .into_response();
    }

    // -- Recall top-k knowledge chunks (sync: lock, query, drop) --
    // /chat is a public RAG endpoint over the server-seeded whitepaper
    // chunks (Decision 9 ownership filter scopes search by owner_pubkey).
    // Use the local server keypair so /chat returns the seeded knowledge
    // base regardless of the caller's auth state — chat is not personalized.
    let chat_owner_pubkey = solana_sdk::signer::Signer::pubkey(&state.keypair).to_string();
    let recall_result = {
        let store = state.store.lock().unwrap();
        tools::recall(
            &state.keypair,
            &store,
            state.embedder.as_ref(),
            message,
            RECALL_LIMIT,
            &chat_owner_pubkey,
        )
    }; // lock dropped here -- safe to .await below

    // Extract context from recall results
    let context = build_context(&recall_result);

    // -- Build prompt --
    let system_prompt = concat!(
        "You are a Mnemonic Protocol expert assistant. ",
        "Answer questions using ONLY the provided context. ",
        "If the context does not contain the answer, say so honestly. ",
        "Do not make up information.",
    );

    let user_message = format!(
        "--- Context ---\n\
         {context}\n\
         --- End Context ---\n\n\
         [USER_QUERY]{message}[/USER_QUERY]"
    );

    // -- Call LLM provider (shared client with redirect Policy::none() -- Decision 8) --
    let answer = match state
        .llm_client
        .chat(&state.ollama_client, system_prompt, &user_message)
        .await
    {
        Ok(text) => text,
        Err(e) => {
            tracing::error!("LLM request failed: {e}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ChatError {
                    error: "inference service unavailable".into(),
                    code: "service_unavailable".into(),
                }),
            )
                .into_response();
        }
    };

    (StatusCode::OK, Json(ChatSuccess { response: answer })).into_response()
}

/// Build a context string from recall results for the prompt.
fn build_context(recall: &serde_json::Value) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(results) = recall["results"].as_array() {
        for (i, r) in results.iter().enumerate() {
            let content = r["content"].as_str().unwrap_or("");
            if !content.is_empty() {
                parts.push(format!("[Chunk {}]\n{}", i + 1, content));
            }
        }
    }

    if parts.is_empty() {
        "No relevant context found.".to_string()
    } else {
        parts.join("\n\n")
    }
}

// ── GET /download-knowledge ──────────────────────────────────────────────────

pub async fn download_knowledge_handler(State(state): State<Arc<McpState>>) -> Response {
    let zip_path = {
        let guard = state.artifact_zip_path.lock().unwrap();
        guard.clone()
    };

    let path = match zip_path {
        Some(p) if p.exists() => p,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(ChatError {
                    error: "knowledge artifact not available".into(),
                    code: "not_found".into(),
                }),
            )
                .into_response();
        }
    };

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to read artifact zip: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatError {
                    error: "failed to read knowledge artifact".into(),
                    code: "internal_error".into(),
                }),
            )
                .into_response();
        }
    };

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("protocol-knowledge.zip");

    (
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/zip".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_context_with_results() {
        let recall = serde_json::json!({
            "results": [
                {"content": "Chunk A text", "score": 0.9},
                {"content": "Chunk B text", "score": 0.8},
            ]
        });
        let ctx = build_context(&recall);
        assert!(ctx.contains("[Chunk 1]"));
        assert!(ctx.contains("Chunk A text"));
        assert!(ctx.contains("[Chunk 2]"));
        assert!(ctx.contains("Chunk B text"));
    }

    #[test]
    fn build_context_empty_results() {
        let recall = serde_json::json!({"results": []});
        let ctx = build_context(&recall);
        assert_eq!(ctx, "No relevant context found.");
    }

    #[test]
    fn build_context_missing_results_key() {
        let recall = serde_json::json!({});
        let ctx = build_context(&recall);
        assert_eq!(ctx, "No relevant context found.");
    }

    #[test]
    fn build_context_skips_empty_content() {
        let recall = serde_json::json!({
            "results": [
                {"content": "", "score": 0.9},
                {"content": "Real content", "score": 0.8},
            ]
        });
        let ctx = build_context(&recall);
        assert!(!ctx.contains("[Chunk 1]"));
        assert!(ctx.contains("[Chunk 2]"));
        assert!(ctx.contains("Real content"));
    }
}

/// Handler-level tests using tower::ServiceExt with httpmock for Ollama.
#[cfg(test)]
mod handler_tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Router,
    };
    use http_body_util::BodyExt;
    use httpmock::MockServer;
    use mnemonic_core::arweave::ArweaveClient;
    use mnemonic_core::compress::EmbeddingCompressor;
    use mnemonic_core::embed::Embedder;
    use mnemonic_core::solana::SolanaClient;
    use mnemonic_core::storage::SqliteStore;
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// Minimal embedder for handler tests. Returns zero vectors.
    struct StubEmbedder;
    impl Embedder for StubEmbedder {
        fn embed(&self, _text: &str) -> Vec<f32> {
            vec![0.0; 8]
        }
        fn dim(&self) -> usize {
            8
        }
        fn provider_name(&self) -> &str {
            "stub"
        }
        fn model_id(&self) -> &str {
            "stub-zero"
        }
    }

    /// Build a test McpState pointing at the given LLM base URL.
    fn build_test_state(llm_base_url: &str) -> Arc<McpState> {
        use governor::Quota;
        use std::num::NonZeroU32;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = SqliteStore::open(tmp.path()).unwrap();
        let compressor = EmbeddingCompressor::new(8, 4, 42);
        let quota = Quota::per_minute(NonZeroU32::new(10).unwrap());
        let chat_limiter = governor::RateLimiter::keyed(quota);
        let ollama_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        // Use ollama provider with custom URL override so no API key is needed
        let llm_client =
            crate::llm::LlmClient::new("ollama", "", "test-model", llm_base_url, 512).unwrap();

        Arc::new(McpState {
            keypair: solana_sdk::signature::Keypair::new(),
            solana: SolanaClient::new("http://localhost:0"),
            arweave: ArweaveClient::new("http://localhost:0"),
            store: std::sync::Mutex::new(store),
            embedder: Box::new(StubEmbedder),
            compressor,
            payment_mode: "none".into(),
            treasury_pubkey: String::new(),
            usdc_mint: String::new(),
            sign_memory_cost_micro_usdc: 0,
            pricing: crate::pricing::PricingEngine::new(0),
            sol_tx_fee_lamports: 0,
            storage_mode: "local".into(),
            ollama_url: llm_base_url.to_string(),
            ollama_model: "test-model".into(),
            rag_chunk_dir: PathBuf::from("/tmp"),
            llm_client,
            artifact_zip_path: std::sync::Mutex::new(None),
            ollama_client,
            chat_limiter,
        })
    }

    /// Build an axum app with /chat and /download-knowledge routes for testing.
    fn build_app(state: Arc<McpState>) -> Router {
        Router::new()
            .route("/chat", post(chat_handler))
            .route("/download-knowledge", get(download_knowledge_handler))
            .with_state(state)
    }

    /// Send a POST /chat request with JSON body, returning (status, body bytes).
    async fn post_chat(app: Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/chat")
            .header("content-type", "application/json")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999))))
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, json)
    }

    // -- Input validation tests --

    #[tokio::test]
    async fn chat_empty_message_returns_400() {
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({"message": ""})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
    }

    #[tokio::test]
    async fn chat_missing_message_field_returns_400() {
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
    }

    #[tokio::test]
    async fn chat_whitespace_only_message_returns_400() {
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({"message": "   "})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
    }

    #[tokio::test]
    async fn chat_message_over_2000_chars_returns_400() {
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);
        let long_msg: String = "a".repeat(2001);
        let (status, body) = post_chat(app, serde_json::json!({"message": long_msg})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
        assert!(body["error"].as_str().unwrap().contains("2000"));
    }

    #[tokio::test]
    async fn chat_message_exactly_2000_chars_passes_validation() {
        // This should pass validation and reach Ollama (which will fail since no mock).
        // We check that we do NOT get a 400 -- any other status is fine (503 from Ollama being down).
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);
        let exact_msg: String = "a".repeat(2000);
        let (status, _body) = post_chat(app, serde_json::json!({"message": exact_msg})).await;
        assert_ne!(
            status,
            StatusCode::BAD_REQUEST,
            "2000-char message should pass validation"
        );
    }

    // -- Ollama error handling tests --

    #[tokio::test]
    async fn chat_llm_error_returns_503() {
        let mock_server = MockServer::start();
        mock_server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/chat/completions");
            then.status(500)
                .header("content-type", "application/json")
                .body(r#"{"error":"internal"}"#);
        });

        let state = build_test_state(&mock_server.base_url());
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({"message": "hello"})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "service_unavailable");
    }

    #[tokio::test]
    async fn chat_llm_timeout_returns_503() {
        // Point to a non-listening port to simulate timeout/connection refused.
        let state = build_test_state("http://127.0.0.1:1");
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({"message": "hello"})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "service_unavailable");
    }

    #[tokio::test]
    async fn chat_llm_success_returns_200() {
        let mock_server = MockServer::start();
        mock_server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/chat/completions");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"choices":[{"message":{"content":"Test answer from LLM"}}]}"#);
        });

        let state = build_test_state(&mock_server.base_url());
        let app = build_app(state);
        let (status, body) = post_chat(app, serde_json::json!({"message": "hello"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["response"], "Test answer from LLM");
    }

    // -- Download handler tests --

    #[tokio::test]
    async fn download_artifact_missing_returns_404() {
        let state = build_test_state("http://localhost:0");
        let app = build_app(state);

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/download-knowledge")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn download_artifact_exists_returns_200_zip() {
        let state = build_test_state("http://localhost:0");
        // Create a temp file to act as the zip artifact.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"PK-fake-zip-content").unwrap();
        {
            let mut guard = state.artifact_zip_path.lock().unwrap();
            *guard = Some(tmp.path().to_path_buf());
        }

        let app = build_app(state);

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/download-knowledge")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(content_type, "application/zip");

        let disposition = resp
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(disposition.contains("attachment"));

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], b"PK-fake-zip-content");
    }
}
