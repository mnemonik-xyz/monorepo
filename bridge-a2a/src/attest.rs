//! Core attestation logic decoupled from `AppState`.
//!
//! These functions take explicit store/keypair/lineage/idempotency parameters
//! so they are testable against any `A2aStore` implementation (including the
//! in-memory test fake).

use serde_json::Value;
use solana_sdk::signature::Keypair;
use tracing::{debug, error, warn};

use mnemonic_a2a::{
    attest_artifact, attest_message, attest_task, A2aArtifact, A2aStore, Message, Task,
};
use mnemonic_core::codec::a2a::task::TaskArtifact;
use mnemonic_core::storage::traits::AttestationStore;

use crate::config::FailureMode;
use crate::idem::{IdempotencyCache, IdempotencyKey};
use crate::lineage::LineageMap;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Inject `extensions.x-mnemonic` into a JSON-RPC response body.
pub fn inject_extension(body: &mut Value, attestation_id: &str, content_hash: &str) {
    let target = if body.get("result").is_some() {
        body.get_mut("result").unwrap()
    } else {
        body
    };

    if let Some(obj) = target.as_object_mut() {
        let exts = obj
            .entry("extensions")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(exts_obj) = exts.as_object_mut() {
            exts_obj.insert(
                "x-mnemonic".to_string(),
                serde_json::json!({
                    "attestation_id": attestation_id,
                    "blake3": content_hash,
                }),
            );
        }
    }
}

/// Extract `params.message` as a [`Message`] from a JSON-RPC `message/send`
/// request body.
pub fn extract_message(body: &Value) -> Option<Message> {
    let params = body.get("params")?;
    let msg_val = params.get("message")?;
    serde_json::from_value(msg_val.clone()).ok()
}

/// Extract the top-level `Task` from a JSON-RPC result.
pub fn extract_task_from_result(body: &Value) -> Option<Task> {
    let result = body.get("result")?;
    serde_json::from_value(result.clone()).ok()
}

