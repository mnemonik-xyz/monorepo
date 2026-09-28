//! Integration tests for the T2 per-request `mode` field on
//! `mnemonic_sign_memory` (work/modes-user-choice).
//!
//! Seven scenarios pin the user-spec invariants end-to-end through the
//! MCP HTTP dispatcher:
//!
//! 1. `local_against_full_server_returns_free` — `STORAGE_MODE=full +
//!    PAYMENT_MODE=x402`, `mode: "local"` returns free, no
//!    `attestation_costs` row, synthetic `local:` ids, row tagged
//!    `write_mode='local'`.
//! 2. `explicit_local_against_local_only_server_is_inline_not_deferred`
//!    (round-2 regression guard) — local-only deploy + JWT + explicit
//!    `mode: "local"` routes through the inline path, NOT the deferred
//!    branch. Round-1 had this case silently routing to deferred.
//!    `explicit_local_remote_user_is_hash_only_not_signed` — a remote JWT
//!    subject's explicit-local write is an inline hash-only row owned by
//!    that subject (no operator or client signature);
//!    `anchored_remote_user_is_deferred_not_operator_signed` — a remote
//!    anchored write stays on the client-signing path.
//! 2. `anchored_against_local_only_server_returns_unsupported` —
//!    `STORAGE_MODE=local`, `mode: "anchored"` → JSON-RPC `-32010
//!    UnsupportedMode { supported: ["local"] }`, DB unchanged.
//! 3. `mode_absent_response_shape_is_unchanged_from_legacy` — golden-
//!    fixture byte-equality on the response envelope for a `mode`-absent
//!    request (compat guard for the shipped chrome-extension's Cloud-tier
//!    consumer).
//! 4. `whoami_envelope_per_deploy_variant` — three sub-cases pinning the
//!    `supported_modes` / `default_mode` / `anchored_cost` shape on
//!    local-only, self-operator (`full + none`), hosted-x402.
//! 5. `invalid_mode_returns_invalid_params` — table-driven over `null`,
//!    integer, array, object, `""`, `" "`, `"Local"`, `"PARTICIPATE"`,
//!    `"unknown"`. Each returns `-32602 InvalidParams` with
//!    `data.field == "mode"` and `data.received` echoing the input.
//! 6. `mixed_mode_coexistence_recall_returns_both` — one DB with one
//!    `local` row and one `anchored` row for the same owner; `recall`
//!    surfaces both, each carries its stored `write_mode`. (NOTE:
//!    surfacing `write_mode` in the recall result envelope is T4 — this
//!    test seeds the rows directly via `save_attestation` and verifies
//!    they coexist; the recall-result shape part of the assertion is
//!    asserted with the underlying `search` API which is unchanged.)

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
use serde_json::json;
use solana_sdk::signature::{Keypair, Signer as _};

// ── 1. local-mode write against a full-mode + paid server is free ──────────

#[tokio::test]
async fn explicit_local_over_http_is_refused() {
    // Replaces three tests that asserted a hosted `mode: "local"` write landed
    // in the OPERATOR's database — free on a local-only deploy, hash-only for a
    // remote JWT subject. That tier is retired: `local` means the agent's own
    // machine, and a hosted deploy cannot provide it
    // (work/binary-mode-cleanup, work/arweave-as-source-of-truth Decision 8).
    //
    // Local-mode coverage did not disappear: it moved to the stdio transport,
    // where the mode is still legal and where the operator key really is the
    // agent's own identity.
    for storage_mode in ["local", "full"] {
        let server = TestServer::builder().storage_mode(storage_mode).build();
        let owner = server.server_pubkey();

        let result = server
            .call_tool(
                Some(&owner),
                "mnemonic_sign_memory",
                json!({"content": "hosted local is gone", "mode": "local"}),
            )
            .await;

        let err = result.expect_error();
        assert_eq!(err["code"], -32010, "on {storage_mode}: {err:?}");
        assert_eq!(err["data"]["kind"], "UnsupportedMode");
        assert_eq!(err["data"]["requested"], "local");
        assert_eq!(
            server.attestation_count(&owner),
            0,
            "a refused write must leave no row behind"
        );
    }
}

