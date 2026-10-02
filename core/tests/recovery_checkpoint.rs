//! Client-local exact-byte reconstruction and independently reported sources.
use mnemonic_core::{
    codec::{
        canonical::to_canonical_cbor,
        schema,
        sign::{sign_artifact, sign_cose},
    },
    identity,
    rebuild::recover_memory,
    sealed::{seal_memory, x25519_secret_from_solana_keypair},
};
use solana_sdk::signature::{Keypair, Signer};
fn memory(kp: &Keypair) -> serde_json::Value {
    serde_json::json!({"artifact_id":"memory-1","type":"memory","schema_version":1,"content":"complete private text 🧠","producer":identity::did_sol(kp),"created_at":"2026-10-02T00:00:00+00:00","tags":["exact"]})
}
#[test]
fn exact_plain_and_sealed_restore_requires_pinned_author_and_key() {
    let kp = Keypair::new();
    let author = kp.pubkey().to_string();
    let m = memory(&kp);
    let plain = sign_artifact(&m, &schema::MEMORY_V1, &kp).unwrap();
    let recovered = recover_memory(&plain.cose_bytes, &author, None).unwrap();
    assert_eq!(recovered.memory, m);
    assert_eq!(recovered.envelope, plain.cose_bytes);
    assert_eq!(recovered.plaintext, plain.canonical_cbor);
    let inner = to_canonical_cbor(&m, &schema::MEMORY_V1).unwrap();
    let sealed = seal_memory(
        &inner,
        &kp.pubkey().to_bytes(),
        "memory-1",
        &identity::did_sol(&kp),
        "2026-10-02T00:00:00+00:00",
        &mut rand::rngs::OsRng,
    )
    .unwrap();
    let envelope = sign_cose(&sealed.outer_cbor, &kp).unwrap();
    let key = x25519_secret_from_solana_keypair(&kp);
    let recovered = recover_memory(&envelope, &author, Some(&key)).unwrap();
    assert!(recovered.sealed);
    assert_eq!(recovered.memory, m);
    assert_eq!(recovered.plaintext, inner);
    assert_eq!(recovered.envelope, envelope);
    assert_eq!(
        recover_memory(&envelope, &author, None).unwrap_err().code,
        "key_required"
    );
    assert_eq!(
        recover_memory(&envelope, &author, Some(&[7; 32]))
            .unwrap_err()
            .code,
        "open_failed"
    );
    assert_eq!(
        recover_memory(&envelope, &Keypair::new().pubkey().to_string(), Some(&key))
            .unwrap_err()
            .code,
        "untrusted_author"
    );
    let mut corrupt = envelope;
    let end = corrupt.len() - 1;
    corrupt[end] ^= 1;
    assert!(recover_memory(&corrupt, &author, Some(&key)).is_err());
}
#[test]
fn unknown_signed_kind_and_forged_producer_are_explicit() {
    let kp = Keypair::new();
    let author = kp.pubkey().to_string();
    let mut m = memory(&kp);
    m["type"] = serde_json::json!("future.kind");
    // Sign arbitrary valid CBOR directly: schema dispatch must reject it after signature verification.
    let mut cbor = Vec::new();
    ciborium::into_writer(&m, &mut cbor).unwrap();
    let envelope = sign_cose(&cbor, &kp).unwrap();
    assert_eq!(
        recover_memory(&envelope, &author, None).unwrap_err().code,
        "unsupported_kind"
    );
    m = memory(&kp);
    m["producer"] = serde_json::json!(identity::did_sol(&Keypair::new()));
    let envelope = sign_artifact(&m, &schema::MEMORY_V1, &kp).unwrap();
    assert_eq!(
        recover_memory(&envelope.cose_bytes, &author, None)
            .unwrap_err()
            .code,
        "author_binding"
    );
}
#[tokio::test]
async fn failed_sources_differ_from_empty_and_preserve_other_source_items() {
    use httpmock::prelude::*;
    use mnemonic_core::{
        arweave::graphql::GraphQlClient, restore::enumerate_anchored, solana::SolanaClient,
    };
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/rpc");
        then.status(503);
    });
    let index=server.mock(|when,then|{when.method(POST).path("/graphql");then.status(200).json_body(serde_json::json!({"data":{"transactions":{"edges":[{"cursor":"cursor1","node":{"id":"A".repeat(43),"tags":[],"block":null}}],"pageInfo":{"hasNextPage":false}}}}));});
    let report = enumerate_anchored(
        &GraphQlClient::new(&server.url("/graphql")),
        &SolanaClient::new(&server.url("/rpc")),
        "owner",
        &[],
    )
    .await;
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.sources[0].status, "failed");
    assert_eq!(report.sources[1].status, "exhausted");
    index.assert();
    let report = enumerate_anchored(
        &GraphQlClient::new(&server.url("/fail")),
        &SolanaClient::new(&server.url("/rpc")),
        "owner",
        &[],
    )
    .await;
    assert!(report.items.is_empty());
    assert!(report
        .sources
        .iter()
        .all(|s| s.status == "failed" && s.error.is_some()));
}

