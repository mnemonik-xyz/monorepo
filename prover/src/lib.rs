//! `mnemonic-prover` — PRODUCES intent–action correspondence certificates.
//!
//! Architectural split (decisions.md): proving lives here, **never** in
//! `mnemonic-core` (which is verify-only). Wave 1 ships `MockProver` +
//! `StubEvidence` so the stack runs end-to-end with no real crypto; Wave 2
//! swaps in the zigz backend + real evidence.
//!
//! All content is gated behind `correspondence-experimental` (which forwards to
//! `mnemonic-core/correspondence-experimental`), so the default
//! `cargo build --workspace` compiles this crate empty.
#![cfg_attr(not(feature = "correspondence-experimental"), allow(unused))]

#[cfg(feature = "correspondence-experimental")]
mod prover;
#[cfg(feature = "correspondence-experimental")]
pub use prover::*;