/// Convert `TaskArtifact` list from a task into `A2aArtifact` list.
pub fn extract_artifacts_from_task(task: &Task) -> Vec<A2aArtifact> {
    task.artifacts
        .as_ref()
        .map(|arts| {
            arts.iter()
                .map(|ta: &TaskArtifact| A2aArtifact {
                    artifact_id: ta.artifact_id.clone(),
                    name: ta.name.clone(),
                    parts: ta.parts.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the RPC method string from a JSON-RPC request body.
pub fn rpc_method(body: &Value) -> &str {
    body.get("method").and_then(|v| v.as_str()).unwrap_or("")
}

/// Returns the `taskId` from request params.
pub fn extract_task_id(body: &Value) -> Option<String> {
    body.get("params")
        .and_then(|p| p.get("taskId").or_else(|| p.get("id")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Returns the `messageId` from request params or nested message.
pub fn extract_message_id(body: &Value) -> Option<String> {
    body.get("params")
        .and_then(|p| {
            p.get("messageId")
                .or_else(|| p.get("message").and_then(|m| m.get("messageId")))
        })
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Look up the content_hash for the most recent attestation in a context.
pub fn blake3_for<S: AttestationStore>(store: &S, ctx_id: &str) -> String {
    mnemonic_a2a::recall_by_context(store, ctx_id, Some(1))
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .map(|row| row.content_hash)
        .unwrap_or_default()
}

// ── attestation entry points ───────────────────────────────────────────────────

/// Attest a message/send request and inject the attestation id into
/// `response_body`.
///
/// # Parameters
/// - `store` — any `A2aStore` implementation.
/// - `keypair` — signing keypair.
/// - `idem` — idempotency cache.
/// - `lineage` — lineage map.
/// - `failure_mode` — what to do on attestation failure.
/// - `req_body` — parsed JSON-RPC request.
/// - `response_body` — parsed JSON-RPC response (mutated in-place).
pub fn attest_message_send<S: A2aStore>(
    store: &S,
    keypair: &Keypair,
    idem: &IdempotencyCache,
    lineage: &LineageMap,
    failure_mode: &FailureMode,
    req_body: &Value,
    response_body: &mut Value,
) -> Result<(), String> {
    let msg = match extract_message(req_body) {
        Some(m) => m,
        None => {
            debug!("message/send: no parseable message in params; skipping attestation");
            return Ok(());
        }
    };

    let method = rpc_method(req_body).to_string();
    let task_id = extract_task_id(req_body);
    let message_id = Some(msg.message_id.clone());
    let idem_key = IdempotencyKey {
        method: method.clone(),
        task_id: task_id.clone(),
        message_id: message_id.clone(),
    };

    // Idempotency: return cached result for retries.
    if let Some(cached_id) = idem.check(&idem_key) {
        debug!(
            "message/send: idempotency hit for message_id={:?}",
            message_id
        );
        let ctx_id = msg
            .context_id
            .as_deref()
            .unwrap_or(&msg.message_id)
            .to_string();
        let blake3 = blake3_for(store, &ctx_id);
        inject_extension(response_body, &cached_id, &blake3);
        return Ok(());
    }

    let ctx_id = msg
        .context_id
        .clone()
        .unwrap_or_else(|| format!("msg:{}", msg.message_id));

    let attest_result = attest_message(store, &msg, keypair, None);

    let att_id = match attest_result {
        Ok(id) => id,
        Err(e) => {
            if *failure_mode == FailureMode::AttestStrict {
                return Err(format!("attest-strict: message attestation failed: {e}"));
            }
            error!("message attestation failed (best-effort): {e}");
            return Ok(());
        }
    };

    lineage.advance(&ctx_id, &att_id);

    // Also attest the resulting task if present.
    if let Some(task) = extract_task_from_result(response_body) {
        let task_ctx = task.context_id.clone();
        let task_attest = attest_task(store, &task, keypair, Some(&att_id));
        match task_attest {
            Ok(task_att_id) => {
                lineage.advance(&task_ctx, &task_att_id);
                for artifact in extract_artifacts_from_task(&task) {
                    if let Err(e) =
                        attest_artifact(store, &artifact, &task_ctx, keypair, Some(&task_att_id))
                    {
                        if *failure_mode == FailureMode::AttestStrict {
                            return Err(format!("attest-strict: artifact attestation failed: {e}"));
                        }
                        warn!("artifact attestation failed (best-effort): {e}");
                    }
                }
            }
            Err(e) => {
                if *failure_mode == FailureMode::AttestStrict {
                    return Err(format!("attest-strict: task attestation failed: {e}"));
                }
                warn!("task attestation failed (best-effort): {e}");
            }
        }
    }

    let blake3 = blake3_for(store, &ctx_id);
    inject_extension(response_body, &att_id, &blake3);
    idem.mark(idem_key, att_id);
    Ok(())
}

/// Attest a tasks/get response and inject the attestation id.
pub fn attest_tasks_get<S: A2aStore>(
    store: &S,
    keypair: &Keypair,
    idem: &IdempotencyCache,
    lineage: &LineageMap,
    failure_mode: &FailureMode,
    req_body: &Value,
    response_body: &mut Value,
) -> Result<(), String> {
    let task = match extract_task_from_result(response_body) {
        Some(t) => t,
        None => {
            debug!("tasks/get: no parseable task in result; skipping attestation");
            return Ok(());
        }
    };

    // Only attest terminal tasks.
    if task.status != "completed" && task.status != "failed" {
        debug!(
            "tasks/get: task status={} is not terminal; skipping",
            task.status
        );
        return Ok(());
    }

    let method = rpc_method(req_body).to_string();
    let idem_key = IdempotencyKey {
        method: format!("{}:{}", method, task.status),
        task_id: Some(task.id.clone()),
        message_id: None,
    };

    if let Some(cached_id) = idem.check(&idem_key) {
        debug!("tasks/get: idempotency hit for task_id={}", task.id);
        let blake3 = blake3_for(store, &task.context_id);
        inject_extension(response_body, &cached_id, &blake3);
        return Ok(());
    }

    let ctx_id = task.context_id.clone();
    let prev_id = lineage.get(&ctx_id);

    let attest_result = attest_task(store, &task, keypair, prev_id.as_deref());

    let att_id = match attest_result {
        Ok(id) => id,
        Err(e) => {
            if *failure_mode == FailureMode::AttestStrict {
                return Err(format!("attest-strict: task attestation failed: {e}"));
            }
            error!("task attestation failed (best-effort): {e}");
            return Ok(());
        }
    };

    lineage.advance(&ctx_id, &att_id);

    for artifact in extract_artifacts_from_task(&task) {
        if let Err(e) = attest_artifact(store, &artifact, &ctx_id, keypair, Some(&att_id)) {
            if *failure_mode == FailureMode::AttestStrict {
                return Err(format!("attest-strict: artifact attestation failed: {e}"));
            }
            warn!("artifact attestation failed (best-effort): {e}");
        }
    }

    let blake3 = blake3_for(store, &ctx_id);
    inject_extension(response_body, &att_id, &blake3);
    idem.mark(idem_key, att_id);
    Ok(())
}