// ── 1b (round-2). Explicit local request on a local-only deploy bypasses ───
//     the deferred-signing path. Without the round-2 fix this case routed
//     to deferred (returning `awaiting_signature`) because the round-1
//     `envelope.supports_anchored()` predicate was false on local
//     deploys. The user-spec invariant "Личная память бесплатна всегда"
//     applies uniformly across deploys, not just to `full + JWT`.

// ── 1c. A REMOTE user's explicit-local write is a hash-only row ──────────
//     owned by the remote user. Nothing is signed: no operator signature
//     (the operator never signs for another identity) and no client
//     signature (a free local write needs no keychain prompt). Only
//     anchored writes are client-signed — see 1d.

// ── 1d. A REMOTE user's anchored write stays client-signed ─────────────
//     (deferred). The operator key never signs an anchored memory for
//     another identity.

#[tokio::test]
async fn anchored_remote_user_is_deferred_not_operator_signed() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("none")
        .build();
    // Use a valid base58 Solana pubkey (32 bytes decoded). Task 6: the sealed
    // path (default visibility = private) decodes jwt_sub as a Solana pubkey to
    // derive the owner's X25519 key for encryption.
    let remote_kp = solana_sdk::signature::Keypair::new();
    let remote = remote_kp.pubkey().to_string();
    let operator = server.server_pubkey();
    assert_ne!(remote, operator.as_str());

    let result = server
        .call_tool(
            Some(remote.as_str()),
            "mnemonic_sign_memory",
            json!({"content": "remote anchored memo", "mode": "anchored"}),
        )
        .await;
    assert!(result.error().is_none(), "envelope: {:?}", result.envelope);
    let inner = result.result_text();
    assert_eq!(
        inner["status"], "awaiting_signature",
        "remote anchored must defer to client signing, got {inner:?}",
    );
    assert!(inner["correlation_id"].is_string(), "{inner:?}");
    assert!(inner.get("attestation_id").is_none(), "{inner:?}");
    assert_eq!(server.attestation_count(remote.as_str()), 0);
    assert_eq!(server.attestation_count(&operator), 0);
}

// ── 2. anchored against local-only server: typed UnsupportedMode ────────

#[tokio::test]
async fn anchored_against_local_only_server_returns_unsupported() {
    let server = TestServer::builder()
        .storage_mode("local")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({"content": "would-be paid", "mode": "anchored"}),
        )
        .await;

    let err = result.expect_error();
    assert_eq!(err["code"], -32010, "expected -32010 UnsupportedMode");
    assert_eq!(err["message"], "Unsupported mode");
    let data = err["data"].as_object().expect("data object");
    assert_eq!(data["kind"], "UnsupportedMode");
    assert_eq!(data["requested"], "anchored");
    assert_eq!(
        data["supported"],
        json!(["local"]),
        "local-only deploy must advertise only [\"local\"]"
    );
    assert_eq!(
        server.attestation_count(&owner),
        0,
        "rejected request must not persist any row"
    );
}

// ── 3. Golden-fixture compat: mode-absent response shape is stable ─────────

