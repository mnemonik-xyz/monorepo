//! Real HTTP dispatch and original signed memory bytes; external storage is mocked.
mod _helpers;
use _helpers::TestServer;
use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http_body_util::BodyExt;
use mnemonic_core::codec::sign::{sign_cose, verify_artifact};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_sdk::signature::{Keypair, Signer};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tower::ServiceExt;
#[derive(Clone, Default)]
struct Remote {
    blobs: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    fail_upload: Arc<AtomicBool>,
}
async fn upload(
    State(s): State<Remote>,
    bytes: Bytes,
) -> Result<Json<Value>, axum::http::StatusCode> {
    if s.fail_upload.load(Ordering::SeqCst) {
        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
    let id = URL_SAFE_NO_PAD.encode(Sha256::digest(&bytes[2..66]));
    let tags = u64::from_le_bytes(bytes[108..116].try_into().unwrap()) as usize;
    let original = bytes[116 + tags..].to_vec();
    assert!(verify_artifact(&original, None).unwrap().valid);
    s.blobs.lock().unwrap().insert(id.clone(), original);
    Ok(Json(json!({"id":id})))
}
async fn read(
    State(s): State<Remote>,
    Path(id): Path<String>,
) -> Result<Vec<u8>, axum::http::StatusCode> {
    s.blobs
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}
async fn remote() -> (String, Remote, tokio::task::JoinHandle<()>) {
    let s = Remote::default();
    let app = Router::new()
        .route("/upload", post(upload))
        .route("/{id}", get(read))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, s, task)
}
fn sealed(kp: &Keypair) -> Vec<u8> {
    let s = mnemonic_core::sealed::seal_memory(
        br#"{"type":"memory","content":"client secret"}"#,
        &kp.pubkey().to_bytes(),
        "art:test",
        &format!("did:sol:{}", kp.pubkey()),
        "2026-10-02T00:00:00Z",
        &mut rand::rngs::OsRng,
    )
    .unwrap();
    sign_cose(&s.outer_cbor, kp).unwrap()
}
async fn submit(server: &TestServer, owner: &str, path: &str, bytes: Vec<u8>) -> (u16, Value) {
    let response = server
        .app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header(
                    "authorization",
                    format!("Bearer {}", server.mint_jwt(owner)),
                )
                .header("content-type", "application/cbor")
                .body(axum::body::Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
fn assert_metadata_only(server: &TestServer) {
    let store = server.state.store.lock().unwrap();
    for table in ["attestations", "attestation_embeddings", "grants"] {
        let n: i64 = store
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}
// Evidence is optional and contains only declared observations, never request bodies,
// author keys, payment proofs or signed envelopes. Invalidate success before setup.
fn evidence_path(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("MNEMONIC_DEMO_FINANCIAL_EVIDENCE_DIR")?;
    let path = std::path::PathBuf::from(dir).join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("cannot invalidate financial evidence: {e}"),
    }
    Some(path)
}
fn write_evidence(path: Option<std::path::PathBuf>, evidence: Value) {
    if let Some(path) = path {
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        std::fs::rename(temporary, path).unwrap();
    }
}
#[tokio::test]
async fn exact_delivery_retry_and_receipt_failure_are_independent() {
    let evidence = evidence_path("receipt-failure.json");
    let (url, s, task) = remote().await;
    let server = TestServer::builder()
        .storage_mode("full")
        .arweave_gateway(url)
        .build();
    let kp = Keypair::new();
    let bytes = sealed(&kp);
    let owner = kp.pubkey().to_string();
    let (status, a) = submit(&server, &owner, "/api/ingest-artifact", bytes.clone()).await;
    assert_eq!(status, 200, "{a}");
    assert_eq!(a["delivery_status"], "verified");
    assert_eq!(a["receipt_persisted"], true);
    assert_eq!(
        s.blobs.lock().unwrap()[a["arweave_tx"].as_str().unwrap()],
        bytes
    );
    mnemonic_mcp::tools::perform_delivery_check(
        &server.state.arweave,
        a["arweave_tx"].as_str().unwrap(),
        a["content_hash"].as_str().unwrap(),
        &owner,
        &bytes,
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert!(mnemonic_mcp::tools::perform_delivery_check(
        &server.state.arweave,
        a["arweave_tx"].as_str().unwrap(),
        a["content_hash"].as_str().unwrap(),
        &Keypair::new().pubkey().to_string(),
        &bytes,
        std::time::Duration::from_secs(1)
    )
    .await
    .is_err());
    let mut altered = bytes.clone();
    altered[0] ^= 1;
    assert!(mnemonic_mcp::tools::perform_delivery_check(
        &server.state.arweave,
        a["arweave_tx"].as_str().unwrap(),
        a["content_hash"].as_str().unwrap(),
        &owner,
        &altered,
        std::time::Duration::from_secs(1)
    )
    .await
    .is_err());
    // A read-only receipt database must not demote verified external delivery.
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    let (status, b) = submit(&server, &owner, "/api/anchor-sealed", bytes).await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["delivery_status"], "verified");
    assert_eq!(b["receipt_persisted"], false);
    assert_eq!(b["payment_status"], "not_required");
    assert_eq!(a["locator"], b["locator"]);
    assert_eq!(s.blobs.lock().unwrap().len(), 1);
    assert_metadata_only(&server);
    task.abort();
    write_evidence(
        evidence,
        json!({
            "version":1,"environment":"local_mock_services","scenario":"receipt_failure",
            "failures":[{"name":"receipt_database_read_only","status":"recovered","detail":"Exact external bytes remained verified while receipt persistence failed; same locator and one stored blob."}],
            "billing":[{"scenario":"verified_delivery_receipt_write_failure","paymentStatus":"not_required","deliveryStatus":"verified","receiptPersisted":false,"detail":"Unpaid fixture; PRAGMA query_only rejects receipt writes after verified external delivery."}]
        }),
    );
}
#[tokio::test]
async fn invalid_author_envelope_and_local_fail_before_upload_or_payment() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .arweave_gateway(url)
        .build();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    for bytes in [vec![0], sealed(&Keypair::new())] {
        let (status, _) = submit(&server, &owner, "/api/ingest-artifact", bytes).await;
        assert_eq!(status, 400);
    }
    let (status, _) = submit(&server, &owner, "/api/store-sealed", sealed(&kp)).await;
    assert_eq!(status, 410);
    let (status, body) = submit(&server, &owner, "/api/ingest-artifact", sealed(&kp)).await;
    assert_eq!(status, 503, "{body}");
    assert!(s.blobs.lock().unwrap().is_empty());
    assert_metadata_only(&server);
    task.abort();
}
#[tokio::test]
async fn tampered_external_bytes_are_not_verified() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder()
        .storage_mode("full")
        .arweave_gateway(url)
        .build();
    let kp = Keypair::new();
    let bytes = sealed(&kp);
    let owner = kp.pubkey().to_string();
    let (_, a) = submit(&server, &owner, "/api/ingest-artifact", bytes.clone()).await;
    s.blobs
        .lock()
        .unwrap()
        .get_mut(a["arweave_tx"].as_str().unwrap())
        .unwrap()[0] ^= 1;
    let (status, _) = submit(&server, &owner, "/api/ingest-artifact", bytes).await;
    assert_eq!(status, 502);
    task.abort();
}
#[tokio::test]
#[ignore = "requires built SDK and real WASM; run explicitly"]
async fn sdk_wasm_real_http_private_ingestion() {
    let (url, s, task) = remote().await;
    let server = TestServer::builder()
        .storage_mode("full")
        .arweave_gateway(url.clone())
        .build();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = server.app.clone();
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let fixture: Value = serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap();
    let script = r#"
      import assert from 'node:assert/strict';
      import {MnemonicClient,Keypair,LocalSigner} from './packages/sdk/dist/index.js';
      const kp=new Keypair(JSON.parse(process.env.INGEST_KEY));
      let requestBody;
      const client=new MnemonicClient({baseUrl:process.env.INGEST_URL,jwt:process.env.INGEST_JWT,signer:new LocalSigner(kp),a2aGatewayUrl:process.env.INGEST_GATEWAY,fetch:async(url,init)=>{if(String(url).includes('/api/ingest-artifact')){requestBody=new Uint8Array(init.body);assert.equal(init.headers['Content-Type'],'application/cbor');}return fetch(url,init);}});
      client.setKeypair(kp);
      const saved=await client.sealMemory('private sdk wasm secret',{mode:'anchor'});
      assert.equal(saved.receipt.delivery_status,'verified');
      assert.deepEqual(requestBody,saved.signedBytes);
      assert(!Buffer.from(requestBody).includes(Buffer.from('private sdk wasm secret')));
      assert.equal((await client.openMemory(saved.outerCbor)).content,'private sdk wasm secret');
      const offline=new MnemonicClient({baseUrl:'https://offline.invalid',signer:new LocalSigner(kp),fetch:async()=>{throw Error('offline')}});offline.setKeypair(kp);
      const local=await offline.sealMemory('offline private',{mode:'store'});assert.equal((await offline.openMemory(local.memoryHash)).content,'offline private');
      assert.equal((await offline.openMemory(saved.outerCbor)).content,'private sdk wasm secret');
      const publicMemory=await client.preparePublicMemory('intentional public memory');
      await assert.rejects(client.ingestPreparedMemory(publicMemory.signedBytes),/HTTP 400/);
      const publicReceipt=await client.ingestPreparedMemory(publicMemory.signedBytes,{publicConsent:true});
      assert.equal(publicReceipt.content_hash,publicMemory.contentHash);

    "#;
    let result = tokio::process::Command::new("node")
        .args(["--input-type=module", "-e", script])
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap(),
        )
        .env("INGEST_KEY", fixture["author"].to_string())
        .env("INGEST_URL", endpoint)
        .env("INGEST_GATEWAY", url)
        .env(
            "INGEST_JWT",
            server.mint_jwt(fixture["author"]["pubkey_base58"].as_str().unwrap()),
        )
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(s.blobs.lock().unwrap().len(), 2);
    assert_metadata_only(&server);
    http.abort();
    task.abort();
}

