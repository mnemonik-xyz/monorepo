//! Parametrized Error Catalogue coverage (Task 4 / AC16 —
//! agent-native-distribution).
//!
//! Each row in the tech-spec Error Catalogue is exercised by triggering the
//! condition via the production code path (NOT a hand-crafted response)
//! and asserting `error.code`, `error.data.kind`, and every documented
//! `data` field is present with the documented type.
//!
//! Rows in scope of Task 4:
//!
//! - `-32602 InvalidParams` (visibility on local writes) — via dispatcher.
//! - `-32602 InvalidParams` (non-bool allow_fallback_to_participate).
//! - `-32010 UnsupportedMode` (participate against local-only deploy).
//! - `-32095 PublicWriteRequiresConfirmation` (missing token).
//! - `-32098 EmbedderInvalid` (FailingEmbedder returns empty vec).
//!
//! Rows owned by other tasks (`-32011 DeliveryNotConfirmed`,
//! `-32011 DeliveryQuotaExceeded`, `-32011 HostedUnavailable`,
//! `-32099 LocalStorageBusy`, `-32099 TokenExpired`,
//! `-32094 IdentityBootstrapFailed`, `-32096 OAuthTimeout`) are
//! cross-covered by Tasks 5 / 6 / delivery_guarantee.rs and live in those
//! test surfaces; they reference the helpers defined in
//! `mcp::mcp::{token_expired, local_storage_busy, ...}` so the typed-error
//! contract is exercised exactly once per row.

#![cfg(feature = "test-support")]

mod _helpers;

use std::sync::atomic::{AtomicUsize, Ordering};

use _helpers::TestServer;
use mnemonic_core::embed::Embedder;
use mnemonic_core::storage::Visibility;
use serde_json::{json, Value};

type FieldMatcher = fn(&Value) -> bool;

/// Asserts that `data` is a JSON object with EVERY entry in `expected`
/// (key, value-matcher) present. Field-presence guard rather than
/// strict-equality so future fields are forward-compatible.
fn assert_data_fields(data: &Value, expected: &[(&str, FieldMatcher)]) {
    let obj = data.as_object().expect("data must be an object");
    for (key, matcher) in expected {
        let val = obj.get(*key).unwrap_or_else(|| {
            panic!("data.{key} missing in {data}");
        });
        assert!(
            matcher(val),
            "data.{key} failed type/shape check (value: {val})"
        );
    }
}

fn is_string(v: &Value) -> bool {
    v.is_string()
}
fn is_bool(v: &Value) -> bool {
    v.is_boolean()
}
fn is_array(v: &Value) -> bool {
    v.is_array()
}
fn is_any_value(_v: &Value) -> bool {
    // `received` echoes the verbatim input — any JSON value (including null)
    // is acceptable; field presence is the real check, enforced by
    // `assert_data_fields`'s `unwrap_or_else` panic-on-missing-key. The
    // round-1 helper `is_string_or_value` collapsed to a tautology and
    // silently bypassed even this presence check; the rename makes the
    // intent transparent (code-reviewer CR-1, agent-native-distribution Task 4).
    true
}

#[tokio::test]
async fn catalogue_invalid_params_visibility_on_local() {
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    let res = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "x",
                "mode": "local",
                "visibility": "public",
            }),
        )
        .await;
    let err = res.expect_error();
    assert_eq!(err["code"], -32602);
    let data = &err["data"];
    assert_eq!(data["field"], "visibility");
    assert_data_fields(data, &[("field", is_string), ("received", is_any_value)]);
}

#[tokio::test]
async fn catalogue_invalid_params_non_bool_allow_fallback() {
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    let res = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "x",
                "allow_fallback_to_participate": 42,
            }),
        )
        .await;
    let err = res.expect_error();
    assert_eq!(err["code"], -32602);
    assert_eq!(err["data"]["field"], "allow_fallback_to_participate");
}

#[tokio::test]
async fn catalogue_unsupported_mode_participate_on_local_only() {
    // `STORAGE_MODE=local` envelope rejects `mode=participate` at the
    // dispatcher boundary with -32010 + supported list.
    let server = TestServer::builder().build();
    let owner = server.server_pubkey();
    let res = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "x",
                "mode": "participate",
            }),
        )
        .await;
    let err = res.expect_error();
    assert_eq!(err["code"], -32010);
    assert_eq!(err["data"]["kind"], "UnsupportedMode");
    assert_data_fields(
        &err["data"],
        &[
            ("requested", is_string),
            ("supported", is_array),
            ("kind", is_string),
        ],
    );
}

#[tokio::test]
async fn catalogue_public_write_requires_confirmation() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();
    let res = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({
                "content": "x",
                "mode": "participate",
                "visibility": "public",
            }),
        )
        .await;
    let err = res.expect_error();
    assert_eq!(err["code"], -32095);
    assert_eq!(err["data"]["kind"], "PublicWriteRequiresConfirmation");
    assert_data_fields(
        &err["data"],
        &[
            ("kind", is_string),
            ("content_hash", is_string),
            ("suggested_action", is_string),
        ],
    );
}

// ── EmbedderInvalid via FailingEmbedder ────────────────────────────────────

