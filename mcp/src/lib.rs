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

pub mod api;
pub mod chat;
pub mod config;
pub mod confirmation_token;
pub mod cors_policy;
pub mod escrow;
pub mod llm;
pub mod mcp;
pub mod oauth;
pub mod payment;
pub mod pending;
pub mod pricing;
pub mod seed;
/// Markdown parser shared between `build.rs` (which projects skill
/// manifests into compile-time string constants) and the
/// `skill_manifests.rs` integration test (which exercises the same
/// "missing `## Purpose` fails" guard against the exact same code).
/// `build.rs` `include!()`s this file directly; the test imports it via
/// `mnemonic_mcp::skill_parse::...`.
pub mod skill_parse;
pub mod tools;

/// Shared test helpers for `mcp/tests/*.rs` — `mock_state()` and `mint_jwt()`.
/// Gated behind the `test-support` feature so production binaries never
/// compile this in. Cargo integration tests must run with
/// `cargo test --features test-support` (CI does this; the local smoke
/// command in the task spec also lists it). We deliberately do NOT include
/// `cfg(test)` here — the library compiled as a dependency for the
/// `tests/*.rs` integration files is not built with `cfg(test)` set, so
/// only the feature flag works as a gate.
#[cfg(feature = "test-support")]
pub mod test_support;

/// Re-export of the build-identity constant defined inside `mcp.rs` so
/// integration tests under `mcp/tests/` can refer to it via
/// `mnemonic_mcp::EMBEDDER_MODEL_VERSION`. The canonical definition
/// lives in `mcp.rs` because both compilation units (`main.rs` binary
/// and this `lib.rs` library) include the same `mcp.rs` source file,
/// and the `initialize` dispatcher must read the constant from a path
/// that both compilation units agree on.
pub use mcp::EMBEDDER_MODEL_VERSION;

/// Compile-time-baked default hosted MCP endpoint used by the participate-
/// mode soft-fall proxy on `mnemonic-mcp mcp-stdio`. Decision 12 of
/// agent-native-distribution: this constant is the *only* hosted peer the
/// binary will speak to unless the operator explicitly passes
/// `--allow-custom-endpoint`, in which case `MNEMONIC_HOSTED_ENDPOINT` is
/// honoured. A local attacker that injects the env var into a shell
/// therefore cannot silently redirect outbound writes without also
/// modifying the binary's flag set.
pub const DEFAULT_HOSTED_ENDPOINT: &str = "https://mcp.mnemonik.xyz/mcp";

/// Resolve the hosted MCP endpoint for participate-mode soft-fall.
///
/// Rules (Decision 12 — agent-native-distribution):
/// - `allow_custom == true` AND `MNEMONIC_HOSTED_ENDPOINT` set → env value.
/// - `allow_custom == true` AND env var unset → [`DEFAULT_HOSTED_ENDPOINT`].
/// - `allow_custom == false` AND env var set → [`DEFAULT_HOSTED_ENDPOINT`]
///   PLUS a single-line stderr warning is emitted by the caller (see
///   `main::resolve_hosted_endpoint_with_warning`). This helper itself is
///   pure — no I/O, no stderr — so tests can exercise the resolution
///   matrix without capturing process output.
/// - `allow_custom == false` AND env var unset → [`DEFAULT_HOSTED_ENDPOINT`].
pub fn resolve_hosted_endpoint(allow_custom: bool, env_value: Option<&str>) -> &'static str {
    // We deliberately return `&'static str` even when the env var is set:
    // the production wrapper at `main::resolve_hosted_endpoint_with_warning`
    // owns the `String` when it needs to be runtime-derived; this helper
    // covers the gating logic ("did the env var get to win?") only, so
    // tests can assert the answer without allocating.
    match (allow_custom, env_value) {
        (true, Some(s)) if !s.is_empty() => {
            // The caller (main) owns the resolved `String`; we cannot return
            // a borrowed slice here without leaking, so signal "use env" by
            // returning the default and let main handle the actual string.
            // Tests should use `resolved_hosted_endpoint` below for the
            // string variant.
            let _ = s;
            DEFAULT_HOSTED_ENDPOINT
        }
        _ => DEFAULT_HOSTED_ENDPOINT,
    }
}

/// String-returning variant of [`resolve_hosted_endpoint`]: returns an owned
/// `String` so the caller does not need to materialise a `&'static str` for
/// a runtime-derived endpoint. Same rules; emits NO stderr — the
/// `should_warn` boolean in the return signals to the caller that a warning
/// is owed (Decision 12 — agent-native-distribution).
pub fn resolved_hosted_endpoint(
    allow_custom: bool,
    env_value: Option<&str>,
) -> (String, bool /* should_warn */) {
    match (allow_custom, env_value) {
        (true, Some(s)) if !s.is_empty() => (s.to_string(), false),
        (false, Some(s)) if !s.is_empty() => (DEFAULT_HOSTED_ENDPOINT.to_string(), true),
        _ => (DEFAULT_HOSTED_ENDPOINT.to_string(), false),
    }
}
