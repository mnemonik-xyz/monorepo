//! Task 5 — agent-native-distribution CLI surface tests.
//!
//! Covers:
//! - `mnemonic-mcp mcp-stdio` accepts JSON-RPC on stdin and emits a
//!   well-formed `tools/list` response on stdout.
//! - `mnemonic-mcp logout` removes `~/.mnemonic/token.json` (sandboxed via
//!   `MNEMONIC_CONFIG_DIR`) and is idempotent when the file is absent.
//! - `MNEMONIC_HOSTED_ENDPOINT` is ignored unless `--allow-custom-endpoint`
//!   is passed AND emits the spec'd stderr warning when the flag is missing
//!   (Decision 12). Verified via in-process `resolved_hosted_endpoint` for
//!   the gating matrix AND via spawned-binary stderr capture for the
//!   end-to-end stderr-line guarantee.
//!
//! Why a mix of process-spawn vs in-process? The binary is the production
//! surface for the spec strings ("mnemonic: logged out", the env-var
//! warning line) so those tests spawn the binary. The flag-gating logic
//! itself is a pure function (`mnemonic_mcp::resolved_hosted_endpoint`)
//! and we cover the matrix exhaustively in-process — spawning a binary
//! per cell of the matrix would 4x the test wall-clock for no extra
//! coverage.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Test budget. Each spawned-binary test waits at most this long for the
/// first JSON-RPC response. With `PRICING_REFRESH_DISABLED=1` (no network
/// fetches) AND `EMBED_PROVIDER=stub-test` (no slow rotation-matrix build)
/// the binary boots in well under 1s; the 15s ceiling tolerates a heavily
/// contended CI box without inviting hangs.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const REQ_TIMEOUT: Duration = Duration::from_secs(5);

// ── In-process matrix for `resolved_hosted_endpoint` (Decision 12) ────────

#[test]
fn resolved_hosted_endpoint_default_wins_when_flag_unset() {
    // env var present + flag false → fall back to default, signal warn=true
    let (endpoint, should_warn) =
        mnemonic_mcp::resolved_hosted_endpoint(false, Some("http://attacker.example"));
    assert_eq!(endpoint, mnemonic_mcp::DEFAULT_HOSTED_ENDPOINT);
    assert!(
        should_warn,
        "warning required when env var present + flag absent"
    );
}

#[test]
fn resolved_hosted_endpoint_env_wins_when_flag_set() {
    let (endpoint, should_warn) =
        mnemonic_mcp::resolved_hosted_endpoint(true, Some("https://test.local/mcp"));
    assert_eq!(endpoint, "https://test.local/mcp");
    assert!(!should_warn, "flag-allowed env value emits no warning");
}

#[test]
fn resolved_hosted_endpoint_default_when_env_unset() {
    // No env var → default endpoint, no warn (whether or not flag is set).
    let (e1, w1) = mnemonic_mcp::resolved_hosted_endpoint(false, None);
    let (e2, w2) = mnemonic_mcp::resolved_hosted_endpoint(true, None);
    assert_eq!(e1, mnemonic_mcp::DEFAULT_HOSTED_ENDPOINT);
    assert_eq!(e2, mnemonic_mcp::DEFAULT_HOSTED_ENDPOINT);
    assert!(!w1 && !w2, "no env var means no warning");
}

#[test]
fn resolved_hosted_endpoint_empty_env_treated_as_unset() {
    // Empty string is ambiguous (set-but-blank). Treat as "no override" so a
    // shell snippet like `MNEMONIC_HOSTED_ENDPOINT= mnemonic-mcp ...` does
    // not silently surrender to the default with a warning that confuses
    // the operator. Falls through to the default with no warn.
    let (endpoint, should_warn) = mnemonic_mcp::resolved_hosted_endpoint(false, Some(""));
    assert_eq!(endpoint, mnemonic_mcp::DEFAULT_HOSTED_ENDPOINT);
    assert!(!should_warn);
}

// ── logout: file-removal + idempotency (sandboxed via MNEMONIC_CONFIG_DIR) ──

