//! Integration tests for the T2 per-request `mode` field on
//! `mnemonic_sign_memory` (work/modes-user-choice).
//!
//! Six scenarios pin the user-spec invariants end-to-end through the MCP
//! HTTP dispatcher:
//!
//! 1. `local_against_full_server_returns_free` — `STORAGE_MODE=full +
//!    PAYMENT_MODE=x402`, `mode: "local"` returns free, no `attestation_costs`
//!    row, synthetic `local:` ids, row tagged `write_mode='local'`.
//! 2. `participate_against_local_only_server_returns_unsupported` —
//!    `STORAGE_MODE=local`, `mode: "participate"` → JSON-RPC `-32010
//!    UnsupportedMode { supported: ["local"] }`, DB unchanged.
//! 3. `mode_absent_response_shape_is_unchanged_from_legacy` — golden-
//!    fixture byte-equality on the response envelope for a `mode`-absent
//!    request (compat guard for the shipped chrome-extension's Cloud-tier
//!    consumer).
//! 4. `whoami_envelope_per_deploy_variant` — three sub-cases pinning the
//!    `supported_modes` / `default_mode` / `participate_cost` shape on
//!    local-only, self-operator (`full + none`), hosted-x402.
//! 5. `invalid_mode_returns_invalid_params` — table-driven over `null`,
//!    integer, array, object, `""`, `" "`, `"Local"`, `"PARTICIPATE"`,
//!    `"unknown"`. Each returns `-32602 InvalidParams` with
//!    `data.field == "mode"` and `data.received` echoing the input.
//! 6. `mixed_mode_coexistence_recall_returns_both` — one DB with one
//!    `local` row and one `participate` row for the same owner; `recall`
//!    surfaces both, each carries its stored `write_mode`. (NOTE:
//!    surfacing `write_mode` in the recall result envelope is T4 — this
//!    test seeds the rows directly via `save_attestation` and verifies
//!    they coexist; the recall-result shape part of the assertion is
//!    asserted with the underlying `search` API which is unchanged.)

mod _helpers;

use _helpers::TestServer;
use mnemonic_core::storage::{AttestationStore, WriteMode};
use serde_json::json;

// ── 1. local-mode write against a full-mode + paid server is free ──────────

#[tokio::test]
async fn local_against_full_server_returns_free() {
    // Full + x402 + non-zero cost. A `local` request must bypass the
    // paywall entirely (no x402 challenge, no charge) and persist with
    // synthetic `local:` ids — proves the gate is keyed on resolved
    // `WriteMode`, not env-var.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .sign_memory_cost_micro_usdc(10_000) // 1 cent / write
        .build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({"content": "free local note", "mode": "local"}),
        )
        .await;

    // No JSON-RPC error.
    assert!(
        result.error().is_none(),
        "expected success; got {:?}",
        result.envelope
    );

    let inner = result.result_text();
    assert_eq!(
        inner["write_mode"], "local",
        "response envelope must report write_mode = local"
    );
    let sol_tx = inner["solana_tx"].as_str().expect("solana_tx");
    let ar_tx = inner["arweave_tx"].as_str().expect("arweave_tx");
    assert!(sol_tx.starts_with("local:"), "got sol_tx={sol_tx}");
    assert!(ar_tx.starts_with("local:"), "got ar_tx={ar_tx}");

    // DB: 1 row, write_mode='local', no attestation_costs.
    assert_eq!(server.attestation_count(&owner), 1);
    let attestation_id = inner["attestation_id"].as_str().expect("attestation_id");
    assert_eq!(
        server.write_mode_for_tx(sol_tx),
        Some("local".to_string()),
        "row must be tagged write_mode='local'"
    );
    assert_eq!(
        server.attestation_cost_rows(attestation_id),
        0,
        "free path must NOT write an attestation_costs row"
    );
}

// ── 2. participate against local-only server: typed UnsupportedMode ────────

#[tokio::test]
async fn participate_against_local_only_server_returns_unsupported() {
    let server = TestServer::builder()
        .storage_mode("local")
        .payment_mode("none")
        .build();
    let owner = server.server_pubkey();

    let result = server
        .call_tool(
            Some(&owner),
            "mnemonic_sign_memory",
            json!({"content": "would-be paid", "mode": "participate"}),
        )
        .await;

    let err = result.expect_error();
    assert_eq!(err["code"], -32010, "expected -32010 UnsupportedMode");
    assert_eq!(err["message"], "Unsupported mode");
    let data = err["data"].as_object().expect("data object");
    assert_eq!(data["kind"], "UnsupportedMode");
    assert_eq!(data["requested"], "participate");
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
    // Captures the inline (no-JWT-deferred) path that the chrome-extension's
    // Cloud-tier sign_callback resolves to. The fixture pins the field NAMES
    // and TYPES; volatile values (uuids, timestamps, content hashes,
    // synthetic ids) are normalised before comparison so the test is stable
    // across runs.
    //
    // Why a local-only deploy: the shipped chrome-extension Local tier
    // never sends `mode`, and the server it talks to is `STORAGE_MODE=local`.
    // That's the path we must not break.
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
            inner["participate_cost"].is_null(),
            "local-only deploy must render participate_cost = null"
        );
        // Legacy `storage_mode` field still present (chrome-extension reads it).
        assert_eq!(inner["storage_mode"], "local");
    }

    // 4b. Self-operator (`full + none`) → ["local","participate"], cost 0 / [].
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
        assert_eq!(inner["supported_modes"], json!(["local", "participate"]));
        assert_eq!(inner["default_mode"], "local");
        let cost = inner["participate_cost"]
            .as_object()
            .expect("participate_cost must be an object on full + none");
        assert_eq!(cost["currency"], "USD");
        assert_eq!(cost["amount_cents"], 0);
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
        assert_eq!(inner["supported_modes"], json!(["local", "participate"]));
        let cost = inner["participate_cost"]
            .as_object()
            .expect("participate_cost must be an object on full + x402");
        assert_eq!(cost["amount_cents"], 5);
        assert_eq!(cost["payment_methods"], json!(["x402"]));
    }
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
    // Seed one local row + one participate row directly via the storage
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
                &[0.1; 8],
            )
            .expect("save local row");
        store
            .save_attestation(
                "att-participate-id",
                "participate content",
                "hash-participate",
                &["t".to_string()],
                // Real-looking (non-`local:`) tx ids so the row falls on
                // the participate side of the synthetic-id discrimination.
                "5j2K9aRRRRYWfLh8x2y2y9X7Cxe1aN3aN3aN3aN3aN3a",
                "PdhTPLPmHvX0iE6iAtJ8X5Y0WqQ8MzC8KvU9JhQ0aN0",
                &owner,
                &owner,
                &now,
                WriteMode::Participate,
                &[0.1; 8],
            )
            .expect("save participate row");
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
        "expected both local + participate rows to surface in recall"
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
        Some("participate".to_string())
    );
}
