//! Codec module -- CBOR/COSE canonical encoding for verifiable artifacts.
//!
//! Dual-layer encoding:
//! - **Internal (canonical):** CBOR + COSE -- deterministic, compact, cryptographically signed
//! - **External (API surface):** JSON -- developer-friendly, compatible with MCP
//!
//! This module is a prerequisite for the VAA (Verifiable Artifact Anchor) protocol.

pub mod canonical;
pub mod hash;
pub mod schema;
pub mod sign;

#[cfg(feature = "a2a-experimental")]
pub mod a2a;