/// Drive the binary's `logout` subcommand against a temporary `$HOME`-like
/// directory by setting `MNEMONIC_CONFIG_DIR=<tmp>`. Returns the captured
/// stderr lines so tests can match against the spec'd "mnemonic: logged
/// out" string.
async fn run_logout(config_dir: &std::path::Path) -> (i32, String) {
    let bin = env!("CARGO_BIN_EXE_mnemonic-mcp");
    let output = Command::new(bin)
        .arg("logout")
        .env("MNEMONIC_CONFIG_DIR", config_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("spawn mnemonic-mcp logout");
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (code, stderr)
}

#[tokio::test]
async fn logout_removes_token_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let token_path = tmp.path().join("token.json");
    // Seed a well-formed TokenJson per packages/cli/src/config.ts:39-65.
    let token_body = serde_json::json!({
        "jwt": "test.jwt.value",
        "expires_at": "2030-01-01T00:00:00Z",
        "sub": "test-user-sub",
    });
    std::fs::write(&token_path, token_body.to_string()).expect("seed token file");
    assert!(token_path.exists(), "fixture seeded");

    let (code, stderr) = run_logout(tmp.path()).await;
    assert_eq!(code, 0, "logout exits 0 on success");
    assert!(
        stderr.contains("mnemonic: logged out"),
        "expected spec'd stderr line, got: {stderr:?}"
    );
    assert!(!token_path.exists(), "token file removed");
}

#[tokio::test]
async fn logout_is_idempotent_when_file_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let token_path = tmp.path().join("token.json");
    assert!(!token_path.exists(), "fixture: no token file present");

    let (code, stderr) = run_logout(tmp.path()).await;
    assert_eq!(code, 0, "logout exits 0 even without a token to remove");
    assert!(
        stderr.contains("mnemonic: logged out"),
        "stderr line fires uniformly per idempotency contract"
    );
    assert!(!token_path.exists());
}

// ── mcp-stdio subcommand: JSON-RPC on stdin → response on stdout ──────────

