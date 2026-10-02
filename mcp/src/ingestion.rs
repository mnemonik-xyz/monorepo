//! Client-prepared artifacts: bounded validation, shared exact-byte delivery,
//! and metadata-only receipts. Legacy deferred signing is a separate migration API.
use crate::{
    delivery_operation as delivery, mcp::McpState, oauth::Claims, paid_operation, payment,
    wallet_link,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use mnemonic_core::{
    arweave::ArweaveClient,
    codec::{
        canonical::{from_canonical_cbor, to_canonical_cbor},
        hash::hash_bytes,
        schema::{validate_artifact, MEMORY_V1, SEALED_V1},
        sign::verify_artifact,
    },
    identity::LazyKeypair,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Quota refunds and durable payment eligibility must change together, including
/// cancellation and errors before upload. If metadata cannot be reset, retain
/// the reservation instead of granting a permanently free retry.
struct QuotaReservation<'a> {
    grant: Option<payment::FreeAnchorGrant<'a>>,
    store: &'a std::sync::Mutex<mnemonic_core::storage::SqliteStore>,
    operation_id: &'a str,
    lease: &'a str,
}
impl QuotaReservation<'_> {
    fn mark_chain_write(&mut self) {
        if let Some(grant) = self.grant.as_mut() {
            grant.mark_chain_write();
        }
    }
    fn keep(mut self) {
        if let Some(grant) = self.grant.take() {
            grant.keep();
        }
    }
}
impl Drop for QuotaReservation<'_> {
    fn drop(&mut self) {
        let Some(grant) = self.grant.take() else {
            return;
        };
        let safe_to_refund = self.store.lock().ok().is_some_and(|store| {
            match delivery::get(store.conn(), self.operation_id) {
                Ok(Some(op)) if op.payment_status == "required" => true,
                Ok(Some(op)) if op.payment_status == "not_required" => {
                    delivery::quota_refunded(store.conn(), self.operation_id, self.lease).is_ok()
                }
                _ => false,
            }
        });
        if !safe_to_refund {
            grant.keep();
        }
        // Otherwise grant drops after the store lock above is released.
    }
}

pub const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct ValidatedMemory {
    pub author: String,
    pub kind: String,
    pub content_hash: String,
    pub envelope_digest: String,
}

/// Validate the original bytes, never a reconstructed envelope. No side effects.
pub fn validate_memory(bytes: &[u8], owner: &str) -> anyhow::Result<ValidatedMemory> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_ARTIFACT_BYTES,
        "artifact size exceeds limit"
    );
    let v = verify_artifact(bytes, None).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        v.valid && v.cose_signature && v.algorithm_valid && v.signer == owner,
        "invalid signature or author"
    );
    let payload = from_canonical_cbor(&v.payload).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        payload["producer"] == format!("did:sol:{owner}"),
        "producer does not match author"
    );
    anyhow::ensure!(payload["schema_version"] == 1, "unsupported schema version");
    let kind = payload["type"].as_str().unwrap_or("");
    let schema = match kind {
        "memory" => &MEMORY_V1,
        "sealed" => &SEALED_V1,
        _ => anyhow::bail!("unsupported artifact kind"),
    };
    validate_artifact(&payload, schema).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        to_canonical_cbor(&payload, schema).map_err(anyhow::Error::msg)? == v.payload,
        "noncanonical artifact payload"
    );
    anyhow::ensure!(
        payload["artifact_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "artifact_id required"
    );
    if kind == "sealed" {
        anyhow::ensure!(
            payload["alg"] == "XChaCha20Poly1305",
            "unsupported sealed algorithm"
        );
        for (field, length) in [("nonce", 24), ("kc", 32)] {
            let value = base64::engine::general_purpose::STANDARD
                .decode(payload[field].as_str().unwrap_or(""))?;
            anyhow::ensure!(value.len() == length, "invalid sealed {field}");
        }
        anyhow::ensure!(
            base64::engine::general_purpose::STANDARD
                .decode(payload["ct"].as_str().unwrap_or(""))?
                .len()
                >= 16,
            "invalid ciphertext"
        );
        let wraps = payload["wraps"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("wraps required"))?;
        anyhow::ensure!(!wraps.is_empty() && wraps.len() <= 64, "invalid wraps");
        for wrap in wraps {
            for (field, length) in [("enc", 32), ("wk", 48)] {
                anyhow::ensure!(
                    base64::engine::general_purpose::STANDARD
                        .decode(wrap[field].as_str().unwrap_or(""))?
                        .len()
                        == length,
                    "invalid wrap {field}"
                );
            }
        }
    } else {
        anyhow::ensure!(
            payload["content"].is_string() && payload["visibility"] == "public",
            "unsealed hosted memory requires public visibility"
        );
    }
    Ok(ValidatedMemory {
        author: owner.into(),
        kind: kind.into(),
        content_hash: v.content_hash,
        envelope_digest: hash_bytes(bytes),
    })
}

