//! Implementation of `attest_*` and `verify_a2a_attestation`.
//!
//! Pipeline for each `attest_*` call:
//! 1. Serialize the A2A object to RFC 8785 JCS bytes via `to_jcs_bytes()`.
//! 2. Build a COSE_Sign1 envelope with `sign_cose()` (from `core::codec::sign`).
//! 3. Compute `blake3(jcs_bytes)` as the `content_hash`.
//! 4. Persist the row via `AttestationStore::save_attestation`.
//! 5. Associate `context_id` via `A2aStore::set_context_id`.
//! 6. Return the `AttestationId`.
//!
//! `verify_a2a_attestation` is the inverse: it calls `core::codec::sign::verify_artifact`
//! and optionally checks the signer pubkey.

use anyhow::Context;
use chrono::Utc;
use mnemonic_core::codec::sign::{sign_cose, verify_artifact};
use mnemonic_core::storage::{Visibility, WriteMode};
use solana_sdk::signature::{Keypair, Signer};
use uuid::Uuid;

use crate::{A2aArtifact, A2aStore, AttestationId, Message, Task, VerifiedA2AContent};

// ── internal helpers ────────────────────────────────────────────────────────

/// Shared inner function: runs steps 2-6 given pre-computed JCS bytes and a
/// context id.
fn attest_jcs<S: A2aStore>(
    store: &S,
    jcs_bytes: &[u8],
    context_id: &str,
    kind_tag: &str,
    agent_keypair: &Keypair,
    _prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    // Step 2: COSE_Sign1 envelope over the raw JCS bytes.
    let cose_bytes =
        sign_cose(jcs_bytes, agent_keypair).map_err(|e| anyhow::anyhow!("COSE sign: {e}"))?;

    // Step 3: blake3 content hash of the JCS payload bytes (not the envelope).
    let content_hash = mnemonic_core::codec::hash::hash_bytes(jcs_bytes);

    // Attestation metadata.
    let attestation_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let signer_pubkey = agent_keypair.pubkey().to_string();

    // Human-readable content: the JCS bytes as a UTF-8 string (JCS is always
    // valid UTF-8 since it produces ASCII/UTF-8 JSON).
    let content =
        std::str::from_utf8(jcs_bytes).context("JCS bytes are not valid UTF-8")?.to_string();

    // Tags: "a2a" marker + the kind sub-tag.
    let tags = vec!["a2a".to_string(), kind_tag.to_string()];

    // Hex-encode the COSE_Sign1 bytes to store in the `arweave_tx` column
    // so that callers can later re-verify via `verify_a2a_attestation`.
    let cose_hex = hex::encode(&cose_bytes);

    // Step 4: persist via the trait.
    store
        .save_attestation(
            &attestation_id,
            &content,
            &content_hash,
            &tags,
            &format!("local:{attestation_id}"),
            &cose_hex,
            &signer_pubkey,
            &signer_pubkey,
            &now,
            WriteMode::Local,
            Visibility::Private,
            &[], // no embedding for A2A rows
        )
        .context("save_attestation failed")?;

    // Step 5: associate context_id via the A2aStore extension.
    store
        .set_context_id(&attestation_id, context_id)
        .context("set_context_id failed")?;

    Ok(attestation_id)
}

// ── public impl fns (called from lib.rs) ────────────────────────────────────

pub fn attest_message_impl<S: A2aStore>(
    store: &S,
    msg: &Message,
    agent_keypair: &Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    let jcs = msg.to_jcs_bytes().context("Message::to_jcs_bytes")?;
    let ctx_id = msg
        .context_id
        .clone()
        .unwrap_or_else(|| format!("msg:{}", msg.message_id));
    attest_jcs(store, &jcs, &ctx_id, "a2a-message", agent_keypair, prev_id)
}

pub fn attest_task_impl<S: A2aStore>(
    store: &S,
    task: &Task,
    agent_keypair: &Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    let jcs = task.to_jcs_bytes().context("Task::to_jcs_bytes")?;
    attest_jcs(
        store,
        &jcs,
        &task.context_id,
        "a2a-task",
        agent_keypair,
        prev_id,
    )
}

pub fn attest_artifact_impl<S: A2aStore>(
    store: &S,
    art: &A2aArtifact,
    ctx_id: &str,
    agent_keypair: &Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    let jcs = art.to_jcs_bytes().context("A2aArtifact::to_jcs_bytes")?;
    attest_jcs(store, &jcs, ctx_id, "a2a-artifact", agent_keypair, prev_id)
}

pub fn verify_a2a_attestation_impl(
    envelope_bytes: &[u8],
    expected_pubkey: Option<&str>,
) -> anyhow::Result<VerifiedA2AContent> {
    let result =
        verify_artifact(envelope_bytes, None).map_err(|e| anyhow::anyhow!("verify: {e}"))?;

    let pubkey_ok = expected_pubkey
        .map(|pk| pk == result.signer)
        .unwrap_or(true);

    Ok(VerifiedA2AContent {
        payload: result.payload,
        signer: result.signer,
        content_hash: result.content_hash,
        valid: result.valid && pubkey_ok,
    })
}
