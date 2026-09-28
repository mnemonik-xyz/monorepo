//! A2A golden fixture emitter for conformance testing (a2a-bridge Task 2).
//!
//! Gated behind BOTH `golden-fixtures` AND `a2a-experimental` features so the
//! emitter does not run on default `cargo test`. Invoked as:
//!
//! ```sh
//! cargo test --features "golden-fixtures a2a-experimental" -p mnemonic-core \
//!   --test a2a_golden_fixtures emit_fixtures -- --ignored --nocapture
//! ```
//!
//! The emitter prints a JSON array of `{name, a2a_json, jcs_canonical_hex,
//! cbor_envelope_hex, cose_signed_hex, keypair_secret_hex}` entries to stdout.
//! A companion SDK test (or any conformance harness) loads this JSON and verifies
//! byte-for-byte parity of the serialization pipeline.
//!
//! Determinism rules (mirrors `golden_fixtures.rs`):
//! - Hardcoded 32-byte Ed25519 seed → deterministic keypair → same COSE signature
//!   on every run (Ed25519 is deterministic per RFC 8032).
//! - All timestamps are fixed strings, never `chrono::Utc::now()`.
//! - Re-running the emitter twice MUST produce byte-identical stdout; the
//!   `test_emitter_deterministic` test asserts this.
//!
//! Tech-spec reference: `work/a2a-bridge/tech-spec.md` Decision 7.

#![cfg(all(
    feature = "golden-fixtures",
    feature = "a2a-experimental",
    not(target_arch = "wasm32")
))]

use mnemonic_core::codec::a2a::{A2aArtifact, Message, Part, Task};
use serde::Serialize;
use solana_sdk::signature::{Keypair, Signer as _};

/// One emitted A2A fixture entry. Field order = emission order in JSON.
#[derive(Serialize)]
struct A2aGoldenEntry {
    /// Stable, human-readable test-case label.
    name: String,
    /// The A2A object serialised as regular JSON (not JCS). Documentation only —
    /// conformance harnesses consume `jcs_canonical_hex` directly.
    a2a_json: String,
    /// Hex of the RFC 8785 JCS canonical bytes (`to_jcs_bytes()`).
    jcs_canonical_hex: String,
    /// Hex of the CBOR envelope wrapping the JCS bytes (currently the same as
    /// `cose_signed_hex` payload — emitted for transparency and future use when
    /// a dedicated `to_cbor_envelope()` is added to the A2A codec).
    cbor_envelope_hex: String,
    /// Hex of the COSE_Sign1 envelope (`to_canonical_envelope()`).
    cose_signed_hex: String,
    /// Hex of the 64-byte Ed25519 secret used to sign. Identical across entries;
    /// kept per-entry so a conformance harness is self-contained.
    keypair_secret_hex: String,
}

/// Deterministic 32-byte Ed25519 seed used by every fixture.
///
/// Hex: `0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20`.
/// TEST-ONLY — must never be used to sign real attestations.
const FIXED_SEED_HEX: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

fn fixture_keypair() -> Keypair {
    let seed_vec = hex::decode(FIXED_SEED_HEX).expect("seed hex parses");
    assert_eq!(seed_vec.len(), 32);
    let mut seed_arr = [0u8; 32];
    seed_arr.copy_from_slice(&seed_vec);
    Keypair::new_from_array(seed_arr)
}

// ── Fixture case helpers ─────────────────────────────────────────────────────

fn make_message_entry(name: &str, msg: &Message, kp: &Keypair) -> A2aGoldenEntry {
    let a2a_json = serde_json::to_string(msg).expect("Message JSON");
    let jcs = msg.to_jcs_bytes().expect("Message JCS");
    let cose = msg.to_canonical_envelope(kp, None).expect("Message COSE");
    A2aGoldenEntry {
        name: name.to_string(),
        a2a_json,
        jcs_canonical_hex: hex::encode(&jcs),
        cbor_envelope_hex: hex::encode(&jcs), // JCS bytes ARE the inner payload
        cose_signed_hex: hex::encode(&cose),
        keypair_secret_hex: hex::encode(kp.to_bytes()),
    }
}

fn make_task_entry(name: &str, task: &Task, kp: &Keypair) -> A2aGoldenEntry {
    let a2a_json = serde_json::to_string(task).expect("Task JSON");
    let jcs = task.to_jcs_bytes().expect("Task JCS");
    let cose = task.to_canonical_envelope(kp, None).expect("Task COSE");
    A2aGoldenEntry {
        name: name.to_string(),
        a2a_json,
        jcs_canonical_hex: hex::encode(&jcs),
        cbor_envelope_hex: hex::encode(&jcs),
        cose_signed_hex: hex::encode(&cose),
        keypair_secret_hex: hex::encode(kp.to_bytes()),
    }
}