/// Shared by signed memory and A2A. Validation MUST precede this function.
/// The configured gateway bounds reads; SQL is deliberately absent.
pub async fn deliver_exact(
    arweave: &ArweaveClient,
    operator: &LazyKeypair,
    bytes: &[u8],
    tags: &[(&str, &str)],
) -> anyhow::Result<String> {
    let kp = operator.keypair()?;
    let id = arweave.item_id(bytes, kp, tags)?;
    match arweave.read_a2a(&id).await {
        Ok(found) => anyhow::ensure!(found == bytes, "delivery bytes mismatch"),
        Err(_) => {
            let uploaded = arweave.write_item(bytes, kp, tags).await?;
            anyhow::ensure!(uploaded == id, "upload locator mismatch");
            let found = arweave.read_a2a(&id).await?;
            anyhow::ensure!(found == bytes, "delivery bytes mismatch");
        }
    }
    Ok(id)
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"status":"error","error":code}))).into_response()
}

/// Canonical transport: POST application/cbor, body = complete COSE_Sign1.
/// x-mnemonic-mode must be absent or anchored. Public memory also requires
/// x-mnemonic-public-consent: true. JWT binds expected author.
pub async fn ingest_handler(
    State(state): State<Arc<McpState>>,
    request: axum::http::Request<axum::body::Body>,
) -> Response {
    let Some(claims) = request.extensions().get::<Claims>().cloned() else {
        return error(StatusCode::UNAUTHORIZED, "authentication required");
    };
    let ip = crate::client_ip::resolve(&state.trusted_proxies, &request);
    let headers = request.headers().clone();
    if headers
        .get("x-mnemonic-mode")
        .is_some_and(|v| v != "anchored")
    {
        return error(
            StatusCode::BAD_REQUEST,
            "local mode requires agent-owned storage",
        );
    }
    if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/cbor") {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "application/cbor required",
        );
    }
    let bytes = match axum::body::to_bytes(request.into_body(), MAX_ARTIFACT_BYTES).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "artifact size exceeds limit"),
    };
    let v = match validate_memory(&bytes, &claims.sub) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if v.kind == "memory"
        && headers
            .get("x-mnemonic-public-consent")
            .is_none_or(|v| v != "true")
    {
        return error(
            StatusCode::BAD_REQUEST,
            "explicit public disclosure consent required",
        );
    }
    ingest_validated(state, claims, headers, ip, bytes, v, None).await
}

