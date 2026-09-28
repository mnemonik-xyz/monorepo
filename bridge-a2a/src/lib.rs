//! `bridge-a2a` library — public API surface for integration tests.
//!
//! The binary (`main.rs`) re-uses these modules.  Tests import from the lib
//! target so they can call `attest_message_send` etc. with custom stores.

pub mod attest;
pub mod config;
pub mod idem;
pub mod lineage;
pub mod middleware;
pub mod state;
