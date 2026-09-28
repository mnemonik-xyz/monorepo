//! Axum proxy handler: forward requests to upstream, attest, inject extension.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::Value;
use tracing::error;

use crate::{
    attest::{attest_message_send, attest_tasks_get, rpc_method},
    state::AppState,
};

/// Generic proxy handler.
///
/// Reads the full request body, determines the A2A method, forwards to the
/// upstream server, reads the response, attests if applicable, and returns
/// the (possibly mutated) response.
pub async fn proxy_handler(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Response<Body>, StatusCode> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    // Buffer the request body.
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            error!("failed to read request body: {e}");
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    let req_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let method_str = rpc_method(&req_json).to_string();

    // Build the upstream URL.
    let upstream_url = format!(
        "{}{}",
        state.upstream.trim_end_matches('/'),
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
    );

    // Forward to upstream.
    let mut upstream_req = state.client.request(method.clone(), &upstream_url);
    for (name, value) in headers.iter() {
        if name == axum::http::header::HOST {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
    }
    upstream_req = upstream_req.body(body_bytes.to_vec());

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(e) => {
            error!("upstream request failed: {e}");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    let upstream_status = upstream_resp.status();
    let upstream_headers = upstream_resp.headers().clone();

    let resp_bytes = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            error!("failed to read upstream response: {e}");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // Parse response JSON; pass through non-JSON as-is.
    let mut resp_json: Value = match serde_json::from_slice(&resp_bytes) {
        Ok(v) => v,
        Err(_) => {
            let mut resp = Response::new(Body::from(resp_bytes.to_vec()));
            *resp.status_mut() = upstream_status;
            for (name, value) in upstream_headers.iter() {
                resp.headers_mut().insert(name, value.clone());
            }
            return Ok(resp);
        }
    };

    // Skip attestation on JSON-RPC error responses.
    if resp_json.get("error").is_none() {
        // Lock the store for the duration of the synchronous attestation call.
        // The lock is NOT held across any await point — it is released before
        // the response is returned.
        let attest_result = {
            let store_guard = state.store.lock().expect("store mutex poisoned");
            match method_str.as_str() {
                "message/send" => attest_message_send(
                    &*store_guard,
                    state.keypair.as_ref(),
                    state.idem.as_ref(),
                    state.lineage.as_ref(),
                    &state.failure_mode,
                    &req_json,
                    &mut resp_json,
                ),
                "tasks/get" => attest_tasks_get(
                    &*store_guard,
                    state.keypair.as_ref(),
                    state.idem.as_ref(),
                    state.lineage.as_ref(),
                    &state.failure_mode,
                    &req_json,
                    &mut resp_json,
                ),
                _ => Ok(()),
            }
            // `store_guard` is dropped here — before any await.
        };

        if let Err(e) = attest_result {
            // Strict mode: return a JSON-RPC error.
            let err_resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": req_json.get("id"),
                "error": {
                    "code": -32099,
                    "message": e,
                }
            });
            let body = serde_json::to_vec(&err_resp).unwrap_or_default();
            let mut resp = Response::new(Body::from(body));
            *resp.status_mut() = axum::http::StatusCode::OK;
            resp.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
            return Ok(resp);
        }
    }

    // Reserialize.
    let out_bytes = match serde_json::to_vec(&resp_json) {
        Ok(b) => b,
        Err(e) => {
            error!("failed to re-serialize response: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut resp = Response::new(Body::from(out_bytes));
    *resp.status_mut() = upstream_status;
    for (name, value) in upstream_headers.iter() {
        if name == axum::http::header::CONTENT_LENGTH {
            continue;
        }
        resp.headers_mut().insert(name, value.clone());
    }
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    Ok(resp)
}