/// Internal shared coordinator. Adapters MUST verify signature, kind, author,
/// signed bindings, consent and any parent before calling this function.
pub async fn ingest_validated(
    state: Arc<McpState>,
    claims: Claims,
    headers: axum::http::HeaderMap,
    ip: Option<std::net::IpAddr>,
    bytes: axum::body::Bytes,
    v: ValidatedMemory,
    context: Option<String>,
) -> Response {
    if v.author != claims.sub
        || v.envelope_digest != hash_bytes(&bytes)
        || bytes.is_empty()
        || bytes.len() > MAX_ARTIFACT_BYTES
    {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid validated operation binding",
        );
    }
    if v.kind != "a2a" && !state.envelope.supports_anchored() {
        return error(StatusCode::BAD_REQUEST, "anchored storage unavailable");
    }
    let mut tags = vec![
        ("Mnemonic-Type", v.kind.as_str()),
        ("Producer", v.author.as_str()),
        ("Content-Hash", v.content_hash.as_str()),
    ];
    if let Some(ref context) = context {
        tags.push(("Context-Id", context.as_str()));
    }
    let existing = match state
        .keypair
        .keypair()
        .and_then(|kp| state.arweave.item_id(&bytes, kp, &tags))
    {
        Ok(id) => match state.arweave.read_a2a(&id).await {
            Ok(found) if found == bytes => Some(id),
            Ok(_) => return error(StatusCode::BAD_GATEWAY, "delivery bytes mismatch"),
            Err(_) => None,
        },
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "operator identity unavailable",
            )
        }
    };
    let digest = crate::paid_artifact::hash_client_signed_cose(&bytes);
    let default_id = format!(
        "artifact-{}",
        blake3::hash(format!("{}:{digest}", claims.sub).as_bytes()).to_hex()
    );
    let operation_id = match headers.get("x-mnemonic-operation-id") {
        Some(h) => match h.to_str() {
            Ok(v) => v.to_owned(),
            Err(_) => return error(StatusCode::BAD_REQUEST, "invalid operation ID"),
        },
        None => default_id,
    };
    let financial_record = state.store.lock().ok().and_then(|store| {
        paid_operation::get(store.conn(), &operation_id)
            .ok()
            .flatten()
    });
    if financial_record.as_ref().is_some_and(|op| {
        op.subject_hash != blake3::hash(claims.sub.as_bytes()).to_hex().to_string()
            || op.artifact_hash != digest
    }) {
        return error(StatusCode::CONFLICT, "financial operation binding conflict");
    }
    let settled_receipt = financial_record
        .as_ref()
        .map(paid_operation::settled_receipt)
        .transpose();
    let financial_valid = matches!(&settled_receipt, Ok(Some(Some(_))));
    let financial_invalid = settled_receipt.is_err();
    let deterministic = match state
        .keypair
        .keypair()
        .and_then(|kp| state.arweave.item_id(&bytes, kp, &tags))
    {
        Ok(id) => id,
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "operator identity unavailable",
            )
        }
    };
    // A verified exact-byte read is external success even when receipt SQL is
    // read-only or lost. Never accept conflicting existing operation metadata.
    if let Some(ref locator) = existing {
        let previous = state
            .store
            .lock()
            .ok()
            .and_then(|store| delivery::get(store.conn(), &operation_id).ok().flatten());
        if previous.as_ref().is_some_and(|op| {
            op.author != claims.sub
                || op.envelope_digest != digest
                || op.byte_length != bytes.len()
                || op.backend != "arweave"
                || op.locator != *locator
        }) {
            return error(StatusCode::CONFLICT, "operation binding conflict");
        }
        let payment_status = if financial_invalid {
            "unknown"
        } else if let Some(ref prior) = previous {
            if ["remedy_pending", "remedied"].contains(&prior.payment_status.as_str()) {
                prior.payment_status.as_str()
            } else if financial_valid {
                "settled"
            } else {
                prior.payment_status.as_str()
            }
        } else if financial_valid {
            "settled"
        } else {
            previous
                .as_ref()
                .map(|op| op.payment_status.as_str())
                .unwrap_or(if state.payment_mode == "none" {
                    "not_required"
                } else {
                    "unknown"
                })
        };
        let now = chrono::Utc::now().timestamp();
        let lease = uuid::Uuid::new_v4().to_string();
        let operation_persisted = previous.is_some()
            && state.store.lock().ok().is_some_and(|store| {
                if !delivery::acquire(store.conn(), &operation_id, &lease, now).unwrap_or(false) {
                    return false;
                }
                if payment_status == "settled"
                    && delivery::payment_settled(store.conn(), &operation_id, &lease, now).is_err()
                {
                    return false;
                }
                delivery::release(store.conn(), &operation_id, &lease, "verified", None, now)
                    .is_ok()
            });
        return (StatusCode::OK,Json(json!({"status":"ok","operation_id":operation_id,"delivery_status":"verified","payment_status":payment_status,"financial_reconciliation_required":payment_status=="unknown","operation_persisted":operation_persisted,"receipt_persisted":false,"persistence_error":if operation_persisted{Value::Null}else{json!("operation_receipt_write_unavailable")},"artifact_kind":v.kind,"author":v.author,"content_hash":v.content_hash,"memory_hash":v.content_hash,"envelope_digest":v.envelope_digest,"byte_length":bytes.len(),"arweave_tx":locator,"locator":format!("ar://{locator}"),"write_mode":"anchored"}))).into_response();
    }
    if financial_invalid {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "stored payment receipt requires reconciliation",
        );
    }
    let now = chrono::Utc::now().timestamp();
    let operation = match state.store.lock().ok().and_then(|store| {
        delivery::bind(
            store.conn(),
            &operation_id,
            &claims.sub,
            &digest,
            bytes.len(),
            "arweave",
            &deterministic,
            state.payment_mode != "none",
            now,
        )
        .ok()
    }) {
        Some(op) => op,
        None => {
            return error(
                StatusCode::CONFLICT,
                "operation binding or persistence conflict",
            )
        }
    };
    let lease = uuid::Uuid::new_v4().to_string();
    if !state.store.lock().ok().is_some_and(|store| {
        delivery::acquire(store.conn(), &operation_id, &lease, now).unwrap_or(false)
    }) {
        return (StatusCode::CONFLICT,Json(json!({"operation_id":operation_id,"payment_status":operation.payment_status,"delivery_status":operation.delivery_status,"retry_action":if operation.delivery_status=="failed_terminal"{"contact_operator_for_remedy"}else{"retry_same_operation_and_bytes"}}))).into_response();
    }
    let release = |status: &str, code: Option<&str>| {
        state.store.lock().ok().is_some_and(|store| {
            delivery::release(
                store.conn(),
                &operation_id,
                &lease,
                status,
                code,
                chrono::Utc::now().timestamp(),
            )
            .is_ok()
        })
    };
    let has_financial = financial_record.is_some();
    let mut free = None;
    if operation.payment_status == "required"
        && existing.is_none()
        && !has_financial
        && payment::free_quota_applies(&state.payment_mode)
    {
        match payment::claim_free_anchor(
            &state.store,
            &claims.sub,
            ip,
            bytes.len(),
            Some(&operation_id),
            state.free_anchors,
        ) {
            Ok(payment::FreeAnchorClaim::Granted(grant)) => {
                free = Some(QuotaReservation {
                    grant: Some(grant),
                    store: &state.store,
                    operation_id: &operation_id,
                    lease: &lease,
                })
            }
            Ok(payment::FreeAnchorClaim::Denied(_)) => {}
            Err(_) => {
                release("validated", Some("quota_state_unavailable"));
                return error(StatusCode::SERVICE_UNAVAILABLE, "quota state unavailable");
            }
        }
    }
    let mut payment_status = operation.payment_status.clone();
    if payment_status == "required" && financial_valid {
        if !state.store.lock().ok().is_some_and(|store| {
            delivery::payment_settled(store.conn(), &operation_id, &lease, now).is_ok()
        }) {
            release("validated", Some("payment_state_unavailable"));
            return error(StatusCode::SERVICE_UNAVAILABLE, "payment state unavailable");
        }
        payment_status = "settled".into();
    }
    if payment_status == "required" {
        if free.is_some()
            || (!has_financial && existing.is_some())
            || (state.payment_mode == "none" && !has_financial)
        {
            if !state.store.lock().ok().is_some_and(|store| {
                delivery::payment_free(store.conn(), &operation_id, &lease, now).is_ok()
            }) {
                release("validated", Some("payment_state_unavailable"));
                return error(StatusCode::SERVICE_UNAVAILABLE, "payment state unavailable");
            }
            payment_status = "not_required".into();
        } else {
            if let Err(response) =
                paid_ingestion_gate(&state, &headers, &claims.sub, &operation_id, &digest).await
            {
                release("validated", Some("payment_required"));
                return response;
            }
            if !state.store.lock().ok().is_some_and(|store| {
                delivery::payment_settled(store.conn(), &operation_id, &lease, now).is_ok()
            }) {
                release("validated", Some("payment_state_unavailable"));
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "payment settled; resubmit same operation and bytes",
                );
            }
            payment_status = "settled".into();
        }
    }
    if !state
        .store
        .lock()
        .ok()
        .is_some_and(|store| delivery::uploading(store.conn(), &operation_id, &lease, now).is_ok())
    {
        release("failed_retryable", Some("delivery_state_unavailable"));
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "delivery state unavailable",
        );
    }
    if let Some(grant) = free.as_mut() {
        grant.mark_chain_write();
    }
    let locator = match deliver_exact(&state.arweave, &state.keypair, &bytes, &tags).await {
        Ok(id) => id,
        Err(_) => {
            let terminal = operation.attempts >= 7 && payment_status == "settled";
            release(
                if terminal {
                    "failed_terminal"
                } else {
                    "failed_retryable"
                },
                Some("delivery_unavailable"),
            );
            return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"operation_id":operation_id,"delivery_status":if terminal{"failed_terminal"}else{"failed_retryable"},"payment_status":if terminal{"remedy_pending"}else{payment_status.as_str()},"error":"delivery_unavailable","retry_action":if terminal{"contact_operator_for_refund_or_credit"}else{"resubmit_same_operation_and_original_bytes"},"remedy_policy":"Settled terminal failures require operator-reviewed delivery, refund, or service credit; retries never charge again."}))).into_response();
        }
    };
    if let Some(grant) = free {
        grant.keep();
    }
    let operation_persisted = release("verified", None);
    // Adapters verified the immutable bytes before payment; delivery compared
    // fetched bytes exactly, so memory and A2A share the same evidence.
    let persisted=state.store.lock().ok().is_some_and(|store| {
        store.conn().execute_batch("CREATE TABLE IF NOT EXISTS artifact_receipts (author TEXT NOT NULL, envelope_digest TEXT NOT NULL, content_hash TEXT NOT NULL, artifact_kind TEXT NOT NULL, byte_length INTEGER NOT NULL, locator TEXT NOT NULL, verified_at TEXT NOT NULL, PRIMARY KEY(author,envelope_digest,locator));").and_then(|_|store.conn().execute("INSERT OR REPLACE INTO artifact_receipts VALUES (?1,?2,?3,?4,?5,?6,?7)",rusqlite::params![v.author,v.envelope_digest,v.content_hash,v.kind,bytes.len(),locator,chrono::Utc::now().to_rfc3339()]).map(|_|())).is_ok()
    });
    (StatusCode::OK,Json(json!({"status":"ok","delivery_status":"verified","operation_id":operation_id,"payment_status":payment_status,"operation_persisted":operation_persisted,"receipt_persisted":persisted,"persistence_error":if persisted {Value::Null}else{json!("receipt_write_failed")},"artifact_kind":v.kind,"author":v.author,"content_hash":v.content_hash,"memory_hash":v.content_hash,"envelope_digest":v.envelope_digest,"byte_length":bytes.len(),"arweave_tx":locator,"locator":format!("ar://{locator}"),"write_mode":"anchored"}))).into_response()
}