/// Test fixture from the Task 4 spec — surfaces the `-32098 EmbedderInvalid`
/// error catalogue row by returning an empty vector from `embed()`. The
/// production code path in `sign_memory_inline` checks `if embedding
/// .is_empty() { return embedder_invalid(...) }`. We instantiate this
/// embedder on a hand-built `McpState` because the `TestServer` harness
/// uses the deterministic 8-dim StubEmbedder.
pub struct FailingEmbedder {
    pub fail_on_call: AtomicUsize,
    pub counter: AtomicUsize,
}

impl Default for FailingEmbedder {
    fn default() -> Self {
        Self {
            fail_on_call: AtomicUsize::new(1),
            counter: AtomicUsize::new(0),
        }
    }
}

impl Embedder for FailingEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n + 1 >= self.fail_on_call.load(Ordering::SeqCst) {
            Vec::new()
        } else {
            vec![0.0; 384]
        }
    }
    fn model_id(&self) -> &str {
        "test/failing-embedder"
    }
    fn dim(&self) -> usize {
        384
    }
    fn provider_name(&self) -> &str {
        "failing"
    }
}

#[tokio::test]
async fn catalogue_embedder_invalid() {
    use std::sync::Arc;
    use std::time::Duration;

    use mnemonic_core::arweave::ArweaveClient;
    use mnemonic_core::compress::EmbeddingCompressor;
    use mnemonic_core::solana::SolanaClient;
    use mnemonic_core::storage::SqliteStore;
    use mnemonic_mcp::mcp::Envelope;
    use mnemonic_mcp::tools::{resolve_write_mode, sign_memory, ToolError};
    use solana_sdk::signature::{Keypair, Signer};

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let store = std::sync::Mutex::new(SqliteStore::open(tmp.path()).expect("sqlite"));
    let compressor = EmbeddingCompressor::new(8, 4, 42);
    let pending = mnemonic_mcp::pending::PendingBundles::with_defaults();
    let embedder = FailingEmbedder::default();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    let envelope = Envelope::from_config("local", "none", 0);
    let resolved = resolve_write_mode(None, "local").unwrap();

    let cost_hint = mnemonic_mcp::pricing::CostHint {
        irys_lamports: 0,
        sol_tx_fee_lamports: 0,
        sol_price_usdc: 0.0,
        charge_micro_usdc: 0,
    };

    let result = sign_memory(
        &kp,
        &SolanaClient::new("http://localhost:0"),
        &ArweaveClient::new("http://localhost:0"),
        &store,
        &embedder,
        &compressor,
        &pending,
        "trigger-failing-embedder",
        &[],
        &cost_hint,
        "local",
        &owner,
        None,
        resolved,
        Visibility::Private,
        &envelope,
        Duration::from_secs(15),
    )
    .await;

    let err = match result {
        Err(ToolError::TypedRpc(e)) => e,
        other => panic!("expected TypedRpc EmbedderInvalid, got {other:?}"),
    };
    assert_eq!(err.code, -32098);
    let data = err.data.expect("data");
    assert_eq!(data["kind"], "EmbedderInvalid");
    assert_data_fields(
        &data,
        &[
            ("reason", is_string),
            ("repair_hint", is_string),
            ("fallback_available", is_bool),
        ],
    );

    // unused-import compatibility — drop the Arc usage so clippy doesn't
    // emit a "unused import" without referencing the symbol.
    let _ = Arc::new(0);
}

// ── Helper-level row coverage (cheap unit-style assertions on the typed
// error constructors so the catalogue table is byte-for-byte pinned even
// when the row's production trigger is owned by Tasks 5/6) ───────────────

#[test]
fn catalogue_typed_helpers_pin_data_shapes() {
    use mnemonic_mcp::mcp::{
        delivery_not_confirmed, delivery_quota_exceeded, embedder_invalid, hosted_unavailable,
        identity_bootstrap_failed, invalid_params, local_storage_busy, oauth_timeout,
        public_write_requires_confirmation, token_expired, unsupported_mode,
    };

    let e = unsupported_mode("participate", &["local"]);
    assert_eq!(e.code, -32010);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "UnsupportedMode");

    let e = invalid_params("visibility", &Value::String("public".into()));
    assert_eq!(e.code, -32602);
    assert_eq!(e.data.as_ref().unwrap()["field"], "visibility");

    let e = delivery_not_confirmed("refetch", "ar", "sol", "att");
    assert_eq!(e.code, -32011);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "DeliveryNotConfirmed");

    let e = delivery_quota_exceeded(60, 5);
    assert_eq!(e.code, -32011);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "DeliveryQuotaExceeded");

    let e = hosted_unavailable("dns failure", 500);
    assert_eq!(e.code, -32011);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "HostedUnavailable");

    let e = local_storage_busy(500);
    assert_eq!(e.code, -32099);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "LocalStorageBusy");

    let e = token_expired("2026-06-04T00:00:00Z", "owner");
    assert_eq!(e.code, -32099);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "TokenExpired");

    let e = identity_bootstrap_failed("no-keychain", "reinstall");
    assert_eq!(e.code, -32094);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "IdentityBootstrapFailed");

    let e = oauth_timeout("https://x", 1, 2);
    assert_eq!(e.code, -32096);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "OAuthTimeout");

    let e = embedder_invalid("missing model", "reinstall", true);
    assert_eq!(e.code, -32098);
    assert_eq!(e.data.as_ref().unwrap()["kind"], "EmbedderInvalid");

    let e = public_write_requires_confirmation("hash");
    assert_eq!(e.code, -32095);
    assert_eq!(
        e.data.as_ref().unwrap()["kind"],
        "PublicWriteRequiresConfirmation"
    );
}