/// `mnemonic-mcp mcp-stdio` should expose the same JSON-RPC handler as
/// `mnemonic-mcp --transport stdio`. We drive a `tools/list` round-trip
/// and assert the response carries the post-Task-2 tool inventory (>= 5
/// tools — 5 pre-existing + new `request_public_write_confirmation`, see
/// Task 4). Test-reviewer F-2: assertion is `>= 5`, not `> 0`, so a
/// dispatcher regression returning a single dummy tool would fail.
///
/// Round-2 (test-reviewer F-1): the binary now honours
/// `PRICING_REFRESH_DISABLED=1` so startup stays offline; the test no
/// longer needs `#[ignore]`.
#[tokio::test]
async fn mcp_stdio_accepts_jsonrpc_on_stdin() {
    let bin = env!("CARGO_BIN_EXE_mnemonic-mcp");

    let tmp = tempfile::tempdir().expect("tempdir");
    // Build a fake HOME so the binary's `dirs::home_dir()` lookup resolves
    // into the tempdir; otherwise `core::identity::ensure()` would write
    // a fresh keypair to the developer's real `~/.mnemonic/identity.json`.
    let fake_home = tmp.path().to_path_buf();
    std::fs::create_dir_all(fake_home.join(".mnemonic")).expect("mnemonic dir");
    let keypair_path = fake_home.join(".mnemonic").join("identity.json");
    let db_path = tmp.path().join("attestations.db");
    let rag_dir = tmp.path().join("rag");
    std::fs::create_dir_all(&rag_dir).expect("rag dir");

    // Pre-seed identity + an attestation row so the binary boots into
    // `run_stdio` without invoking `ensure()` for a new identity AND
    // without the RAG bootstrap loop (which fires on first boot when the
    // owner has zero rows).
    {
        use mnemonic_core::identity::{ensure_with_stores, pubkey_base58, FileKeyStore, KeyStores};
        use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
        let stores = KeyStores {
            os: None,
            file: Box::new(FileKeyStore::new(keypair_path.clone())),
            identity_path: keypair_path.clone(),
            readme_path: fake_home.join(".mnemonic").join("README.txt"),
        };
        let id = ensure_with_stores(stores).expect("ensure");
        let kp = id.keypair;
        let pubkey = pubkey_base58(&kp);
        let store = SqliteStore::open(&db_path).expect("sqlite");
        store
            .save_attestation(
                "cli-test-seed",
                "seed-content",
                "seed-hash",
                &["seed".to_string()],
                "local:seed-sol",
                "local:seed-ar",
                &pubkey,
                &pubkey,
                &chrono::Utc::now().to_rfc3339(),
                WriteMode::Participate,
                Visibility::Private,
                &[0.0; 8],
            )
            .expect("save");
    }

    let mut cmd = Command::new(bin);
    cmd.arg("mcp-stdio")
        // Force `dirs::home_dir()` and the identity stub-path resolution to
        // land inside our tempdir instead of the developer's real $HOME.
        .env("HOME", &fake_home)
        .env("MNEMONIC_CONFIG_DIR", fake_home.join(".mnemonic"))
        .env("STORAGE_MODE", "local")
        .env("PAYMENT_MODE", "none")
        // F-1 round-2 fix: skip the initial+background pricing.refresh()
        // calls so the binary never reaches out to uploader.irys.xyz or
        // api.coingecko.com on startup (the ~20s blocker that previously
        // forced #[ignore]). Test-only env contract; main.rs documents the
        // semantics.
        .env("PRICING_REFRESH_DISABLED", "1")
        // EMBED_PROVIDER=stub-test bypasses the 60s+ 1536-dim TurboQuant
        // rotation-matrix build cost OpenAIEmbedder would trigger; the
        // test only calls `tools/list`, which never invokes embed().
        .env("EMBED_PROVIDER", "stub-test")
        .env("OPENAI_API_KEY", "")
        .env("DATABASE_PATH", &db_path)
        .env("RAG_CHUNK_DIR", &rag_dir)
        .env("OLLAMA_URL", "http://localhost:11434")
        .env("LLM_PROVIDER", "ollama")
        .env("LLM_API_URL", "http://localhost:11434")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn mnemonic-mcp mcp-stdio");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let mut reader = BufReader::new(stdout).lines();

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

    async fn round_trip(
        writer: &mut tokio::process::ChildStdin,
        reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let line = serde_json::to_string(&body).unwrap();
        writer.write_all(line.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        loop {
            let line = reader.next_line().await.unwrap().unwrap();
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                return serde_json::from_str(trimmed).unwrap();
            }
        }
    }

    let resp = match timeout(
        STARTUP_TIMEOUT,
        round_trip(
            &mut stdin,
            &mut reader,
            serde_json::json!({"jsonrpc":"2.0","method":"tools/list","id":1,"params":{}}),
        ),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let logs = stderr_capture.lock().await.clone();
            let _ = child.kill().await;
            panic!("timeout: {e:?}\n--- stderr ---\n{logs}");
        }
    };
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools array missing");
    // F-2 (test-reviewer round 1): the task spec's TDD anchor calls for the
    // full post-Task-2 inventory (5 pre-existing + the new public-write
    // confirmation tool from Task 4). Asserting `>= 5` keeps the test
    // robust to a 6th or 7th tool being added without coupling Task 5 to
    // an exact count owned by Tasks 1/2/4. The earlier `!is_empty()` form
    // would have accepted a regression that returned a single dummy tool.
    assert!(
        tools.len() >= 5,
        "tools/list should return >= 5 tool entries; got {} (full response: {resp})",
        tools.len()
    );

    drop(stdin);
    let _ = timeout(REQ_TIMEOUT, child.wait()).await;
    let _ = child.kill().await;
}

// ── MNEMONIC_HOSTED_ENDPOINT gating: spawned binary, stderr capture ───────