fn make_artifact_entry(name: &str, art: &A2aArtifact, kp: &Keypair) -> A2aGoldenEntry {
    let a2a_json = serde_json::to_string(art).expect("A2aArtifact JSON");
    let jcs = art.to_jcs_bytes().expect("A2aArtifact JCS");
    let cose = art.to_canonical_envelope(kp, None).expect("A2aArtifact COSE");
    A2aGoldenEntry {
        name: name.to_string(),
        a2a_json,
        jcs_canonical_hex: hex::encode(&jcs),
        cbor_envelope_hex: hex::encode(&jcs),
        cose_signed_hex: hex::encode(&cose),
        keypair_secret_hex: hex::encode(kp.to_bytes()),
    }
}

// ── Fixture catalogue ────────────────────────────────────────────────────────

/// Build all golden entries. Pure function — calling it twice produces
/// identical results (determinism contract for `test_emitter_deterministic`).
fn build_all_entries() -> Vec<A2aGoldenEntry> {
    let kp = fixture_keypair();
    let pubkey = kp.pubkey().to_string();
    let mut entries = Vec::new();

    // ── Message fixtures ──────────────────────────────────────────────────────

    // 1. Text-only message (minimal)
    entries.push(make_message_entry(
        "message_text_only",
        &Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: "hello agent".to_string(),
            }],
            message_id: "msg-fixture-001".to_string(),
            task_id: None,
            context_id: None,
        },
        &kp,
    ));

    // 2. Text message with context and task ids
    entries.push(make_message_entry(
        "message_text_with_context",
        &Message {
            role: "agent".to_string(),
            parts: vec![Part::Text {
                text: "I have processed your request.".to_string(),
            }],
            message_id: "msg-fixture-002".to_string(),
            task_id: Some("task-fixture-001".to_string()),
            context_id: Some("ctx-fixture-001".to_string()),
        },
        &kp,
    ));

    // 3. File-part message (URI only)
    entries.push(make_message_entry(
        "message_file_uri",
        &Message {
            role: "user".to_string(),
            parts: vec![Part::File {
                uri: Some("https://example.com/document.pdf".to_string()),
                mime_type: Some("application/pdf".to_string()),
                data: None,
            }],
            message_id: "msg-fixture-003".to_string(),
            task_id: None,
            context_id: Some("ctx-fixture-002".to_string()),
        },
        &kp,
    ));

    // 4. File-part message (inline base64 data)
    entries.push(make_message_entry(
        "message_file_inline",
        &Message {
            role: "user".to_string(),
            parts: vec![Part::File {
                uri: None,
                mime_type: Some("image/png".to_string()),
                data: Some("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string()),
            }],
            message_id: "msg-fixture-004".to_string(),
            task_id: Some("task-fixture-002".to_string()),
            context_id: Some("ctx-fixture-002".to_string()),
        },
        &kp,
    ));

    // 5. Data-part message
    entries.push(make_message_entry(
        "message_data_part",
        &Message {
            role: "agent".to_string(),
            parts: vec![Part::Data {
                data: serde_json::json!({
                    "result": "success",
                    "count": 42,
                    "items": ["a", "b", "c"]
                }),
                mime_type: Some("application/json".to_string()),
            }],
            message_id: "msg-fixture-005".to_string(),
            task_id: None,
            context_id: None,
        },
        &kp,
    ));

    // 6. Multi-part message (text + data)
    entries.push(make_message_entry(
        "message_multi_part",
        &Message {
            role: "agent".to_string(),
            parts: vec![
                Part::Text {
                    text: "Here is the analysis:".to_string(),
                },
                Part::Data {
                    data: serde_json::json!({"score": 0.97, "label": "positive"}),
                    mime_type: None,
                },
            ],
            message_id: "msg-fixture-006".to_string(),
            task_id: Some("task-fixture-003".to_string()),
            context_id: Some("ctx-fixture-003".to_string()),
        },
        &kp,
    ));

    // 7. Message with UTF-8 content (Cyrillic)
    entries.push(make_message_entry(
        "message_utf8_cyrillic",
        &Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: "Привет, агент! Помоги мне с задачей.".to_string(),
            }],
            message_id: "msg-fixture-007".to_string(),
            task_id: None,
            context_id: Some("ctx-fixture-004".to_string()),
        },
        &kp,
    ));

    // 8. Message with empty text part
    entries.push(make_message_entry(
        "message_empty_text",
        &Message {
            role: "user".to_string(),
            parts: vec![Part::Text {
                text: String::new(),
            }],
            message_id: "msg-fixture-008".to_string(),
            task_id: None,
            context_id: None,
        },
        &kp,
    ));

    // ── Task fixtures ─────────────────────────────────────────────────────────

    // 9. Single-turn task (submitted state)
    entries.push(make_task_entry(
        "task_single_turn_submitted",
        &Task {
            id: "task-fixture-001".to_string(),
            context_id: "ctx-fixture-001".to_string(),
            status: "submitted".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    // 10. Task in working state
    entries.push(make_task_entry(
        "task_working",
        &Task {
            id: "task-fixture-002".to_string(),
            context_id: "ctx-fixture-002".to_string(),
            status: "working".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    // 11. Completed task (no artifacts)
    entries.push(make_task_entry(
        "task_completed_no_artifacts",
        &Task {
            id: "task-fixture-003".to_string(),
            context_id: "ctx-fixture-003".to_string(),
            status: "completed".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    // 12. Completed task with single artifact (text)
    entries.push(make_task_entry(
        "task_completed_single_artifact",
        &Task {
            id: "task-fixture-004".to_string(),
            context_id: "ctx-fixture-003".to_string(),
            status: "completed".to_string(),
            history: None,
            artifacts: Some(vec![mnemonic_core::codec::a2a::task::TaskArtifact {
                artifact_id: Some("art-fixture-001".to_string()),
                name: Some("summary".to_string()),
                parts: vec![Part::Text {
                    text: "The analysis is complete.".to_string(),
                }],
            }]),
        },
        &kp,
    ));

    // 13. Multi-turn task (with history)
    let history_msgs = vec![
        serde_json::json!({
            "role": "user",
            "parts": [{"kind": "text", "text": "Start the task"}],
            "messageId": "hist-msg-001"
        }),
        serde_json::json!({
            "role": "agent",
            "parts": [{"kind": "text", "text": "Working on it..."}],
            "messageId": "hist-msg-002"
        }),
    ];
    entries.push(make_task_entry(
        "task_multi_turn_with_history",
        &Task {
            id: "task-fixture-005".to_string(),
            context_id: "ctx-fixture-005".to_string(),
            status: "working".to_string(),
            history: Some(history_msgs),
            artifacts: None,
        },
        &kp,
    ));

    // 14. Failed task
    entries.push(make_task_entry(
        "task_failed",
        &Task {
            id: "task-fixture-006".to_string(),
            context_id: "ctx-fixture-006".to_string(),
            status: "failed".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    // 15. Cancelled task
    entries.push(make_task_entry(
        "task_cancelled",
        &Task {
            id: "task-fixture-007".to_string(),
            context_id: "ctx-fixture-007".to_string(),
            status: "cancelled".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    // 16. Completed task with multi-artifact output
    entries.push(make_task_entry(
        "task_completed_multi_artifact",
        &Task {
            id: "task-fixture-008".to_string(),
            context_id: "ctx-fixture-008".to_string(),
            status: "completed".to_string(),
            history: None,
            artifacts: Some(vec![
                mnemonic_core::codec::a2a::task::TaskArtifact {
                    artifact_id: Some("art-multi-001".to_string()),
                    name: Some("text-result".to_string()),
                    parts: vec![Part::Text {
                        text: "First output artifact.".to_string(),
                    }],
                },
                mnemonic_core::codec::a2a::task::TaskArtifact {
                    artifact_id: Some("art-multi-002".to_string()),
                    name: Some("data-result".to_string()),
                    parts: vec![Part::Data {
                        data: serde_json::json!({"rows": 100, "ok": true}),
                        mime_type: Some("application/json".to_string()),
                    }],
                },
            ]),
        },
        &kp,
    ));

    // ── Artifact fixtures ─────────────────────────────────────────────────────

    // 17. Standalone artifact — text only
    entries.push(make_artifact_entry(
        "artifact_text_only",
        &A2aArtifact {
            artifact_id: Some("art-standalone-001".to_string()),
            name: Some("report".to_string()),
            parts: vec![Part::Text {
                text: "Final report content goes here.".to_string(),
            }],
        },
        &kp,
    ));

    // 18. Artifact — file reference
    entries.push(make_artifact_entry(
        "artifact_file_uri",
        &A2aArtifact {
            artifact_id: Some("art-standalone-002".to_string()),
            name: Some("exported-csv".to_string()),
            parts: vec![Part::File {
                uri: Some("https://storage.example.com/output.csv".to_string()),
                mime_type: Some("text/csv".to_string()),
                data: None,
            }],
        },
        &kp,
    ));

    // 19. Artifact — data part (structured JSON payload)
    entries.push(make_artifact_entry(
        "artifact_data_json",
        &A2aArtifact {
            artifact_id: Some("art-standalone-003".to_string()),
            name: None,
            parts: vec![Part::Data {
                data: serde_json::json!({
                    "model": "gpt-4o",
                    "usage": {"prompt_tokens": 1234, "completion_tokens": 567},
                    "verified": true,
                    "signer": pubkey
                }),
                mime_type: Some("application/json".to_string()),
            }],
        },
        &kp,
    ));

    // 20. Artifact — multi-part (text + data)
    entries.push(make_artifact_entry(
        "artifact_multi_part",
        &A2aArtifact {
            artifact_id: Some("art-standalone-004".to_string()),
            name: Some("combined-output".to_string()),
            parts: vec![
                Part::Text {
                    text: "Summary: analysis completed successfully.".to_string(),
                },
                Part::Data {
                    data: serde_json::json!({"confidence": 0.99, "categories": ["A", "B"]}),
                    mime_type: None,
                },
            ],
        },
        &kp,
    ));

    // 21. Artifact with no artifact_id (anonymous)
    entries.push(make_artifact_entry(
        "artifact_no_id",
        &A2aArtifact {
            artifact_id: None,
            name: None,
            parts: vec![Part::Text {
                text: "Anonymous artifact content.".to_string(),
            }],
        },
        &kp,
    ));

    // 22. Task — unknown status (forward-compat test)
    entries.push(make_task_entry(
        "task_unknown_status",
        &Task {
            id: "task-fixture-009".to_string(),
            context_id: "ctx-fixture-009".to_string(),
            status: "unknown".to_string(),
            history: None,
            artifacts: None,
        },
        &kp,
    ));

    entries
}

// ── Emitter ──────────────────────────────────────────────────────────────────

/// Emit the A2A fixture JSON to stdout. Marked `#[ignore]` so default
/// `cargo test --features "golden-fixtures a2a-experimental"` does not produce
/// noisy output — run explicitly with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn emit_fixtures() {
    let entries = build_all_entries();
    let json = serde_json::to_string_pretty(&entries).expect("serialize A2A fixtures");
    println!("{}", json);
}

// ── Determinism + sanity guards ───────────────────────────────────────────────

/// Building the fixture list twice in one process must produce byte-identical JSON.
#[test]
fn test_emitter_deterministic() {
    let a = serde_json::to_string_pretty(&build_all_entries()).expect("a");
    let b = serde_json::to_string_pretty(&build_all_entries()).expect("b");
    assert_eq!(a, b, "A2A golden fixture emitter must be deterministic");
}

/// Catalogue must have at least 20 entries with unique names.
#[test]
fn test_fixture_count_and_unique_names() {
    let entries = build_all_entries();
    assert!(
        entries.len() >= 20,
        "must have at least 20 A2A fixture entries, got {}",
        entries.len()
    );
    let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    let original_len = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        original_len,
        "A2A fixture names must be unique"
    );
}

/// Every entry must produce a non-empty COSE envelope and JCS hex.
#[test]
fn test_all_entries_non_empty() {
    for entry in build_all_entries() {
        assert!(
            !entry.jcs_canonical_hex.is_empty(),
            "entry '{}' must have non-empty jcs_canonical_hex",
            entry.name
        );
        assert!(
            !entry.cose_signed_hex.is_empty(),
            "entry '{}' must have non-empty cose_signed_hex",
            entry.name
        );
    }
}

/// The fixed seed must produce a stable pubkey (anchors the determinism contract).
#[test]
fn test_fixed_keypair_stable() {
    let kp = fixture_keypair();
    let pubkey = kp.pubkey().to_string();
    assert!(!pubkey.is_empty());
    let bytes = kp.to_bytes();
    assert_eq!(bytes.len(), 64);
    let recovered = Keypair::try_from(&bytes[..]).expect("from_bytes round-trip");
    assert_eq!(recovered.pubkey().to_string(), pubkey);
}