#[tokio::test]
async fn mode_absent_response_shape_is_unchanged_from_legacy() {
    // Captures the DEFERRED-SIGNING envelope (`status: "awaiting_signature"`)
    // that an HTTP/JWT caller without an explicit `mode` field sees against
    // the test_support default deploy (`STORAGE_MODE=local`, JWT present).
    // The resolver maps None+local → `ResolvedMode::fallback(Local)` —
    // marked `explicit = false` — and the routing rule in `sign_memory`
    // routes non-explicit JWT requests to the deferred path. The fixture
    // pins the field NAMES; volatile values (uuids in `correlation_id`,
    // `approve_url`, `next_step`) are normalised before comparison so the
    // test is stable across runs.
    //
    // This is the wire shape the shipped chrome-extension's Cloud-tier
    // consumer expects byte-for-byte — any drift on a real field fails
    // here even if every other test passes.
    let server = TestServer::builder()
        .storage_mode("local")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            // NO `mode` field — legacy clients (shipped extension).
            json!({"content": "legacy memo", "tags": ["compat"]}),
        )
        .await;
    assert!(result.error().is_none(), "envelope: {:?}", result.envelope);
    let actual = normalise_volatile(&result.result_text());

    // Optional regen path: set `REGEN_FIXTURES=1` in the env to write the
    // current shape back to the fixture instead of comparing. Used once
    // after intentional schema changes — never in CI.
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/modes/legacy_sign_response.json");
    if std::env::var_os("REGEN_FIXTURES").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(&fixture_path, pretty + "\n").expect("write fixture");
        eprintln!("REGEN_FIXTURES set — wrote {}", fixture_path.display());
        return;
    }

    let fixture_bytes = std::fs::read(&fixture_path).unwrap_or_else(|e| {
        panic!(
            "failed to read golden fixture {}: {e}\nrun with `REGEN_FIXTURES=1` to bootstrap",
            fixture_path.display()
        )
    });
    let expected: serde_json::Value =
        serde_json::from_slice(&fixture_bytes).expect("golden fixture must be valid JSON");

    assert_eq!(
        actual, expected,
        "response shape drift — if intentional, run with REGEN_FIXTURES=1 to regenerate the fixture at {}",
        fixture_path.display()
    );
}

/// Replace volatile fields (uuids, timestamps, synthetic ids, content
/// hashes, bytes counts that depend on uuid length, signer pubkeys) with
/// stable placeholder strings so the test can byte-compare across runs.
fn normalise_volatile(v: &serde_json::Value) -> serde_json::Value {
    let mut out = v.clone();
    if let Some(obj) = out.as_object_mut() {
        for key in [
            "attestation_id",
            "content_hash",
            "solana_tx",
            "arweave_tx",
            "signer",
            "did_sol",
            "timestamp",
            // Deferred-signing path also has these volatile fields:
            "correlation_id",
            "approve_url",
            "next_step",
            // Wave 2 — programmatic client-signing handoff (volatile bytes/ids):
            "canonical_cbor_b64",
            "client_sign",
        ] {
            if obj.contains_key(key) {
                obj.insert(
                    key.to_string(),
                    serde_json::Value::String("<NORMALISED>".into()),
                );
            }
        }
        // Compression block: `original_bytes` and `compressed_bytes` are
        // stable for the StubEmbedder (8 dims), so we leave them; just
        // normalise the formatted `ratio` (e.g. "8.0x" — stable too in
        // practice but defensive). Embedding block carries `model`,
        // `provider`, `dim`, `verifiable`; all stable for `StubEmbedder`.
    }
    out
}

// ── 4. whoami envelope per deploy variant ──────────────────────────────────

