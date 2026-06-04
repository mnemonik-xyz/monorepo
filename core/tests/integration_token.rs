//! Integration tests for `mnemonic_core::identity::token_store`.
//!
//! Anchor tests for Task 6 of the agent-native-distribution feature. All
//! file I/O is sandboxed in `tempfile::TempDir` so the suite does not
//! collide with the developer's real `~/.mnemonic/token.json`.

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use mnemonic_core::identity::{
    delete_token_at, read_token_from, save_token_to, TokenJson, TokenStoreError,
};
use serde_json::json;
use tempfile::TempDir;

fn future_iso(hours_ahead: i64) -> String {
    (Utc::now() + chrono::Duration::hours(hours_ahead)).to_rfc3339()
}

fn past_iso(hours_back: i64) -> String {
    (Utc::now() - chrono::Duration::hours(hours_back)).to_rfc3339()
}

fn sample_token(expires_at: &str) -> TokenJson {
    TokenJson {
        jwt: "header.payload.signature".to_string(),
        expires_at: expires_at.to_string(),
        sub: "TestPubkey111111111111111111111111111111111".to_string(),
    }
}

#[test]
fn roundtrip_write_read_delete() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    let original = sample_token(&future_iso(1));
    save_token_to(&path, &original).expect("save_token");

    let loaded = read_token_from(&path)
        .expect("read_token")
        .expect("token present");
    assert_eq!(loaded, original, "round-trip must preserve all fields");

    delete_token_at(&path).expect("delete_token");
    let after_delete = read_token_from(&path).expect("read after delete");
    assert!(
        after_delete.is_none(),
        "read after delete must return Ok(None), got {after_delete:?}"
    );
}

#[test]
fn expired_token_returns_expired_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    let expired_at = past_iso(1);
    save_token_to(&path, &sample_token(&expired_at)).unwrap();

    let err = read_token_from(&path).expect_err("expected Expired");
    match err {
        TokenStoreError::Expired { expires_at, sub } => {
            assert_eq!(expires_at, expired_at);
            assert_eq!(sub, "TestPubkey111111111111111111111111111111111");
        }
        other => panic!("expected TokenStoreError::Expired, got {other:?}"),
    }
}

#[test]
fn malformed_json_returns_none() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    // {"not": "valid"} — well-formed JSON but does not deserialize into
    // TokenJson (missing required fields). The spec says we degrade to
    // "no usable token" rather than crashing.
    std::fs::write(&path, json!({"not": "valid"}).to_string()).unwrap();

    let result = read_token_from(&path).expect("must not error on malformed");
    assert!(
        result.is_none(),
        "malformed JSON must read as Ok(None), got {result:?}"
    );
}

#[test]
fn missing_field_returns_none() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    // Missing `expires_at` — same degradation contract.
    std::fs::write(&path, json!({"jwt": "abc", "sub": "user"}).to_string()).unwrap();

    let result = read_token_from(&path).expect("must not error on missing field");
    assert!(
        result.is_none(),
        "missing field must read as Ok(None), got {result:?}"
    );
}

#[test]
fn save_is_atomic() {
    // Atomicity contract under concurrent writes: rename(2) is atomic on
    // POSIX, so a reader racing a `save_token_to` call sees either the
    // PREVIOUS file or the NEW file — never a half-written file, never
    // a missing-file moment. Round-1 test review F1: the writer here runs
    // a TIGHT loop with no sleep so the rename window is maximally
    // adversarial against the reader; a regression that replaces the
    // tempfile+rename path with a direct in-place truncate-then-write
    // would let the reader observe `Ok(None)` (parser fail on a partial
    // file) or a NotFound (file vanished between truncate and rewrite).
    //
    // We give the writer a 1ms head start AFTER the barrier so it has a
    // pending write queued by the time the reader's loop begins —
    // without this the reader's first ~few reads happen before the
    // writer has scheduled anything.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    let v1 = sample_token(&future_iso(1));
    let v2 = TokenJson {
        jwt: "v2-jwt".to_string(),
        ..v1.clone()
    };
    save_token_to(&path, &v1).unwrap();

    let path_clone = path.clone();
    let v2_clone = v2.clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_writer = Arc::clone(&stop);
    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);

    let writer = thread::spawn(move || {
        writer_barrier.wait();
        // Tight loop with no sleep — maximises the rename race window.
        while !stop_writer.load(std::sync::atomic::Ordering::Relaxed) {
            save_token_to(&path_clone, &v2_clone).unwrap();
        }
    });

    barrier.wait();
    // Give the writer a 1ms head start so the reader's first iterations
    // collide with an in-flight write rather than the seeded v1.
    thread::sleep(Duration::from_millis(1));
    let deadline = std::time::Instant::now() + Duration::from_millis(100);
    let mut reads = 0u64;
    while std::time::Instant::now() < deadline {
        let observed = read_token_from(&path).expect("reader must never error mid-rename");
        let observed = observed.expect(
            "reader observed Ok(None) — a half-written or missing file. \
             save_token_to is no longer atomic; check that NamedTempFile + \
             persist() (rename(2)) is still in use rather than a direct \
             truncate-then-write.",
        );
        assert!(
            observed == v1 || observed == v2,
            "reader observed a value that is neither v1 nor v2: {observed:?}"
        );
        reads += 1;
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().unwrap();
    assert!(
        reads > 100,
        "reader loop must execute many reads; got {reads}"
    );
}