/// Only the already supported immutable Universal Paywall exact rail is accepted.
/// Linked wallet proof precedes a quote, and settlement precedes delivery.
#[allow(clippy::result_large_err)]
async fn paid_ingestion_gate(
    state: &Arc<McpState>,
    headers: &axum::http::HeaderMap,
    author: &str,
    operation_id: &str,
    digest: &str,
) -> Result<(), Response> {
    let config =
        payment::active_universal_paywall(&state.payment_mode, state.universal_paywall.as_ref())
            .ok_or_else(|| {
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "paid client-prepared ingestion unavailable; no payment accepted",
                )
            })?;
    let chain_id = config
        .network
        .strip_prefix("eip155:")
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| error(StatusCode::SERVICE_UNAVAILABLE, "invalid payment network"))?;
    let subject_hash = blake3::hash(author.as_bytes()).to_hex().to_string();
    let wallet = {
        let store = state
            .store
            .lock()
            .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "payment state unavailable"))?;
        let link = wallet_link::get_verified(store.conn(), operation_id)
            .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "wallet link unavailable"))?;
        match link {
            Some(link) if link.subject_hash == subject_hash && link.chain_id == chain_id => {
                link.wallet_address
            }
            Some(_) => {
                return Err(error(
                    StatusCode::UNAUTHORIZED,
                    "wallet link binding conflict",
                ))
            }
            None => {
                let challenge = wallet_link::create_or_get_challenge(
                    store.conn(),
                    operation_id,
                    &subject_hash,
                    chain_id,
                    chrono::Utc::now(),
                )
                .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "wallet link unavailable"))?;
                return Err((StatusCode::PRECONDITION_REQUIRED,Json(json!({"status":"awaiting_wallet_link","operation_id":operation_id,"payment_status":"required","delivery_status":"validated","wallet_link_url":format!("/approve?operation_id={operation_id}"),"challenge":challenge,"message":wallet_link::challenge_message(&challenge),"retry_action":"resubmit_same_operation_and_original_bytes"}))).into_response());
            }
        }
    };
    let client = crate::universal_paywall::UniversalPaywallClient::new(config.clone());
    match payment::check_universal_paywall(
        headers,
        &client,
        config,
        state.pricing.current_price(),
        &state.store,
        &state.universal_paywall_quotes,
        author,
        &wallet,
        digest,
        Some(operation_id),
    )
    .await
    {
        payment::PaymentGate::Proceed => Ok(()),
        payment::PaymentGate::NeedUniversalPaywall(ref required) => {
            let body = payment::up_payment_required(required, "");
            let mut response = (StatusCode::PAYMENT_REQUIRED, Json(&body)).into_response();
            if let Ok(json) = serde_json::to_vec(&body) {
                if let Ok(header) = axum::http::HeaderValue::from_str(
                    &base64::engine::general_purpose::STANDARD.encode(json),
                ) {
                    response.headers_mut().insert("payment-required", header);
                }
            }
            Err(response)
        }
        payment::PaymentGate::Unauthorized(message) => {
            Err(error(StatusCode::UNAUTHORIZED, &message))
        }
        _ => Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "unsupported payment rail",
        )),
    }
}