#[tokio::test]
async fn whoami_envelope_per_deploy_variant() {
    // 4a. Local-only deploy → ["local"] / null cost.
    {
        let server = TestServer::builder().storage_mode("local").build();
        let owner = server.server_pubkey();
        let result = server
            .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
            .await;
        let inner = result.result_text();
        assert_eq!(inner["supported_modes"], json!(["local"]));
        assert_eq!(inner["default_mode"], "local");
        assert!(
            inner["anchored_cost"].is_null(),
            "local-only deploy must render anchored_cost = null"
        );
        // Legacy `storage_mode` field still present (chrome-extension reads it).
        assert_eq!(inner["storage_mode"], "local");
    }

    // 4b. Self-operator (`full + none`) → ["local","anchored"], cost 0 / [].
    {
        let server = TestServer::builder()
            .storage_mode("full")
            .payment_mode("none")
            .build();
        let owner = server.server_pubkey();
        let result = server
            .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
            .await;
        let inner = result.result_text();
        assert_eq!(inner["supported_modes"], json!(["local", "anchored"]));
        assert_eq!(inner["default_mode"], "local");
        let cost = inner["anchored_cost"]
            .as_object()
            .expect("anchored_cost must be an object on full + none");
        assert_eq!(cost["currency"], "USD");
        assert_eq!(cost["amount_cents"], 0);
        assert_eq!(cost["amount_micro_usdc"], 0);
        // #165: "free" is explicit, not a side effect of truncation.
        assert_eq!(cost["pricing_status"], "disabled");
        assert_eq!(cost["payment_methods"], json!([]));
    }

    // 4c. Hosted-x402 (`full + x402`, non-zero cost) → ["x402"] methods.
    {
        let server = TestServer::builder()
            .storage_mode("full")
            .payment_mode("x402")
            .sign_memory_cost_micro_usdc(50_000) // $0.05 → 5 cents
            .build();
        let owner = server.server_pubkey();
        let result = server
            .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
            .await;
        let inner = result.result_text();
        assert_eq!(inner["supported_modes"], json!(["local", "anchored"]));
        // `default_mode` is invariant across deploys: always "local"
        // (user-spec — the free-private-memory path is the default,
        // operators don't get to override).
        assert_eq!(inner["default_mode"], "local");
        let cost = inner["anchored_cost"]
            .as_object()
            .expect("anchored_cost must be an object on full + x402");
        assert_eq!(cost["currency"], "USD");
        assert_eq!(cost["amount_cents"], 5);
        assert_eq!(cost["amount_micro_usdc"], 50_000);
        // No refresh has run in the test server: the seed price is a floor.
        assert_eq!(cost["pricing_status"], "fallback");
        assert_eq!(cost["payment_methods"], json!(["x402"]));
    }
}

// ── 4b. whoami pricing block: floor rounding + live refresh (#165) ─────────

fn whoami_cost(inner: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    inner["anchored_cost"]
        .as_object()
        .cloned()
        .expect("anchored_cost must be an object on full + x402")
}

/// The default 1000 µUSDC floor ($0.001) used to truncate to
/// `amount_cents: 0` and read as "free". It must round UP to 1 cent, carry
/// the exact micro-USDC amount, and say it is a fallback price.
#[tokio::test]
async fn whoami_floor_price_never_renders_as_zero() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(1000)
        .build();
    let owner = server.server_pubkey();
    let result = server
        .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
        .await;
    let cost = whoami_cost(&result.result_text());
    assert_eq!(cost["amount_micro_usdc"], 1000);
    assert_eq!(cost["amount_cents"], 1, "non-zero price must not show 0");
    assert_eq!(cost["pricing_status"], "fallback");
}

/// The envelope used to be frozen at boot. A background refresh (or a
/// refresh failure) must show up on the next `mnemonic_whoami` call.
#[tokio::test]
async fn whoami_tracks_live_pricing_engine_state() {
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(1000)
        .build();
    let owner = server.server_pubkey();
    let pricing_cfg = mnemonic_mcp::pricing::PricingConfig {
        margin_bps: 0,
        min_price_micro_usdc: 1000,
        typical_payload_bytes: 2048,
        sol_tx_fee_lamports: 0,
    };
    // 200_000 lamports at $100/SOL = 20_000 µUSDC = 2 cents.
    server
        .state
        .pricing
        .apply_quote(200_000, 100.0, &pricing_cfg)
        .expect("valid quote");

    let result = server
        .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
        .await;
    let cost = whoami_cost(&result.result_text());
    assert_eq!(cost["amount_micro_usdc"], 20_000);
    assert_eq!(cost["amount_cents"], 2);
    assert_eq!(cost["pricing_status"], "live");
    assert_eq!(cost["payment_methods"], json!(["x402"]));

    // Next refresh fails: last good quote stays, status degrades.
    server.state.pricing.record_refresh_failure();
    let result = server
        .call_tool(Some(&owner), "mnemonic_whoami", json!({}))
        .await;
    let cost = whoami_cost(&result.result_text());
    assert_eq!(cost["amount_micro_usdc"], 20_000);
    assert_eq!(cost["pricing_status"], "fallback");
}

// ── 5. Malformed `mode` returns InvalidParams (table-driven) ──────────────

