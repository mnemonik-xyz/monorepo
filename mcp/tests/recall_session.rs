//! Integration tests for Task 13 — hosted recall session.
//!
//! Acceptance criteria:
//! - After session ends (DELETE or TTL), recall returns `sealed_hidden` again.
//! - A process restart drops all sessions (sessions are in-memory only).
//! - Audit log contains start/end events but no RK or content.
//! - `mnemonic_recall` during session returns opened sealed rows.

use std::sync::Arc;

use mnemonic_core::identity::recall_key::{generate_rk, wrap_rk};
use mnemonic_mcp::api::{get_active_rk, RecallSession};
use mnemonic_mcp::test_support::mock_state;

// ── Tests ────────────────────────────────────────────────────────────────────

/// Session map is empty on creation — a new process has no sessions.
#[tokio::test]
async fn new_process_has_no_sessions() {
    let state = mock_state();
    let rk = get_active_rk(&state.recall_sessions, "any-owner").await;
    assert!(rk.is_none(), "fresh process must have no sessions");
}

/// `get_active_rk` returns `None` for an unknown owner.
#[tokio::test]
async fn unknown_owner_returns_none() {
    let state = mock_state();
    let result = get_active_rk(&state.recall_sessions, "unknown-owner-pubkey").await;
    assert!(result.is_none());
}

/// A session inserted directly into the map is retrievable until deleted.
#[tokio::test]
async fn insert_and_retrieve_session() {
    let state = mock_state();
    let owner = "owner-pubkey-for-insert";
    let rk_bytes = [0x42u8; 32];

    {
        let mut sessions = state.recall_sessions.lock().await;
        sessions.insert(
            owner.to_string(),
            RecallSession {
                rk: zeroize::Zeroizing::new(rk_bytes),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
                started_at: "2026-09-28T00:00:00Z".to_string(),
            },
        );
    }

    let retrieved = get_active_rk(&state.recall_sessions, owner).await;
    assert!(retrieved.is_some(), "session should be retrievable");
    assert_eq!(*retrieved.unwrap(), rk_bytes);

    // Remove the session.
    {
        let mut sessions = state.recall_sessions.lock().await;
        sessions.remove(owner);
    }

    let after_delete = get_active_rk(&state.recall_sessions, owner).await;
    assert!(
        after_delete.is_none(),
        "session should be gone after deletion"
    );
}

/// An expired session is lazily evicted by `get_active_rk`.
#[tokio::test]
async fn expired_session_is_evicted() {
    let state = mock_state();
    let owner = "owner-pubkey-expired";

    {
        let mut sessions = state.recall_sessions.lock().await;
        sessions.insert(
            owner.to_string(),
            RecallSession {
                rk: zeroize::Zeroizing::new([0xAA; 32]),
                // Already expired.
                expires_at: std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or(std::time::Instant::now()),
                started_at: "2026-09-28T00:00:00Z".to_string(),
            },
        );
    }

    let rk = get_active_rk(&state.recall_sessions, owner).await;
    assert!(rk.is_none(), "expired session must not be returned");

    // Also verify it was evicted from the map.
    let sessions = state.recall_sessions.lock().await;
    assert!(
        !sessions.contains_key(owner),
        "expired session must be evicted"
    );
}

/// Verify that the `RECALL_SESSION_MAX_TTL_SECS` constant is honoured.
#[tokio::test]
async fn max_ttl_constant_is_one_hour() {
    assert_eq!(
        mnemonic_mcp::api::RECALL_SESSION_MAX_TTL_SECS,
        3600,
        "max TTL must be 3600s (1 hour) per spec"
    );
}

/// Recall key round-trip: generate RK, wrap it to a mock public key,
/// verify unwrap_rk recovers the same key.
#[tokio::test]
async fn recall_key_wrap_unwrap_round_trip() {
    // Use the state's bootstrap server key as the recipient to drive a full
    // wrap/unwrap cycle without any external X25519 crate imports.
    let state = mock_state();

    // Generate a fresh RK.
    let rk = generate_rk();

    // Wrap the RK to the server's bootstrap public key.
    let server_pub: [u8; 32] = state.bootstrap_server_x25519_public.to_bytes();
    let wrapped = wrap_rk(&rk, &server_pub).expect("wrap_rk failed");

    // Unwrap using the server's bootstrap secret key.
    let server_sk: [u8; 32] = state.bootstrap_server_x25519_secret.to_bytes();
    let recovered =
        mnemonic_core::identity::recall_key::unwrap_rk(&wrapped.enc, &wrapped.wk, &server_sk)
            .expect("unwrap_rk failed");

    assert_eq!(*recovered, rk, "recovered RK must match original");
}

/// Session isolation: different owners have independent sessions.
#[tokio::test]
async fn session_isolation_between_owners() {
    let state = mock_state();
    let owner_a = "owner-a";
    let owner_b = "owner-b";
    let rk_a = [0x11u8; 32];
    let rk_b = [0x22u8; 32];

    {
        let mut sessions = state.recall_sessions.lock().await;
        sessions.insert(
            owner_a.to_string(),
            RecallSession {
                rk: zeroize::Zeroizing::new(rk_a),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
                started_at: "2026-09-28T00:00:00Z".to_string(),
            },
        );
        sessions.insert(
            owner_b.to_string(),
            RecallSession {
                rk: zeroize::Zeroizing::new(rk_b),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
                started_at: "2026-09-28T00:00:00Z".to_string(),
            },
        );
    }

    let ra = get_active_rk(&state.recall_sessions, owner_a)
        .await
        .unwrap();
    let rb = get_active_rk(&state.recall_sessions, owner_b)
        .await
        .unwrap();

    assert_eq!(*ra, rk_a, "owner A gets their own RK");
    assert_eq!(*rb, rk_b, "owner B gets their own RK");
    assert_ne!(*ra, *rb, "sessions must not cross-contaminate");

    // Deleting A does not affect B.
    state.recall_sessions.lock().await.remove(owner_a);

    assert!(
        get_active_rk(&state.recall_sessions, owner_a)
            .await
            .is_none(),
        "owner A session should be gone"
    );
    assert!(
        get_active_rk(&state.recall_sessions, owner_b)
            .await
            .is_some(),
        "owner B session should still exist"
    );
}

/// The session map is purely in-memory: creating a new `mock_state()` gives a
/// fresh empty map, modelling a process restart.
#[tokio::test]
async fn process_restart_drops_all_sessions() {
    let state1 = mock_state();
    let owner = "owner-restart-test";

    {
        let mut sessions = state1.recall_sessions.lock().await;
        sessions.insert(
            owner.to_string(),
            RecallSession {
                rk: zeroize::Zeroizing::new([0xCC; 32]),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(60),
                started_at: "2026-09-28T00:00:00Z".to_string(),
            },
        );
    }

    // Simulate restart by creating a fresh state (new process has no sessions).
    let state2 = mock_state();
    let rk = get_active_rk(&state2.recall_sessions, owner).await;
    assert!(
        rk.is_none(),
        "new state (simulated restart) must have no sessions"
    );
}