#[cfg(unix)]
#[test]
fn save_sets_mode_0600_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    save_token_to(&path, &sample_token(&future_iso(1))).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "Unix file mode must be 0600, got {mode:o}");
}

/// F2 — task spec Details: "HOME env var unset → dirs::home_dir() returns
/// None → Err(TokenStoreError::Io(...))". Exercised here via the
/// `MNEMONIC_CONFIG_DIR` override pointing at a path whose parent does
/// not exist (and cannot be created), which surfaces the same I/O error
/// without mutating the process-global `$HOME`. The `set_var` call is
/// scoped under a static Mutex shared with [`config_dir_override_isolates_path`]
/// so the two tests do not race on the env var.
#[test]
fn home_unset_or_unwritable_yields_io_error() {
    let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os("MNEMONIC_CONFIG_DIR");
    // SAFETY: this test owns the ENV_GUARD lock for the duration of the
    // mutation; restore on exit. No async runtime is touched.
    // Point the override at a path whose parent does not exist AND cannot
    // be created (file at /dev/null is not a directory).
    unsafe {
        std::env::set_var("MNEMONIC_CONFIG_DIR", "/dev/null/mnemonic-token-test");
    }
    let result = mnemonic_core::identity::save_token(&sample_token(&future_iso(1)));
    unsafe {
        match previous {
            Some(v) => std::env::set_var("MNEMONIC_CONFIG_DIR", v),
            None => std::env::remove_var("MNEMONIC_CONFIG_DIR"),
        }
    }
    let err = result.expect_err("save into unwritable path must Err");
    assert!(
        matches!(err, TokenStoreError::Io(_)),
        "expected Io error, got {err:?}"
    );
}

/// `MNEMONIC_CONFIG_DIR` parity test. Setting it makes `token_path()`
/// resolve to `<override>/token.json` — the same behaviour Node CLI's
/// `configDir()` provides at `packages/cli/src/config.ts:48-52`. This
/// is the test seam that replaces HOME mutation in the mcp integration
/// tests (Round-1 code-reviewer R1-MAJOR-2).
#[test]
fn config_dir_override_isolates_path() {
    let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let previous = std::env::var_os("MNEMONIC_CONFIG_DIR");
    unsafe {
        std::env::set_var("MNEMONIC_CONFIG_DIR", dir.path());
    }
    let expected = dir.path().join("token.json");
    let actual = mnemonic_core::identity::token_path();
    let save_result = mnemonic_core::identity::save_token(&sample_token(&future_iso(1)));
    let read_result = mnemonic_core::identity::read_token();
    unsafe {
        match previous {
            Some(v) => std::env::set_var("MNEMONIC_CONFIG_DIR", v),
            None => std::env::remove_var("MNEMONIC_CONFIG_DIR"),
        }
    }
    assert_eq!(
        actual.expect("token_path must succeed under override"),
        expected,
        "MNEMONIC_CONFIG_DIR must override token_path"
    );
    save_result.expect("save under override must succeed");
    let token = read_result
        .expect("read under override must not error")
        .expect("token must be present after save");
    assert_eq!(token.jwt, "header.payload.signature");
}

/// Shared lock for the two tests that mutate `MNEMONIC_CONFIG_DIR`. The
/// `into_inner()` pattern on poisoning is intentional: a panic inside the
/// previous test left the env var possibly stale, but the next test sets
/// it explicitly before reading, so a poisoned lock does not corrupt the
/// test's observed environment.
static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn delete_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("token.json");

    // Never written; first delete is a no-op.
    delete_token_at(&path).expect("first delete must succeed");
    delete_token_at(&path).expect("second delete must succeed");

    // Now write + delete + delete again.
    save_token_to(&path, &sample_token(&future_iso(1))).unwrap();
    delete_token_at(&path).expect("delete after save must succeed");
    delete_token_at(&path).expect("second delete after save must succeed");
}