#[test]
fn sdk_legacy_json_plaintext_is_preserved_with_outer_identity() {
    let kp = Keypair::new();
    let author = kp.pubkey().to_string();
    let plain = br#"{"type":"memory","content":"SDK JSON","tags":["legacy"]}"#;
    let sealed = seal_memory(
        plain,
        &kp.pubkey().to_bytes(),
        "sdk-memory",
        &identity::did_sol(&kp),
        "2026-10-02T00:00:00Z",
        &mut rand::rngs::OsRng,
    )
    .unwrap();
    let envelope = sign_cose(&sealed.outer_cbor, &kp).unwrap();
    let recovered = recover_memory(
        &envelope,
        &author,
        Some(&x25519_secret_from_solana_keypair(&kp)),
    )
    .unwrap();
    assert_eq!(recovered.plaintext, plain);
    assert_eq!(recovered.artifact_id, "sdk-memory");
    assert_eq!(recovered.author, author);
    assert_eq!(recovered.memory["content"], "SDK JSON");
}

#[tokio::test]
async fn source_budget_and_late_failure_preserve_fetched_pages() {
    use httpmock::prelude::*;
    use mnemonic_core::arweave::graphql::GraphQlClient;
    let server = MockServer::start();
    server.mock(|when,then|{when.method(POST).path("/graphql").body_includes("\"after\":null");then.status(200).json_body(serde_json::json!({"data":{"transactions":{"edges":[{"cursor":"next","node":{"id":"A".repeat(43),"tags":[],"block":null}}],"pageInfo":{"hasNextPage":true}}}}));});
    server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .body_includes("\"after\":\"next\"");
        then.status(503);
    });
    let gql = GraphQlClient::new(&server.url("/graphql"));
    let limited = gql.scan_anchored(&[], 1).await;
    assert!(limited.budget_exhausted);
    assert!(!limited.exhausted);
    assert_eq!(limited.items.len(), 1);
    let partial = gql.scan_anchored(&[], 2).await;
    assert!(!partial.budget_exhausted);
    assert!(!partial.exhausted);
    assert_eq!(partial.items.len(), 1);
    assert!(partial.error.is_some());
}

#[test]
fn memory_graph_reports_forks_missing_parents_cycles_and_unpinned_scope() {
    use mnemonic_core::rebuild::{recover_memory_set, MemoryRecoveryHead};
    let kp = Keypair::new();
    let author = kp.pubkey().to_string();
    let sign = |id: &str, parents: Vec<&str>| {
        let mut m = memory(&kp);
        m["artifact_id"] = serde_json::json!(id);
        m["parents"] = serde_json::json!(parents
            .into_iter()
            .map(|id| serde_json::json!({"artifact_id":id}))
            .collect::<Vec<_>>());
        sign_artifact(&m, &schema::MEMORY_V1, &kp)
            .unwrap()
            .cose_bytes
    };
    let root = sign("root", vec![]);
    let left = sign("left", vec!["root"]);
    let right = sign("right", vec!["root"]);
    let pin = |id: &str, bytes: &[u8]| MemoryRecoveryHead {
        artifact_id: id.into(),
        envelope_digest: mnemonic_core::codec::hash::hash_bytes(bytes),
        author: author.clone(),
    };
    let report = recover_memory_set(
        &[(&root, &author), (&left, &author), (&right, &author)],
        None,
        &[pin("left", &left), pin("right", &right)],
        &[pin("root", &root)],
    );
    assert!(report.complete_to_heads);
    assert_eq!(report.forks, vec!["root"]);
    assert_eq!(report.memories.len(), 3);
    let report = recover_memory_set(&[(&left, &author)], None, &[pin("left", &left)], &[]);
    assert!(!report.complete_to_heads);
    assert_eq!(report.missing_parents, vec!["root"]);
    assert!(!recover_memory_set(&[(&root, &author)], None, &[], &[]).complete_to_heads);
    assert!(
        !recover_memory_set(
            &[(&root, &author), (&left, &author)],
            None,
            &[pin("left", &left)],
            &[]
        )
        .complete_to_heads
    );
    let substituted = sign("root", vec!["unseen-parent"]);
    assert!(
        !recover_memory_set(&[(&root, &author)], None, &[pin("root", &substituted)], &[])
            .complete_to_heads
    );
    let a = sign("a", vec!["b"]);
    let b = sign("b", vec!["a"]);
    let report = recover_memory_set(
        &[(&a, &author), (&b, &author)],
        None,
        &[pin("a", &a)],
        &[pin("b", &b)],
    );
    assert!(!report.complete_to_heads);
    assert!(report.memories.is_empty());
}