#[cfg(test)]
mod quota_reservation_tests {
    use super::*;
    use solana_sdk::signature::{Keypair, Signer};
    struct Fixture {
        store: std::sync::Mutex<mnemonic_core::storage::SqliteStore>,
        free_anchors: payment::FreeAnchorLimits,
    }
    fn setup() -> (Arc<Fixture>, String) {
        let store =
            mnemonic_core::storage::SqliteStore::open(std::path::Path::new(":memory:")).unwrap();
        delivery::migrate(store.conn()).unwrap();
        paid_operation::migrate_paid_operations(store.conn()).unwrap();
        payment::migrate_free_anchor_usage(store.conn()).unwrap();
        crate::oauth::google::migrate_google_identity_links(store.conn()).unwrap();
        let state = Arc::new(Fixture {
            store: std::sync::Mutex::new(store),
            free_anchors: payment::FreeAnchorLimits {
                per_account: 2,
                per_ip: 2,
                global: 10,
                max_bytes: 16384,
            },
        });
        let author = Keypair::new().pubkey().to_string();
        {
            let store = state.store.lock().unwrap();
            store.conn().execute("INSERT INTO google_identity_links (google_sub,pubkey_base58,linked_at) VALUES ('quota-guard-account',?1,strftime('%s','now'))",[&author]).unwrap();
            delivery::bind(
                store.conn(),
                "quota-op",
                &author,
                &"a".repeat(64),
                12,
                "arweave",
                "locator",
                true,
                0,
            )
            .unwrap();
            delivery::acquire(store.conn(), "quota-op", "lease", 0).unwrap();
        }
        (state, author)
    }
    fn reserve<'a>(state: &'a Fixture, author: &str) -> QuotaReservation<'a> {
        let grant = match payment::claim_free_anchor(
            &state.store,
            author,
            Some("127.0.0.1".parse().unwrap()),
            12,
            None,
            state.free_anchors,
        )
        .unwrap()
        {
            payment::FreeAnchorClaim::Granted(grant) => grant,
            _ => panic!("expected quota"),
        };
        delivery::payment_free(state.store.lock().unwrap().conn(), "quota-op", "lease", 1).unwrap();
        QuotaReservation {
            grant: Some(grant),
            store: &state.store,
            operation_id: "quota-op",
            lease: "lease",
        }
    }
    fn assert_refunded(state: &Fixture, author: &str) {
        let store = state.store.lock().unwrap();
        assert_eq!(
            delivery::get(store.conn(), "quota-op")
                .unwrap()
                .unwrap()
                .payment_status,
            "required"
        );
        let keys = payment::FreeAnchorKeys::from_parts(
            store.conn(),
            "quota-guard-account",
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        let left = payment::free_anchors_remaining(
            store.conn(),
            &keys,
            &payment::utc_day(chrono::Utc::now()),
            state.free_anchors,
        )
        .unwrap();
        assert_eq!(left.account, state.free_anchors.per_account);
        drop(store);
        assert!(matches!(
            payment::claim_free_anchor(
                &state.store,
                author,
                Some("127.0.0.1".parse().unwrap()),
                12,
                None,
                state.free_anchors
            )
            .unwrap(),
            payment::FreeAnchorClaim::Granted(_)
        ));
    }
    #[test]
    fn upload_state_failure_refunds_only_after_restoring_required_payment() {
        let (state, author) = setup();
        let guard = reserve(&state, &author);
        {
            let store = state.store.lock().unwrap();
            store.conn().execute_batch("CREATE TRIGGER block_upload BEFORE UPDATE ON delivery_operations WHEN NEW.delivery_status='uploading' BEGIN SELECT RAISE(FAIL,'simulated metadata failure'); END;").unwrap();
            assert!(delivery::uploading(store.conn(), "quota-op", "lease", 2).is_err());
            delivery::release(
                store.conn(),
                "quota-op",
                "lease",
                "failed_retryable",
                Some("state_failure"),
                2,
            )
            .unwrap();
        }
        drop(guard);
        assert_refunded(&state, &author);
    }
    #[tokio::test]
    async fn cancelled_delivery_resets_free_eligibility_before_quota_refund() {
        let (state, author) = setup();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task_state = state.clone();
        let task_author = author.clone();
        let task = tokio::spawn(async move {
            let _guard = reserve(&task_state, &task_author);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_refunded(&state, &author);
    }
    #[test]
    fn failed_refund_metadata_keeps_quota_reserved_instead_of_free_retry() {
        let (state, author) = setup();
        let guard = reserve(&state, &author);
        state
            .store
            .lock()
            .unwrap()
            .conn()
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        drop(guard);
        let store = state.store.lock().unwrap();
        assert_eq!(
            delivery::get(store.conn(), "quota-op")
                .unwrap()
                .unwrap()
                .payment_status,
            "not_required"
        );
        let keys = payment::FreeAnchorKeys::from_parts(
            store.conn(),
            "quota-guard-account",
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        let left = payment::free_anchors_remaining(
            store.conn(),
            &keys,
            &payment::utc_day(chrono::Utc::now()),
            state.free_anchors,
        )
        .unwrap();
        assert_eq!(left.account, state.free_anchors.per_account - 1);
    }
}
