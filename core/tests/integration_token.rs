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
    // Atomicity contract: while one thread is mid-write, another reader
    // sees either the OLD contents or the NEW contents — never a
    // half-written file. We seed the path with v1, then drive a writer
    // and a reader concurrently. A reader who races with the writer must
    // see EITHER v1 OR v2, never a partial / malformed file.
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
    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);

    let writer = thread::spawn(move || {
        writer_barrier.wait();
        for _ in 0..50 {
            save_token_to(&path_clone, &v2_clone).unwrap();
            thread::sleep(Duration::from_micros(100));
        }
    });

    barrier.wait();
    // Race the writer: read many times in a tight loop and assert each
    // read returns either v1 or v2 — never `Ok(None)` (which would
    // signal a partially-written file the parser couldn't decode).
    for _ in 0..500 {
        let observed = read_token_from(&path).expect("reader must not error");
        let observed = observed.expect("file must never appear missing or malformed mid-rename");
        assert!(
            observed == v1 || observed == v2,
            "reader observed a corrupt intermediate value: {observed:?}"
        );
    }
    writer.join().unwrap();
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