#[tokio::test]
async fn invalid_mode_returns_invalid_params() {
    let server = TestServer::builder().storage_mode("local").build();
    let owner = server.server_pubkey();

    // (label, JSON value to put under `mode`)
    let cases: Vec<(&str, serde_json::Value)> = vec![
        ("null", serde_json::Value::Null),
        ("integer", json!(42)),
        ("array", json!(["local"])),
        ("object", json!({"mode": "local"})),
        ("empty-string", json!("")),
        ("whitespace", json!(" ")),
        ("capital-Local", json!("Local")),
        ("upper-PARTICIPATE", json!("PARTICIPATE")),
        ("unknown", json!("unknown")),
    ];

    for (label, value) in cases {
        let args = json!({"content": "x", "mode": value});
        let result = server
            .call_tool(Some(&owner), "mnemonic_sign_memory", args.clone())
            .await;
        let err = result.expect_error();
        assert_eq!(
            err["code"], -32602,
            "[{label}] expected -32602 InvalidParams, envelope={:?}",
            result.envelope
        );
        let data = err["data"]
            .as_object()
            .unwrap_or_else(|| panic!("[{label}] missing data object"));
        assert_eq!(
            data["field"], "mode",
            "[{label}] data.field must be \"mode\""
        );
        assert_eq!(
            data["received"], value,
            "[{label}] data.received must echo input verbatim"
        );
    }
    // No rows persisted across the whole table.
    assert_eq!(server.attestation_count(&owner), 0);
}

// ── 6. Mixed-mode coexistence: recall surfaces both ────────────────────────

#[tokio::test]
async fn mixed_mode_coexistence_recall_returns_both() {
    // Seed one local row + one anchored row directly via the storage
    // API so the test is independent of which env-var resolution path
    // produces each row (the resolver is already covered elsewhere).
    let server = TestServer::builder().storage_mode("local").build();
    let owner = server.server_pubkey();

    let now = chrono::Utc::now().to_rfc3339();
    {
        let store = server.state.store.lock().unwrap();
        store
            .save_attestation(
                "att-local-id",
                "local content",
                "hash-local",
                &["t".to_string()],
                "local:abcdef0123456789",
                "local:abcdef01",
                &owner, // signer
                &owner, // owner — same in single-tenant
                &now,
                WriteMode::Local,
                Visibility::Private,
                &[0.1; 8],
            )
            .expect("save local row");
        store
            .save_attestation(
                "att-anchored-id",
                "anchored content",
                "hash-anchored",
                &["t".to_string()],
                // Real-looking (non-`local:`) tx ids so the row falls on
                // the anchored side of the synthetic-id discrimination.
                "5j2K9aRRRRYWfLh8x2y2y9X7Cxe1aN3aN3aN3aN3aN3a",
                "PdhTPLPmHvX0iE6iAtJ8X5Y0WqQ8MzC8KvU9JhQ0aN0",
                &owner,
                &owner,
                &now,
                WriteMode::Anchored,
                Visibility::Private,
                &[0.1; 8],
            )
            .expect("save anchored row");
    }

    // Recall via the MCP tool: a search should surface both rows under the
    // owner's scope.
    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_recall",
            json!({"query": "content", "limit": 10}),
        )
        .await;
    assert!(
        result.error().is_none(),
        "recall envelope: {:?}",
        result.envelope
    );
    let inner = result.result_text();
    let results = inner["results"].as_array().expect("results array").clone();
    assert_eq!(
        results.len(),
        2,
        "expected both local + anchored rows to surface in recall"
    );

    // Verify each row's stored `write_mode` matches the seeded value. The
    // recall result envelope (T4 surfaces write_mode) is out of scope for
    // T2; here we go straight to the DB to assert coexistence.
    assert_eq!(
        server.write_mode_for_tx("local:abcdef0123456789"),
        Some("local".to_string())
    );
    assert_eq!(
        server.write_mode_for_tx("5j2K9aRRRRYWfLh8x2y2y9X7Cxe1aN3aN3aN3aN3aN3a"),
        Some("anchored".to_string())
    );
}
