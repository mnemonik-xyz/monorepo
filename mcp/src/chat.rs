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
    let recall_result = {
        let store = state.store.lock().unwrap();
        tools::recall(
            &state.keypair,
            &store,
            state.embedder.as_ref(),
            message,
            RECALL_LIMIT,
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

    let full_prompt = format!(
        "{system_prompt}\n\n\
         --- Context ---\n\
         {context}\n\
         --- End Context ---\n\n\
         [USER_QUERY]{message}[/USER_QUERY]"
    );

    // -- Call Ollama (Decision 8: redirect Policy::none()) --
    let ollama_url = format!("{}/api/generate", state.ollama_url.trim_end_matches('/'));

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to build reqwest client: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatError {
                    error: "internal error".into(),
                    code: "internal_error".into(),
                }),
            )
                .into_response();
        }
    };

    let ollama_body = serde_json::json!({
        "model": state.ollama_model,
        "prompt": full_prompt,
        "stream": false,
    });

    let resp = match client.post(&ollama_url).json(&ollama_body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Ollama request failed: {e}");
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

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        tracing::warn!("Ollama returned status {status}");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ChatError {
                error: format!("inference service returned status {status}"),
                code: "service_unavailable".into(),
            }),
        )
            .into_response();
    }

    let ollama_json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("failed to parse Ollama response: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatError {
                    error: "failed to parse inference response".into(),
                    code: "internal_error".into(),
                }),
            )
                .into_response();
        }
    };

    let answer = ollama_json["response"]
        .as_str()
        .unwrap_or("")
        .to_string();

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

pub async fn download_knowledge_handler(
    State(state): State<Arc<McpState>>,
) -> Response {
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

    #[test]
    fn max_message_len_constant() {
        assert_eq!(MAX_MESSAGE_LEN, 2000);
    }

    #[test]
    fn recall_limit_constant() {
        assert_eq!(RECALL_LIMIT, 3);
    }
}