/// Helper: spawn the binary in stdio mode with the env var set; capture
/// stderr until we see the warning line OR the startup deadline elapses.
/// We use a tempdir HOME + minimal env so the binary never tries to read
/// the developer's real `~/.mnemonic` state.
async fn run_stdio_with_env(
    config_dir: &std::path::Path,
    db_path: &std::path::Path,
    hosted_env: &str,
    allow_flag: bool,
) -> String {
    let bin = env!("CARGO_BIN_EXE_mnemonic-mcp");
    let rag_dir = config_dir.join("rag");
    std::fs::create_dir_all(&rag_dir).expect("rag dir");

    // Pre-seed identity in the binary's `dirs::home_dir()/.mnemonic`
    // location (we set HOME=config_dir below) so `core::identity::ensure()`
    // finds a pre-existing keypair and does NOT print the "mnemonic:
    // identity created" line. Same fast-path used by
    // `mcp_stdio_accepts_jsonrpc_on_stdin`.
    let dotmnemonic = config_dir.join(".mnemonic");
    std::fs::create_dir_all(&dotmnemonic).expect("dotmnemonic");
    let keypair_path = dotmnemonic.join("identity.json");
    {
        use mnemonic_core::identity::{ensure_with_stores, pubkey_base58, FileKeyStore, KeyStores};
        use mnemonic_core::storage::{AttestationStore, SqliteStore, Visibility, WriteMode};
        let stores = KeyStores {
            os: None,
            file: Box::new(FileKeyStore::new(keypair_path.clone())),
            identity_path: keypair_path.clone(),
            readme_path: dotmnemonic.join("README.txt"),
        };
        let id = ensure_with_stores(stores).expect("ensure");
        let kp = id.keypair;
        let pubkey = pubkey_base58(&kp);
        let store = SqliteStore::open(db_path).expect("sqlite");
        store
            .save_attestation(
                "seed",
                "c",
                "h",
                &["s".to_string()],
                "local:s",
                "local:a",
                &pubkey,
                &pubkey,
                &chrono::Utc::now().to_rfc3339(),
                WriteMode::Participate,
                Visibility::Private,
                &[0.0; 8],
            )
            .expect("save");
    }

    let mut cmd = Command::new(bin);
    if allow_flag {
        cmd.arg("--allow-custom-endpoint");
    }
    cmd.arg("mcp-stdio")
        // HOME redirects dirs::home_dir() into the tempdir, so
        // identity::ensure() resolves to the pre-seeded keypair instead of
        // touching the developer's real ~/.mnemonic.
        .env("HOME", config_dir)
        .env("MNEMONIC_HOSTED_ENDPOINT", hosted_env)
        .env("MNEMONIC_CONFIG_DIR", &dotmnemonic)
        .env("STORAGE_MODE", "local")
        .env("PAYMENT_MODE", "none")
        // F-1 round-2 fix: see comment in mcp_stdio_accepts_jsonrpc_on_stdin.
        .env("PRICING_REFRESH_DISABLED", "1")
        .env("EMBED_PROVIDER", "stub-test")
        .env("OPENAI_API_KEY", "")
        .env("DATABASE_PATH", db_path)
        .env("RAG_CHUNK_DIR", &rag_dir)
        .env("OLLAMA_URL", "http://localhost:11434")
        .env("LLM_PROVIDER", "ollama")
        .env("LLM_API_URL", "http://localhost:11434")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn binary");
    let stderr = child.stderr.take().expect("stderr");
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
    // Give the binary up to ~10s for the stderr line to land. The warning
    // is printed BEFORE the embedder builds (which is fast under
    // EMBED_PROVIDER=stub-test), so it should land within the first
    // hundred ms on any reasonable runner.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        {
            let buf = stderr_capture.lock().await.clone();
            if !buf.is_empty() && buf.lines().count() >= 1 {
                // Bail out once we have ANY stderr (warning OR a tracing line).
                // The caller asserts the precise substring.
                if std::time::Instant::now() >= deadline {
                    break;
                }
                // Wait one more tick for the warning line to fully flush
                // alongside the tracing-subscriber init line.
                tokio::time::sleep(Duration::from_millis(200)).await;
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    let captured = stderr_capture.lock().await.clone();
    captured
}

#[tokio::test]
async fn env_var_ignored_without_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("attestations.db");
    // Use an unreachable, non-routable URL — even if the binary's soft-fall
    // code ever did try to connect (which it doesn't on `tools/list`), this
    // would fail fast rather than touch a real host.
    let stderr_text = run_stdio_with_env(
        tmp.path(),
        &db_path,
        "http://attacker.example.invalid",
        false,
    )
    .await;
    assert!(
        stderr_text
            .contains("ignoring MNEMONIC_HOSTED_ENDPOINT — use --allow-custom-endpoint to enable"),
        "expected spec'd warning line, got: {stderr_text:?}"
    );
}

#[tokio::test]
async fn env_var_honored_with_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("attestations.db");
    let stderr_text = run_stdio_with_env(
        tmp.path(),
        &db_path,
        "http://attacker.example.invalid",
        true,
    )
    .await;
    assert!(
        !stderr_text
            .contains("ignoring MNEMONIC_HOSTED_ENDPOINT — use --allow-custom-endpoint to enable"),
        "warning line MUST NOT fire when --allow-custom-endpoint is passed; \
         got stderr: {stderr_text:?}"
    );
}
