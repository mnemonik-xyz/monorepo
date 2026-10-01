//! `mnemonic-a2a` — pure-Rust adapter over `mnemonic-core` primitives for A2A
//! (Agent-to-Agent) protocol v1.0.0-rc objects.
//!
//! # One-way dependency rule
//!
//! This crate depends on `mnemonic-core` only. It never depends on `mcp/` or
//! `bridge-a2a/`. Downstream consumers that need to attest or recall A2A
//! objects should depend on this crate and not on `mnemonic-core::storage`
//! directly — the public surface here is the stable API boundary.
//!
//! # Relationship to A2A wire types
//!
//! The A2A wire types (`Message`, `Task`, `A2aArtifact`) live in
//! `mnemonic-core::codec::a2a` and are re-exported here so consumers only
//! need one `mnemonic-a2a` entry in their `Cargo.toml`.
//!
//! # Pipeline
//!
//! Each `attest_*` call runs:
//! `to_jcs_bytes()` → `sign_cose()` → `blake3` hash → `save_attestation` →
//! `set_context_id` → return `AttestationId`.
//!
//! `recall_by_context` is a thin wrapper around
//! `AttestationStore::recall_by_context`.
//!
//! `verify_a2a_attestation` verifies the COSE_Sign1 envelope and optionally
//! checks the signer pubkey.

mod attest;
mod recall;

// Re-export core A2A types so consumers do not need a direct `mnemonic-core`
// dependency for these types.
pub use mnemonic_core::codec::a2a::{A2aArtifact, Message, Task};
pub use mnemonic_core::storage::traits::AttestationRow as Attestation;
pub use mnemonic_core::storage::AttestationStore;

pub use attest::*;
pub use recall::recall_by_context;

/// An opaque attestation identifier (UUID v4 string).
pub type AttestationId = String;

/// The decoded, verified content of a `verify_a2a_attestation` call.
#[derive(Debug)]
pub struct VerifiedA2AContent {
    /// Raw JCS payload bytes extracted from the COSE_Sign1 envelope.
    pub payload: Vec<u8>,
    /// Base58 pubkey of the signer (from the COSE_Sign1 `kid` unprotected
    /// header).
    pub signer: String,
    /// blake3 hex digest of the payload bytes.
    pub content_hash: String,
    /// `true` iff the Ed25519 signature and (optional) expected-pubkey check
    /// both passed.
    pub valid: bool,
}

/// Extension of [`AttestationStore`] that supports the A2A `context_id`
/// association required by `attest_*` functions.
///
/// The base [`AttestationStore::save_attestation`] trait pre-dates the
/// `context_id` column; this supertrait adds `set_context_id` so the adapter
/// can populate it after the initial row write.
///
/// Implementors that store rows in-memory track context_id however they like;
/// the SQLite-backed [`mnemonic_core::storage::SqliteStore`] implementation
/// provided in this crate issues an `UPDATE attestations SET context_id = ?`.
pub trait A2aStore: AttestationStore {
    /// Associate `context_id` with the attestation row identified by
    /// `attestation_id`. Called once per `attest_*` invocation after the row
    /// has been written by [`AttestationStore::save_attestation`].
    fn set_context_id(&self, attestation_id: &str, context_id: &str) -> anyhow::Result<()>;
}

/// [`A2aStore`] implementation for the SQLite-backed store shipped with
/// `mnemonic-core`.
///
/// Issues a single `UPDATE attestations SET context_id = ?1 WHERE
/// attestation_id = ?2`.
#[cfg(not(target_arch = "wasm32"))]
impl A2aStore for mnemonic_core::storage::SqliteStore {
    fn set_context_id(&self, attestation_id: &str, context_id: &str) -> anyhow::Result<()> {
        use anyhow::Context as _;
        self.conn()
            .execute(
                "UPDATE attestations SET context_id = ?1 WHERE attestation_id = ?2",
                rusqlite::params![context_id, attestation_id],
            )
            .context("A2aStore::set_context_id (SqliteStore)")?;
        Ok(())
    }
}

/// Attest an A2A `Message`, storing it under the message's `contextId` (if
/// present).
///
/// # Arguments
/// * `store` — Any [`A2aStore`] implementation.
/// * `msg` — The A2A [`Message`] to attest.
/// * `agent_keypair` — Solana [`solana_sdk::signature::Keypair`] that will
///   sign the COSE_Sign1 envelope.
/// * `prev_id` — Optional parent attestation ID for lineage tracking (passed
///   through to the envelope but not yet enforced cryptographically).
///
/// # Returns
/// The [`AttestationId`] (UUID v4) assigned to the new row.
pub fn attest_message<S: A2aStore>(
    store: &S,
    msg: &Message,
    agent_keypair: &solana_sdk::signature::Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    attest::attest_message_impl(store, msg, agent_keypair, prev_id)
}

/// Attest an A2A `Task`, storing it under the task's `contextId`.
///
/// # Arguments
/// * `store` — Any [`A2aStore`] implementation.
/// * `task` — The A2A [`Task`] to attest.
/// * `agent_keypair` — Keypair that signs the COSE_Sign1 envelope.
/// * `prev_id` — Optional parent attestation ID.
///
/// # Returns
/// The [`AttestationId`] assigned to the new row.
pub fn attest_task<S: A2aStore>(
    store: &S,
    task: &Task,
    agent_keypair: &solana_sdk::signature::Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    attest::attest_task_impl(store, task, agent_keypair, prev_id)
}

/// Attest an A2A `A2aArtifact`, storing it under the supplied `ctx_id`.
///
/// Unlike `Message` and `Task`, `A2aArtifact` does not carry an intrinsic
/// `contextId` field; callers must supply one explicitly.
///
/// # Arguments
/// * `store` — Any [`A2aStore`] implementation.
/// * `art` — The A2A [`A2aArtifact`] to attest.
/// * `ctx_id` — Explicit context identifier to associate this row with.
/// * `agent_keypair` — Keypair that signs the COSE_Sign1 envelope.
/// * `prev_id` — Optional parent attestation ID.
///
/// # Returns
/// The [`AttestationId`] assigned to the new row.
pub fn attest_artifact<S: A2aStore>(
    store: &S,
    art: &A2aArtifact,
    ctx_id: &str,
    agent_keypair: &solana_sdk::signature::Keypair,
    prev_id: Option<&str>,
) -> anyhow::Result<AttestationId> {
    attest::attest_artifact_impl(store, art, ctx_id, agent_keypair, prev_id)
}

/// Verify a COSE_Sign1 envelope produced by one of the `attest_*` functions.
///
/// # Arguments
/// * `envelope_bytes` — Raw COSE_Sign1 bytes.
/// * `expected_pubkey` — When `Some(pk)`, the signer pubkey in the `kid`
///   unprotected header must equal `pk`; mismatch sets `valid = false`.
///
/// # Returns
/// A [`VerifiedA2AContent`] describing the result.
pub fn verify_a2a_attestation(
    envelope_bytes: &[u8],
    expected_pubkey: Option<&str>,
) -> anyhow::Result<VerifiedA2AContent> {
    attest::verify_a2a_attestation_impl(envelope_bytes, expected_pubkey)
}

#[cfg(not(target_arch = "wasm32"))]
mod signed;
#[cfg(not(target_arch = "wasm32"))]
pub use signed::validate_signed_a2a;
