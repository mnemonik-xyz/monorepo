//! Cross-context isolation test: attest items in ctx_A and ctx_B; assert that
//! `recall_by_context(ctx_A)` returns only ctx_A rows and does not leak ctx_B.

mod common;
use common::InMemoryA2aStore;

use mnemonic_a2a::{attest_task, recall_by_context, Task};
use solana_sdk::signature::Keypair;

fn make_task(id: &str, ctx: &str) -> Task {
    Task {
        id: id.to_string(),
        context_id: ctx.to_string(),
        status: "completed".to_string(),
        history: None,
        artifacts: None,
    }
}

#[test]
fn context_isolation_a_does_not_see_b() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();

    let ctx_a = "ctx-isolation-A";
    let ctx_b = "ctx-isolation-B";

    // Attest 3 tasks in ctx_A, 2 tasks in ctx_B.
    for i in 0..3 {
        let t = make_task(&format!("task-a-{i}"), ctx_a);
        attest_task(&store, &t, &kp, None).unwrap();
    }
    for i in 0..2 {
        let t = make_task(&format!("task-b-{i}"), ctx_b);
        attest_task(&store, &t, &kp, None).unwrap();
    }

    // Recall ctx_A — must return exactly 3.
    let rows_a = recall_by_context(&store, ctx_a, None).unwrap();
    assert_eq!(rows_a.len(), 3, "ctx_A must have 3 rows");
    for row in &rows_a {
        let content: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        let task_id = content["id"].as_str().unwrap();
        assert!(
            task_id.starts_with("task-a-"),
            "ctx_A recall must not include ctx_B rows: got '{task_id}'"
        );
    }

    // Recall ctx_B — must return exactly 2.
    let rows_b = recall_by_context(&store, ctx_b, None).unwrap();
    assert_eq!(rows_b.len(), 2, "ctx_B must have 2 rows");
    for row in &rows_b {
        let content: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        let task_id = content["id"].as_str().unwrap();
        assert!(
            task_id.starts_with("task-b-"),
            "ctx_B recall must not include ctx_A rows: got '{task_id}'"
        );
    }
}

#[test]
fn unknown_context_returns_empty() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();

    let t = make_task("task-xyz", "ctx-known");
    attest_task(&store, &t, &kp, None).unwrap();

    let rows = recall_by_context(&store, "ctx-unknown", None).unwrap();
    assert!(rows.is_empty(), "unknown context must return empty vec");
}

#[test]
fn context_isolation_with_mixed_types() {
    let store = InMemoryA2aStore::new();
    let kp = Keypair::new();

    use mnemonic_a2a::{attest_artifact, attest_message, A2aArtifact, Message};
    use mnemonic_core::codec::a2a::Part;

    let ctx_a = "ctx-mixed-A";
    let ctx_b = "ctx-mixed-B";

    // Task in ctx_A.
    attest_task(&store, &make_task("t-a", ctx_a), &kp, None).unwrap();

    // Message in ctx_A.
    let msg = Message {
        role: "agent".to_string(),
        parts: vec![Part::Text { text: "hello".to_string() }],
        message_id: "m-a-001".to_string(),
        task_id: None,
        context_id: Some(ctx_a.to_string()),
    };
    attest_message(&store, &msg, &kp, None).unwrap();

    // Artifact in ctx_B.
    let art = A2aArtifact {
        artifact_id: Some("art-b-001".to_string()),
        name: None,
        parts: vec![Part::Text { text: "b content".to_string() }],
    };
    attest_artifact(&store, &art, ctx_b, &kp, None).unwrap();

    let rows_a = recall_by_context(&store, ctx_a, None).unwrap();
    assert_eq!(rows_a.len(), 2, "ctx_A must have 2 rows (task + message)");

    let rows_b = recall_by_context(&store, ctx_b, None).unwrap();
    assert_eq!(rows_b.len(), 1, "ctx_B must have 1 row (artifact)");
}
