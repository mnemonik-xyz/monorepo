//! Client signatures, sealed storage, recipient recall and real SDK HTTP tests.
mod _helpers;
use _helpers::TestServer;
use mnemonic_core::codec::a2a::signed::prepare_signed_a2a;
use serde_json::{json, Value};
use solana_sdk::signature::{Keypair, Signer};

fn result(resp: &_helpers::CallResult) -> Value {
    assert!(resp.envelope["error"].is_null(), "{:?}", resp.envelope);
    serde_json::from_str(
        resp.envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap()
}
fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap()
}
fn signed_args(v: &Value, key: &str) -> Value {
    json!({"kind":"message","context_id":"fixture-ctx","signed":v[key],"sealed":key!="plain"})
}

#[tokio::test]
async fn sealed_a2a_ingest_recall_is_non_custodial_and_recipient_scoped() {
    let server = TestServer::builder().build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let reader = v["reader"]["pubkey_base58"].as_str().unwrap();
    let outsider = v["outsider"]["pubkey_base58"].as_str().unwrap();
    assert_ne!(author, server.server_pubkey());
    let resp = server
        .call_tool(
            Some(author),
            "mnemonic_attest_a2a",
            signed_args(&v, "sealed"),
        )
        .await;
    let stored = result(&resp);
    let replay = result(
        &server
            .call_tool(
                Some(author),
                "mnemonic_attest_a2a",
                signed_args(&v, "sealed"),
            )
            .await,
    );
    assert_eq!(stored["attestation_id"], replay["attestation_id"]);
    for identity in [author, reader] {
        let rows = result(
            &server
                .call_tool(
                    Some(identity),
                    "mnemonic_recall_a2a",
                    json!({"context_id":"fixture-ctx","sealed":true}),
                )
                .await,
        );
        assert_eq!(rows["attestations"].as_array().unwrap().len(), 1);
        assert_eq!(rows["attestations"][0]["cose_envelope_hex"], v["sealed"]);
        assert_eq!(rows["attestations"][0]["signer_pubkey"], author);
    }
    for identity in [None, Some(outsider)] {
        let rows = result(
            &server
                .call_tool(
                    identity,
                    "mnemonic_recall_a2a",
                    json!({"context_id":"fixture-ctx","sealed":true}),
                )
                .await,
        );
        assert!(rows["attestations"].as_array().unwrap().is_empty());
    }
    let guard = server.state.store.lock().unwrap();
    let (content, privacy, blob): (String, String, Vec<u8>) = guard
        .conn()
        .query_row(
            "SELECT content,privacy,sealed_blob FROM attestations WHERE attestation_id=?1",
            [stored["attestation_id"].as_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(content.is_empty());
    assert_eq!(privacy, "sealed");
    assert!(!blob.is_empty());
    let count: i64 = guard
        .conn()
        .query_row("SELECT COUNT(*) FROM attestation_embeddings", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a2a_rejects_wrong_signer_unsigned_and_tool_binding_mismatch() {
    let server = TestServer::builder().build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let outsider = v["outsider"]["pubkey_base58"].as_str().unwrap();
    for args in [
        json!({"kind":"message","context_id":"fixture-ctx","payload":v["payload"]}),
        {
            let mut a = signed_args(&v, "sealed");
            a["sealed"] = json!(false);
            a
        },
        {
            let mut a = signed_args(&v, "sealed");
            a["context_id"] = json!("wrong");
            a
        },
        {
            let mut a = signed_args(&v, "sealed");
            a["kind"] = json!("artifact");
            a
        },
        {
            let mut a = signed_args(&v, "sealed");
            a["prev_id"] = json!("forged-parent");
            a
        },
    ] {
        let resp = server
            .call_tool(Some(author), "mnemonic_attest_a2a", args)
            .await;
        assert!(resp.envelope["error"].is_object());
    }
    assert!(server
        .call_tool(
            Some(outsider),
            "mnemonic_attest_a2a",
            signed_args(&v, "sealed")
        )
        .await
        .envelope["error"]
        .is_object());
    assert_eq!(
        server
            .state
            .store
            .lock()
            .unwrap()
            .conn()
            .query_row("SELECT COUNT(*) FROM attestations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn filters_apply_before_limit_and_parent_is_signed_and_context_bound() {
    let server = TestServer::builder().build();
    let v = vectors();
    let author = Keypair::new_from_array([1; 32]);
    let sub = author.pubkey().to_string();
    let parent = result(
        &server
            .call_tool(Some(&sub), "mnemonic_attest_a2a", signed_args(&v, "sealed"))
            .await,
    );
    let parent_id = parent["attestation_id"].as_str().unwrap();
    let payload = json!({"artifactId":"art","parts":[]});
    let signed = prepare_signed_a2a(
        &author,
        "artifact",
        payload,
        "fixture-ctx",
        Some(parent_id.into()),
        "2026-10-01T00:00:01Z",
        None,
    )
    .unwrap();
    result(&server.call_tool(Some(&sub),"mnemonic_attest_a2a",json!({"kind":"artifact","context_id":"fixture-ctx","prev_id":parent_id,"signed":hex::encode(signed),"sealed":false})).await);
    let rows = result(
        &server
            .call_tool(
                Some(&sub),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx","kind":"message","limit":1,"sealed":true}),
            )
            .await,
    );
    assert_eq!(rows["attestations"][0]["attestation_id"], parent_id);
    let rows = result(
        &server
            .call_tool(
                Some(&sub),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx","sealed":false}),
            )
            .await,
    );
    assert_eq!(rows["attestations"][0]["prev_id"], parent_id);
}

#[tokio::test]
async fn sealed_a2a_store_survives_reopen_and_parent_failure_is_atomic() {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let v = vectors();
    let bytes = hex::decode(v["stream"].as_str().unwrap()).unwrap();
    let verified = mnemonic_core::codec::a2a::signed::verify_signed_a2a(&bytes, None).unwrap();
    {
        let store = mnemonic_core::storage::SqliteStore::open(temp.path()).unwrap();
        store.save_signed_a2a(&bytes, &verified).unwrap();
    }
    let store = mnemonic_core::storage::SqliteStore::open(temp.path()).unwrap();
    assert_eq!(
        store
            .recall_signed_a2a(
                v["reader"]["pubkey_base58"].as_str().unwrap(),
                "fixture-ctx",
                None,
                Some(true),
                100
            )
            .unwrap()
            .len(),
        1
    );
    let author = Keypair::new_from_array([1; 32]);
    let signed = prepare_signed_a2a(
        &author,
        "message",
        v["payload"].clone(),
        "fixture-ctx",
        Some("missing-parent".into()),
        "2026-10-01T00:00:00Z",
        None,
    )
    .unwrap();
    let failed = mnemonic_core::codec::a2a::signed::verify_signed_a2a(&signed, None).unwrap();
    assert!(store.save_signed_a2a(&signed, &failed).is_err());
    assert_eq!(
        store
            .conn()
            .query_row("SELECT COUNT(*) FROM attestations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires built SDK and real WASM; run after building packages/sdk"]
async fn sdk_http_sealed_end_to_end() {
    let server = TestServer::builder().build();
    let v = vectors();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = server.app.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let output = tokio::process::Command::new("node")
        .arg(root.join("packages/sdk/scripts/test-sealed-a2a-e2e.mjs"))
        .env("A2A_TEST_URL", format!("http://{address}"))
        .env(
            "A2A_AUTHOR_JWT",
            server.mint_jwt(v["author"]["pubkey_base58"].as_str().unwrap()),
        )
        .env(
            "A2A_READER_JWT",
            server.mint_jwt(v["reader"]["pubkey_base58"].as_str().unwrap()),
        )
        .env(
            "A2A_OUTSIDER_JWT",
            server.mint_jwt(v["outsider"]["pubkey_base58"].as_str().unwrap()),
        )
        .output()
        .await
        .unwrap();
    task.abort();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let guard = server.state.store.lock().unwrap();
    let count: i64 = guard
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM attestations WHERE privacy='sealed' AND content=''",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 4);
    let embeddings: i64 = guard
        .conn()
        .query_row("SELECT COUNT(*) FROM attestation_embeddings", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(embeddings, 0);
}

#[tokio::test]
async fn valid_client_signature_reaches_paywall_and_invalid_never_does() {
    let server = TestServer::builder().payment_mode("x402").build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let valid = server
        .call_tool(
            Some(author),
            "mnemonic_attest_a2a",
            signed_args(&v, "sealed"),
        )
        .await;
    assert_eq!(valid.status, axum::http::StatusCode::PAYMENT_REQUIRED);
    let mut args = signed_args(&v, "sealed");
    args["signed"] = json!("00");
    let invalid = server
        .call_tool(Some(author), "mnemonic_attest_a2a", args)
        .await;
    assert_eq!(invalid.status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(server.attestation_count(author), 0);
    result(
        &server
            .call_tool(
                Some(author),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx"}),
            )
            .await,
    );
}

#[tokio::test]
async fn withdrawn_recipient_loses_recall_and_unrelated_parent_cannot_be_linked() {
    let server = TestServer::builder().build();
    let v = vectors();
    let author = v["author"]["pubkey_base58"].as_str().unwrap();
    let reader = v["reader"]["pubkey_base58"].as_str().unwrap();
    let parent = result(
        &server
            .call_tool(
                Some(author),
                "mnemonic_attest_a2a",
                signed_args(&v, "sealed"),
            )
            .await,
    );
    let outsider = Keypair::new_from_array([3; 32]);
    let signed = prepare_signed_a2a(
        &outsider,
        "message",
        v["payload"].clone(),
        "fixture-ctx",
        Some(parent["attestation_id"].as_str().unwrap().into()),
        "2026-10-01T00:00:00Z",
        None,
    )
    .unwrap();
    let response=server.call_tool(Some(&outsider.pubkey().to_string()),"mnemonic_attest_a2a",json!({"kind":"message","context_id":"fixture-ctx","signed":hex::encode(signed),"prev_id":parent["attestation_id"]})).await;
    assert!(response.envelope["error"].is_object());
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute(
            "UPDATE grants SET withdrawn_at='2026-10-01T00:00:01Z' WHERE reader_kid=?1",
            [reader],
        )
        .unwrap();
    let rows = result(
        &server
            .call_tool(
                Some(reader),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx"}),
            )
            .await,
    );
    assert!(rows["attestations"].as_array().unwrap().is_empty());
    let own = result(
        &server
            .call_tool(
                Some(author),
                "mnemonic_recall_a2a",
                json!({"context_id":"fixture-ctx"}),
            )
            .await,
    );
    assert_eq!(own["attestations"].as_array().unwrap().len(), 1);
}
