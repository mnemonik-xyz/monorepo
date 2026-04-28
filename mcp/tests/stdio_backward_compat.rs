//! Integration test: stdio transport backward compatibility (Task 8 #4 /
//! tech-spec line 229).
//!
//! Spawns the pre-built `mnemonic-mcp --transport stdio` binary (via the
//! `CARGO_BIN_EXE_mnemonic-mcp` env var Cargo provides to integration
//! tests, NOT `cargo run` — that has a build-vs-run race when the binary is
//! already cached).
//!
//! Pipes line-delimited JSON-RPC requests through stdin and reads
//! responses from stdout, each request/response wrapped in
//! `tokio::time::timeout(5s)`. Asserts the trio (`initialize`,
//! `tools/list`, then a non-state-touching request) round-trips
//! cleanly without OAuth — the stdio path is single-tenant and trusted
//! per Decision 9.
//!
//! NB: The binary needs an embedder. `EMBED_PROVIDER=fastembed` requires
//! the `local-embed` feature (which downloads a ~22MB ONNX model). For CI
//! determinism we use `EMBED_PROVIDER=openai` with `OPENAI_API_KEY=test`;
//! `OpenAIEmbedder` accepts any non-empty key at construction and only
//! hits the network when an `embed()` is invoked. `tools/list` and
//! `initialize` never embed, so the network is never touched.
//!
//! The test is `#[ignore]`d when not running in an environment that has
//! the binary built against the test toolchain (i.e. when the
//! `CARGO_BIN_EXE_mnemonic-mcp` env var is unset at compile time, which
//! shouldn't happen for `cargo test` but is defensive).

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const REQ_TIMEOUT: Duration = Duration::from_secs(5);
/// Total budget for the test — startup + 3 requests.
///
/// Startup performs: keypair load, embedder init (instant for OpenAI mock
/// key), pricing engine refresh (2 × HTTP calls, 10s timeout each), and
/// RAG seeding (parses whitepaper, signs ~10–30 chunks via OpenAI; with a
/// fake key each `embed` fails fast, but the loop still iterates).
///
/// Worst case ~30s on a slow runner. We pick 45s for headroom.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);

