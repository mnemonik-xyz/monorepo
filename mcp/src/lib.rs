//! Library re-exports for integration tests.
//!
//! `mnemonic-mcp` is primarily a binary (`src/main.rs` builds the server),
//! but Task 4 onwards needs integration tests under `mcp/tests/` to call
//! into `oauth::*` directly. Cargo only links a crate's `tests/` directory
//! against a `lib` target, so we expose a thin library face here that lists
//! the modules the binary already pulls in via `mod foo;` in `main.rs`.
//!
//! Both `main.rs` and this `lib.rs` declare the same modules — Rust treats
//! them as separate compilation units (binary + library) using the same
//! source files. To keep them in sync, only ever add a module in BOTH places
//! at once. The binary uses `mod oauth;` (private to the binary), while
//! tests use `mnemonic_mcp::oauth::*` via this library facade.
//!
//! No runtime behavior change: `cargo run -p mnemonic-mcp` still ships the
//! binary as before; `cargo test -p mnemonic-mcp` now also compiles and
//! runs `tests/oauth_flow.rs` etc.

pub mod chat;
pub mod config;
pub mod llm;
pub mod mcp;
pub mod oauth;
pub mod payment;
pub mod pricing;
pub mod seed;
pub mod tools;