#[tokio::test]
#[ignore = "requires built SDK; run explicitly"]
async fn sdk_http_recall_limit_and_evidence() {
    use mnemonic_core::storage::{AttestationStore, Visibility, WriteMode};
    let server = TestServer::builder().build();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap();
    let author = fixture["author"]["pubkey_base58"].as_str().unwrap();
    {
        let store = server.state.store.lock().unwrap();
        for i in 0..6 {
            store
                .save_attestation(
                    &format!("row-{i}"),
                    "recall test content",
                    &format!("{i:064x}"),
                    &[],
                    "local:s",
                    "local:a",
                    author,
                    author,
                    "2026-10-02T00:00:00Z",
                    WriteMode::Local,
                    Visibility::Private,
                    &[0.1; 8],
                )
                .unwrap();
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = server.app.clone();
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let script = r#"
      import assert from 'node:assert/strict';
      import {MnemonicClient,Keypair,LocalSigner} from './packages/sdk/dist/index.js';
      const kp=new Keypair(JSON.parse(process.env.INGEST_KEY));
      const client=new MnemonicClient({baseUrl:process.env.INGEST_URL,jwt:process.env.INGEST_JWT,signer:new LocalSigner(kp)});
      const result=await client.recall('recall test',{topK:1});
      assert.equal(result.hits.length,1);
      assert.equal(result.hits[0].signedAt,'2026-10-02T00:00:00Z');
      assert(result.evidence.merkle_commitment);
      assert.equal(result.total,result.evidence.total_attestations);
      assert.equal(result.hits[0].content_hash,result.evidence.results[0].content_hash);
    "#;
    let result = tokio::process::Command::new("node")
        .args(["--input-type=module", "-e", script])
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap(),
        )
        .env("INGEST_KEY", fixture["author"].to_string())
        .env("INGEST_URL", endpoint)
        .env("INGEST_JWT", server.mint_jwt(author))
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    http.abort();
}

async fn submit_operation(
    server: &TestServer,
    owner: &str,
    operation: &str,
    bytes: Vec<u8>,
) -> (u16, Value) {
    let response = server
        .app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/ingest-artifact")
                .header(
                    "authorization",
                    format!("Bearer {}", server.mint_jwt(owner)),
                )
                .header("content-type", "application/cbor")
                .header("x-mnemonic-operation-id", operation)
                .body(axum::body::Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
fn accepted_receipt(operation: &str) -> String {
    let payload = json!({"version":1,"service_id":"mock-provider","operation_id":operation,"scheme":"exact","binding_digest":"binding","payer_wallet":"wallet","amount":"1000","asset":"asset","network":"network","pay_to":"merchant","settlement_tx":"tx","settled_at":"2026-10-02T00:00:00Z"});
    let mut receipt = payload.clone();
    receipt.as_object_mut().unwrap().remove("version");
    receipt.as_object_mut().unwrap().remove("service_id");
    receipt["status"] = json!("settled");
    receipt["receipt"] = json!({"payload":payload,"signature":{"algorithm":"ed25519","key_id":"mock","value":"signature"}});
    receipt.to_string()
}
fn settled_operation(server: &TestServer, owner: &str, bytes: &[u8], operation: &str) -> String {
    use mnemonic_mcp::{delivery_operation as delivery, paid_artifact, paid_operation};
    let v = mnemonic_mcp::ingestion::validate_memory(bytes, owner).unwrap();
    let tags = [
        ("Mnemonic-Type", v.kind.as_str()),
        ("Producer", owner),
        ("Content-Hash", v.content_hash.as_str()),
    ];
    let locator = server
        .state
        .arweave
        .item_id(bytes, server.state.keypair.keypair().unwrap(), &tags)
        .unwrap();
    let store = server.state.store.lock().unwrap();
    let digest = paid_artifact::hash_client_signed_cose(bytes);
    delivery::bind(
        store.conn(),
        operation,
        owner,
        &digest,
        bytes.len(),
        "arweave",
        &locator,
        true,
        0,
    )
    .unwrap();
    paid_operation::create_or_get(
        store.conn(),
        paid_operation::NewPaidOperation {
            operation_id: operation,
            subject_hash: blake3::hash(owner.as_bytes()).to_hex().as_ref(),
            artifact_hash: &digest,
            created_at: "2026-10-02T00:00:00Z",
        },
    )
    .unwrap();
    paid_operation::record_quote(
        store.conn(),
        operation,
        "wallet",
        "binding",
        "quote",
        "2026-10-03T00:00:00Z",
        "2026-10-02T00:00:00Z",
    )
    .unwrap();
    paid_operation::record_provider_receipt(
        store.conn(),
        operation,
        &accepted_receipt(operation),
        "2026-10-02T00:00:00Z",
    )
    .unwrap();
    delivery::acquire(store.conn(), operation, "crashed-process", 0).unwrap();
    delivery::payment_settled(store.conn(), operation, "crashed-process", 1).unwrap();
    locator
}
#[tokio::test]
async fn settled_crash_resubmission_is_byte_free_and_concurrent_retries_keep_one_financial_operation(
) {
    let (url, remote, task) = remote().await;
    // No payment service exists: a settled retry must never need one.
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .arweave_gateway(url)
        .build();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    let bytes = sealed(&kp);
    let locator = settled_operation(&server, &owner, &bytes, "settled-retry");
    let (a, b) = tokio::join!(
        submit_operation(&server, &owner, "settled-retry", bytes.clone()),
        submit_operation(&server, &owner, "settled-retry", bytes.clone())
    );
    assert!([200, 409].contains(&a.0), "{a:?}");
    assert!([200, 409].contains(&b.0), "{b:?}");
    assert!(a.0 == 200 || b.0 == 200);
    assert_eq!(remote.blobs.lock().unwrap()[&locator], bytes);
    let (status, body) = submit_operation(&server, &owner, "settled-retry", sealed(&kp)).await;
    assert_eq!(status, 409, "{body}");
    let store = server.state.store.lock().unwrap();
    let financial: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM paid_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(financial, 1);
    let staged: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM paid_artifact_staging", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(staged, 0);
    assert_eq!(
        mnemonic_mcp::delivery_operation::get(store.conn(), "settled-retry")
            .unwrap()
            .unwrap()
            .payment_status,
        "settled"
    );
    task.abort();
}
#[tokio::test]
async fn upload_crash_reuses_known_locator_and_terminal_paid_failure_exposes_remedy() {
    let evidence = evidence_path("payment-failure.json");
    let (url, remote, task) = remote().await;
    let server = TestServer::builder()
        .storage_mode("full")
        .payment_mode("x402")
        .arweave_gateway(url)
        .build();
    let kp = Keypair::new();
    let owner = kp.pubkey().to_string();
    let bytes = sealed(&kp);
    let locator = settled_operation(&server, &owner, &bytes, "uploaded-crash");
    remote
        .blobs
        .lock()
        .unwrap()
        .insert(locator.clone(), bytes.clone()); // external success before process died
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    let (status, body) = submit_operation(&server, &owner, "uploaded-crash", bytes.clone()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["payment_status"], "settled");
    assert_eq!(body["delivery_status"], "verified");
    assert_eq!(body["receipt_persisted"], false);
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    let (status, reconciled) =
        submit_operation(&server, &owner, "uploaded-crash", bytes.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(reconciled["operation_persisted"], true);
    assert_eq!(
        mnemonic_mcp::delivery_operation::get(
            server.state.store.lock().unwrap().conn(),
            "uploaded-crash"
        )
        .unwrap()
        .unwrap()
        .delivery_status,
        "verified"
    );
    server.state.store.lock().unwrap().conn().execute_batch("DELETE FROM delivery_operations WHERE operation_id='uploaded-crash'; DELETE FROM paid_operations WHERE operation_id='uploaded-crash';").unwrap();
    let (status, lost) = submit_operation(&server, &owner, "uploaded-crash", bytes).await;
    assert_eq!(status, 200);
    assert_eq!(lost["payment_status"], "unknown");
    assert_eq!(lost["financial_reconciliation_required"], true);
    let failed = sealed(&kp);
    settled_operation(&server, &owner, &failed, "terminal");
    // A seeded settled operation fails external upload, then resumes with the
    // identical bytes and financial receipt. No payment provider is configured.
    remote.fail_upload.store(true, Ordering::SeqCst);
    let (status, pending) = submit_operation(&server, &owner, "terminal", failed.clone()).await;
    assert_eq!(status, 503, "{pending}");
    assert_eq!(pending["payment_status"], "settled");
    assert_eq!(pending["delivery_status"], "failed_retryable");
    assert!(pending.get("receipt_persisted").is_none());
    remote.fail_upload.store(false, Ordering::SeqCst);
    let (status, recovered) = submit_operation(&server, &owner, "terminal", failed.clone()).await;
    assert_eq!(status, 200, "{recovered}");
    assert_eq!(recovered["payment_status"], "settled");
    assert_eq!(recovered["delivery_status"], "verified");
    assert_eq!(recovered["receipt_persisted"], true);
    // Remove only mocked remote bytes to exercise the terminal-delivery boundary.
    remote.blobs.lock().unwrap().clear();
    server
        .state
        .store
        .lock()
        .unwrap()
        .conn()
        .execute(
            "UPDATE delivery_operations SET attempts=7 WHERE operation_id='terminal'",
            [],
        )
        .unwrap();
    task.abort(); // storage is now unavailable
    let (status, body) = submit_operation(&server, &owner, "terminal", failed.clone()).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["payment_status"], "remedy_pending");
    assert_eq!(body["delivery_status"], "failed_terminal");
    assert!(body.get("receipt_persisted").is_none());
    assert_eq!(
        body["retry_action"],
        "contact_operator_for_refund_or_credit"
    );
    let (status, repeated) = submit_operation(&server, &owner, "terminal", failed).await;
    assert_eq!(status, 409, "{repeated}");
    assert_eq!(repeated["payment_status"], "remedy_pending");
    assert_eq!(repeated["delivery_status"], "failed_terminal");
    let store = server.state.store.lock().unwrap();
    let (count, receipt): (i64, String) = store
        .conn()
        .query_row(
            "SELECT COUNT(*), provider_receipt_json FROM paid_operations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(receipt, accepted_receipt("terminal"));
    let op = mnemonic_mcp::delivery_operation::get(store.conn(), "terminal")
        .unwrap()
        .unwrap();
    assert_eq!(op.attempts, 8); // Terminal resubmission did not start another upload.
    drop(store);
    assert_metadata_only(&server);
    write_evidence(
        evidence,
        json!({
            "version":1,"environment":"local_mock_services","scenario":"settled_delivery_failure",
            "failures":[
                {"name":"settled_upload_failure","status":"recovered","detail":"Mock upload returned 503; identical resubmission delivered with the original seeded receipt and one financial operation."},
                {"name":"terminal_delivery_failure","status":"rejected","detail":"Attempt count seeded to seven, eighth failed upload entered remedy_pending; repeated request stayed terminal at eight."},
                {"name":"lost_financial_metadata","status":"passed","detail":"Exact external delivery remained verified while payment was unknown and reconciliation required."}
            ],
            "billing":[
                {"scenario":"settled_receipt_write_failure","paymentStatus":"settled","deliveryStatus":"verified","receiptPersisted":false,"detail":"Accepted payment receipt was seeded, not obtained from a live payment provider."},
                {"scenario":"settled_upload_failed","paymentStatus":"settled","deliveryStatus":"failed_retryable","receiptPersisted":false,"detail":"HTTP 503 after mocked upload failure; no new verified delivery receipt returned. Payment receipt was seeded and remains retained."},
            {"scenario":"settled_upload_retry","paymentStatus":"settled","deliveryStatus":"verified","receiptPersisted":true,"detail":"Mock storage failure recovered; one financial row and unchanged seeded receipt. No payment provider configured; no real settlement proven."},
                {"scenario":"terminal_remedy","paymentStatus":"remedy_pending","deliveryStatus":"failed_terminal","receiptPersisted":false,"detail":"No new verified delivery receipt returned; historical receipt retained. Remedy request visible, no refund or credit executed."}
            ]
        }),
    );
}

#[tokio::test]
async fn paid_a2a_uses_same_durable_operation_after_settlement_crash() {
    use mnemonic_mcp::{delivery_operation as delivery, paid_artifact, paid_operation};
    let (url, remote, task) = remote().await;
    let server = TestServer::builder()
        .payment_mode("x402")
        .arweave_gateway(url)
        .build();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap();
    let owner = fixture["author"]["pubkey_base58"].as_str().unwrap();
    let bytes = hex::decode(fixture["stream"].as_str().unwrap()).unwrap();
    let verified =
        mnemonic_core::codec::a2a::signed::verify_signed_a2a(&bytes, Some(owner)).unwrap();
    let digest = paid_artifact::hash_client_signed_cose(&bytes);
    let operation = format!(
        "artifact-{}",
        blake3::hash(format!("{owner}:{digest}").as_bytes()).to_hex()
    );
    let tags = [
        ("Mnemonic-Type", "a2a"),
        ("Producer", owner),
        ("Content-Hash", verified.content_hash.as_str()),
        ("Context-Id", verified.binding.context_id.as_str()),
    ];
    let locator = server
        .state
        .arweave
        .item_id(&bytes, server.state.keypair.keypair().unwrap(), &tags)
        .unwrap();
    {
        let store = server.state.store.lock().unwrap();
        delivery::bind(
            store.conn(),
            &operation,
            owner,
            &digest,
            bytes.len(),
            "arweave",
            &locator,
            true,
            0,
        )
        .unwrap();
        paid_operation::create_or_get(
            store.conn(),
            paid_operation::NewPaidOperation {
                operation_id: &operation,
                subject_hash: blake3::hash(owner.as_bytes()).to_hex().as_ref(),
                artifact_hash: &digest,
                created_at: "now",
            },
        )
        .unwrap();
        paid_operation::record_quote(
            store.conn(),
            &operation,
            "wallet",
            "binding",
            "quote",
            "later",
            "now",
        )
        .unwrap();
        paid_operation::record_provider_receipt(
            store.conn(),
            &operation,
            &accepted_receipt(&operation),
            "now",
        )
        .unwrap();
        // Crash before coordinator observed the financial receipt: it must recover
        // from paid_operations without contacting a second payment service.
    }
    let args = json!({"kind":"message","context_id":verified.binding.context_id,"sealed":true,"signed":fixture["stream"]});
    let (a, b) = tokio::join!(
        server.call_tool(Some(owner), "mnemonic_attest_a2a", args.clone()),
        server.call_tool(Some(owner), "mnemonic_attest_a2a", args)
    );
    assert!([200, 409].contains(&a.status.as_u16()), "{:?}", a.envelope);
    assert!([200, 409].contains(&b.status.as_u16()), "{:?}", b.envelope);
    assert!(a.status.is_success() || b.status.is_success());
    assert_eq!(remote.blobs.lock().unwrap()[&locator], bytes);
    let store = server.state.store.lock().unwrap();
    let op = delivery::get(store.conn(), &operation).unwrap().unwrap();
    assert_eq!(op.payment_status, "settled");
    assert_eq!(op.delivery_status, "verified");
    let count: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM paid_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    task.abort();
}