/// **Network-dependent** — this test spawns the full `mnemonic-mcp` binary,
/// whose `pricing.refresh().await` at startup makes outbound HTTPS calls
/// to `uploader.irys.xyz` and `api.coingecko.com` (both with 10s
/// per-request reqwest timeouts). On a sandboxed runner with no internet,
/// startup blocks for the full ~20s before falling through to `run_stdio`,
/// which exceeds CI's preferred per-test budget.
///
/// We `#[ignore]` by default and run via `cargo test --features
/// test-support -- --ignored stdio_backward_compat` in the dedicated
/// network-allowed CI job (see `.github/workflows/ci.yml::test-stdio` —
/// added by Task 8). Local devs who want to verify can run the same.
///
/// Compile-only coverage: this file always compiles (proves the binary's
/// public surface — `CARGO_BIN_EXE_mnemonic-mcp`, env contract, JSON-RPC
/// shape — stays buildable). The functional assertion only runs with
/// `--ignored`.
#[tokio::test]
#[ignore = "spawns mnemonic-mcp binary; pricing.refresh() needs internet (run with --ignored in CI)"]
async fn test_stdio_tools_list_sign_memory_recall_without_oauth() {
    let bin = env!("CARGO_BIN_EXE_mnemonic-mcp");

    // Per-test scratch dirs so the binary doesn't touch the developer's
    // ~/.mnemonic state.
    let tmp = tempfile::tempdir().expect("tempdir");
    let keypair_path = tmp.path().join("id.json");
    let db_path = tmp.path().join("attestations.db");
    let rag_dir = tmp.path().join("rag");
    std::fs::create_dir_all(&rag_dir).expect("rag dir");

    // Pre-create the keypair AND seed one attestation row for the keypair's
    // pubkey so the binary's `seed::run` finds `count > 0` and skips the
    // (slow + network-touching) RAG bootstrap loop.
    {
        use mnemonic_core::identity::{load_or_create_keypair, pubkey_base58};
        use mnemonic_core::storage::{AttestationStore, SqliteStore};
        let kp = load_or_create_keypair(&keypair_path).expect("keypair");
        let pubkey = pubkey_base58(&kp);
        let store = SqliteStore::open(&db_path).expect("sqlite open");
        store
            .save_attestation(
                "stdio-test-seed",
                "seed-content",
                "seed-hash",
                &["seed".to_string()],
                "local:seed-sol",
                "local:seed-ar",
                &pubkey,
                &pubkey,
                &chrono::Utc::now().to_rfc3339(),
                &[0.0; 8],
            )
            .expect("save_attestation");
    }

    let mut cmd = Command::new(bin);
    cmd.arg("--transport")
        .arg("stdio")
        .env("STORAGE_MODE", "local")
        .env("PAYMENT_MODE", "none")
        // OpenAIEmbedder accepts any non-empty key at construction; we never
        // call embed() in this test (tools/list + initialize don't embed).
        .env("EMBED_PROVIDER", "openai")
        .env("OPENAI_API_KEY", "stdio-test-key-not-real")
        .env("MNEMONIC_KEYPAIR_PATH", &keypair_path)
        .env("DATABASE_PATH", &db_path)
        .env("RAG_CHUNK_DIR", &rag_dir)
        .env("OLLAMA_URL", "http://localhost:11434")
        .env("LLM_PROVIDER", "ollama")
        .env("LLM_API_URL", "http://localhost:11434")
        // tracing logs go to stderr (default tracing-subscriber target), so
        // turning info on here doesn't pollute the stdout JSON-RPC framing.
        // The stderr drain task captures these for diagnostics on failure.
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn mnemonic-mcp");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let mut writer = stdin;
    let mut reader = BufReader::new(stdout).lines();

    // Spawn a stderr-drain task — startup logs go here. Capture for test
    // diagnostics only; never blocks the test.
    let stderr_capture = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    {
        let cap = stderr_capture.clone();
        tokio::spawn(async move {
            let mut elines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = elines.next_line().await {
                let mut g = cap.lock().await;
                g.push_str(&l);
                g.push('\n');
            }
        });
    }

    // Helper: send a JSON-RPC line, await one response line that parses as
    // JSON. The mcp binary's `tracing_subscriber::fmt::init()` default
    // emits log lines to stdout interleaved with the JSON-RPC frames, so
    // we skip any line that does not start with `{` (the JSON-RPC framing
    // is always a single-line object). The caller wraps this in
    // `tokio::time::timeout` so we do NOT inner-timeout here.
    async fn round_trip(
        writer: &mut tokio::process::ChildStdin,
        reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let line = serde_json::to_string(&body).expect("serialize");
        writer.write_all(line.as_bytes()).await.expect("write");
        writer.write_all(b"\n").await.expect("newline");
        writer.flush().await.expect("flush");
        loop {
            let line = reader
                .next_line()
                .await
                .expect("read line")
                .expect("non-empty line");
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                return serde_json::from_str(trimmed).expect("parse response JSON");
            }
            // Non-JSON line (likely a tracing log on stdout) — skip.
        }
    }

    // Wrap each request in STARTUP_TIMEOUT for the first (server may still
    // be booting + price-refreshing) and REQ_TIMEOUT for subsequent ones.
    macro_rules! must_round_trip {
        ($timeout:expr, $body:expr, $label:literal) => {{
            let result = timeout($timeout, round_trip(&mut writer, &mut reader, $body)).await;
            match result {
                Ok(v) => v,
                Err(e) => {
                    let logs = stderr_capture.lock().await.clone();
                    let _ = child.kill().await;
                    panic!("{} timeout: {e:?}\n--- stderr ---\n{}", $label, logs);
                }
            }
        }};
    }

    // 1. initialize — must round-trip even before any tool call.
    let resp1 = must_round_trip!(
        STARTUP_TIMEOUT,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialize", "id": 1}),
        "initialize"
    );
    assert_eq!(resp1["jsonrpc"], "2.0");
    assert_eq!(resp1["id"], 1);
    assert!(
        resp1["result"]["protocolVersion"].is_string(),
        "initialize missing protocolVersion: {resp1}"
    );

    // 2. tools/list — exactly 5 tools.
    let resp2 = must_round_trip!(
        REQ_TIMEOUT,
        serde_json::json!({"jsonrpc": "2.0", "method": "tools/list", "id": 2}),
        "tools/list"
    );
    let tools = resp2["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 5, "expected 5 tools, got: {resp2}");

    // 3. mnemonic_whoami — exercises an actual tool that doesn't need
    //    embedding (proves the dispatcher is wired). `whoami` reads the
    //    keypair only.
    let resp3 = must_round_trip!(
        REQ_TIMEOUT,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "mnemonic_whoami", "arguments": {}},
            "id": 3,
        }),
        "tools/call mnemonic_whoami"
    );
    assert!(
        resp3["result"].is_object(),
        "whoami should return a result: {resp3}"
    );

    // Clean shutdown: drop stdin, wait briefly, then kill if needed.
    drop(writer);
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    let _ = child.kill().await;
}
