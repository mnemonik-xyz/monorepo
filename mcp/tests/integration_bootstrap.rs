//! Integration test: invisible-identity bootstrap on a fresh HOME (Task 4).
//!
//! Spawns `mnemonic-mcp --transport stdio` with a clean `HOME` tempdir, sends
//! `initialize` + `mnemonic_whoami`, then asserts:
//! 1. `~/.mnemonic/identity.json` was created.
//! 2. The pubkey in that file matches the pubkey returned by `mnemonic_whoami`.
//!
//! The test is `#[ignore]`d by default — same reason as `stdio_backward_compat`:
//! `pricing.refresh()` makes outbound HTTPS calls at startup which block on
//! sandboxed runners. Run with `cargo test -- --ignored integration_bootstrap`
//! in the network-allowed CI job or locally.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const REQ_TIMEOUT: Duration = Duration::from_secs(5);

async fn write_line(writer: &mut tokio::process::ChildStdin, body: serde_json::Value) {
    let line = serde_json::to_string(&body).expect("serialize");
    writer.write_all(line.as_bytes()).await.expect("write");
    writer.write_all(b"\n").await.expect("newline");
    writer.flush().await.expect("flush");
}

async fn read_line_with_timeout(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    dur: Duration,
) -> serde_json::Value {
    timeout(dur, async {
        loop {
            let line = reader
                .next_line()
                .await
                .expect("read line")
                .expect("non-empty line");
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                return serde_json::from_str(trimmed).expect("parse JSON");
            }
        }
    })
    .await
    .expect("read_line_with_timeout: timed out")
}

#[tokio::test]
#[ignore = "spawns mnemonic-mcp binary; pricing.refresh() needs internet (run with --ignored in CI)"]
async fn test_bootstrap_creates_identity_on_fresh_home() {
    let bin = env!("CARGO_BIN_EXE_mnemonic-mcp");

    let tmp = tempfile::tempdir().expect("tempdir");
    let home_dir = tmp.path().to_path_buf();
    let db_path = home_dir.join("attestations.db");

    // Determine PATH so the binary can find system tools it may exec.
    let path_env = std::env::var("PATH").unwrap_or_default();

    let mut cmd = Command::new(bin);
    cmd.arg("--transport")
        .arg("stdio")
        .env_clear()
        .env("HOME", &home_dir)
        .env("PATH", &path_env)
        .env("STORAGE_MODE", "local")
        .env("PAYMENT_MODE", "none")
        .env("EMBED_PROVIDER", "openai")
        .env("OPENAI_API_KEY", "bootstrap-test-key-not-real")
        .env("DATABASE_PATH", &db_path)
        .env("MNEMONIC_QUIET", "1")
        .env("RUST_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn mnemonic-mcp");
    let mut writer = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let mut reader = BufReader::new(stdout).lines();

    // Drain stderr so the process doesn't block on a full pipe.
    let stderr_capture = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    {
        let cap = stderr_capture.clone();
        tokio::spawn(async move {
            let mut elines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = elines.next_line().await {
                cap.lock().await.push_str(&l);
                cap.lock().await.push('\n');
            }
        });
    }

    // 1. initialize
    write_line(
        &mut writer,
        serde_json::json!({"jsonrpc": "2.0", "method": "initialize", "id": 1}),
    )
    .await;
    let _init_resp = read_line_with_timeout(&mut reader, STARTUP_TIMEOUT).await;

    // 2. mnemonic_whoami
    write_line(
        &mut writer,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "mnemonic_whoami", "arguments": {}}
        }),
    )
    .await;
    let whoami_resp = read_line_with_timeout(&mut reader, REQ_TIMEOUT).await;

    let _ = child.kill().await;

    // Assert identity.json was created.
    let identity_path = home_dir.join(".mnemonic").join("identity.json");
    assert!(
        identity_path.exists(),
        "identity.json not created at {identity_path:?}"
    );

    // Parse pubkey from identity.json.
    let identity_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&identity_path).expect("read identity"))
            .expect("parse identity json");
    let file_pubkey = identity_json["pubkey_base58"]
        .as_str()
        .expect("pubkey_base58 in identity.json");

    // Assert whoami returned the same pubkey.
    let text = whoami_resp["result"]["content"][0]["text"]
        .as_str()
        .expect("whoami text content");
    let whoami_json: serde_json::Value = serde_json::from_str(text).expect("parse whoami json");
    let whoami_pubkey = whoami_json["pubkey_base58"]
        .as_str()
        .expect("pubkey_base58 in whoami response");

    assert_eq!(
        file_pubkey, whoami_pubkey,
        "identity.json pubkey does not match mnemonic_whoami response"
    );
}
