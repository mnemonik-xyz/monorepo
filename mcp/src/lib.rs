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
pub mod chain_stats;
pub mod chat;
pub mod config;
pub mod confirmation_token;
pub mod cors_policy;
pub mod escrow;
pub mod llm;
pub mod mcp;
pub mod oauth;
pub mod payment;
pub mod paid_operation;
pub mod pending;
pub mod pricing;
pub mod publish;
pub mod seed;
/// Markdown parser shared between `build.rs` (which projects skill
/// manifests into compile-time string constants) and the
/// `skill_manifests.rs` integration test (which exercises the same
/// "missing `## Purpose` fails" guard against the exact same code).
/// `build.rs` `include!()`s this file directly; the test imports it via
/// `mnemonic_mcp::skill_parse::...`.
pub mod skill_parse;
pub mod tools;
pub mod universal_paywall;
// Verifiable-trajectories MCP tool handlers (attest_step / attest_verdict /
// verify_trajectory). Experimental; gated so default builds never compile it.
#[cfg(feature = "trajectory-experimental")]
pub mod trajectory_tools;

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
/// honoured **and validated**. A local attacker that injects the env var
/// into a shell therefore cannot silently redirect outbound writes without
/// also modifying the binary's flag set; and an attacker who can flip the
/// flag (via a compromised npm postinstall script, etc.) still cannot
/// redirect to arbitrary SSRF targets — only `https://` and loopback
/// `http://` URLs survive the validation gate.
pub const DEFAULT_HOSTED_ENDPOINT: &str = "https://mcp.mnemonik.xyz/mcp";

/// Reason the caller's `MNEMONIC_HOSTED_ENDPOINT` value was not honoured.
/// Returned by [`resolved_hosted_endpoint`] so the production caller can
/// emit a distinguishable stderr line per case. Decision 12 +
/// security-audit round 1 finding SAR5-M1 (agent-native-distribution Task
/// 5 round 2): the env var being rejected at the validation step is
/// different from the env var being silently ignored because the flag is
/// absent — both must be loud, but the operator-facing message text needs
/// to point at the different remedy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedEndpointWarning {
    /// Env var was honoured and the default was returned. No warning.
    None,
    /// `--allow-custom-endpoint` was absent but the env var was set, so
    /// the value was ignored. Caller should emit:
    /// `[mnemonik-mcp] ignoring MNEMONIC_HOSTED_ENDPOINT — use --allow-custom-endpoint to enable`
    FlagAbsent,
    /// `--allow-custom-endpoint` WAS set but the env value failed URL
    /// validation (non-https + non-loopback, or unparseable). Caller
    /// should emit:
    /// `[mnemonik-mcp] rejecting unsafe MNEMONIC_HOSTED_ENDPOINT — only https:// or http://127.0.0.1|localhost|[::1] are accepted`
    RejectedUnsafe,
}

/// Validate that a candidate hosted-endpoint URL is safe to use as a
/// participate-mode proxy destination. Decision 12 + SAR5-M1: only
/// production-shape (`https://`) and dev-loopback (`http://127.0.0.1`,
/// `http://localhost`, `http://[::1]`) URLs survive — `file://`,
/// cloud-metadata IPs (`http://169.254.169.254`), and arbitrary
/// attacker-controlled HTTPS URLs with credentials in the userinfo
/// component are all rejected.
///
/// Returns `true` when the URL passes. Rejection is silent — the caller
/// (production `main.rs`) emits the operator-facing stderr line via the
/// [`HostedEndpointWarning::RejectedUnsafe`] signal.
fn is_safe_hosted_endpoint(url: &str) -> bool {
    // Reject any URL containing userinfo (`user:pass@host`) regardless of
    // scheme — credentials in the URL would leak into reqwest error
    // messages and the JSON-RPC `data.last_error` field (SAR5-L1). The
    // detection works because the userinfo component, if present, sits
    // between `://` and the first `/` (path) or end-of-string and contains
    // an `@` that the host alone cannot contain.
    if let Some((_, after_scheme)) = url.split_once("://") {
        let host_and_rest = after_scheme.split('/').next().unwrap_or("");
        if host_and_rest.contains('@') {
            return false;
        }
    }
    if url.starts_with("https://") {
        return true;
    }
    // Loopback HTTP variants — exact prefix match against the canonical
    // forms a developer's localhost test infra produces. The trailing
    // character must be `/`, `:`, or end-of-string so that
    // `http://localhost.attacker.example` does NOT match.
    const LOOPBACK_PREFIXES: &[&str] = &["http://127.0.0.1", "http://localhost", "http://[::1]"];
    for prefix in LOOPBACK_PREFIXES {
        if let Some(rest) = url.strip_prefix(prefix) {
            // After the prefix the URL must either end, start a port
            // (`:`), or start a path (`/`). Anything else is a different
            // host that just happens to share the prefix.
            if rest.is_empty() || rest.starts_with(':') || rest.starts_with('/') {
                return true;
            }
        }
    }
    false
}

/// Resolve the hosted MCP endpoint for participate-mode soft-fall.
///
/// Decision 12 (Task 5) — gating logic in three layers:
/// 1. **No flag** → env var is fully ignored; default returned. When env
///    var was set, signal [`HostedEndpointWarning::FlagAbsent`] so the
///    operator notices their override was rejected.
/// 2. **Flag set, env var unset** → default returned.
/// 3. **Flag set, env var set** → URL passes through
///    [`is_safe_hosted_endpoint`]. On success, env value is returned with
///    no warning. On failure, default returned with
///    [`HostedEndpointWarning::RejectedUnsafe`] — SAR5-M1 closes the SSRF
///    attack surface for the case where an attacker can flip BOTH the
///    flag and the env var (e.g., a compromised npm postinstall script).
///
/// Empty env values are treated as "no override" so a shell snippet like
/// `MNEMONIC_HOSTED_ENDPOINT= mnemonic-mcp ...` does not surrender to the
/// default with a confusing warning.
///
/// Pure function: no I/O, no stderr — the production wrapper at
/// `main.rs` owns the operator-facing message text.
pub fn resolved_hosted_endpoint(
    allow_custom: bool,
    env_value: Option<&str>,
) -> (String, HostedEndpointWarning) {
    match (allow_custom, env_value) {
        (true, Some(s)) if !s.is_empty() => {
            if is_safe_hosted_endpoint(s) {
                (s.to_string(), HostedEndpointWarning::None)
            } else {
                (
                    DEFAULT_HOSTED_ENDPOINT.to_string(),
                    HostedEndpointWarning::RejectedUnsafe,
                )
            }
        }
        (false, Some(s)) if !s.is_empty() => (
            DEFAULT_HOSTED_ENDPOINT.to_string(),
            HostedEndpointWarning::FlagAbsent,
        ),
        _ => (
            DEFAULT_HOSTED_ENDPOINT.to_string(),
            HostedEndpointWarning::None,
        ),
    }
}

#[cfg(test)]
mod hosted_endpoint_validation_tests {
    use super::*;

    #[test]
    fn https_is_safe() {
        assert!(is_safe_hosted_endpoint("https://mcp.mnemonik.xyz/mcp"));
        assert!(is_safe_hosted_endpoint("https://example.com:8443/mcp"));
    }

    #[test]
    fn loopback_http_is_safe() {
        assert!(is_safe_hosted_endpoint("http://127.0.0.1/mcp"));
        assert!(is_safe_hosted_endpoint("http://127.0.0.1:3000/mcp"));
        assert!(is_safe_hosted_endpoint("http://localhost/mcp"));
        assert!(is_safe_hosted_endpoint("http://localhost:3000"));
        assert!(is_safe_hosted_endpoint("http://[::1]/mcp"));
        assert!(is_safe_hosted_endpoint("http://[::1]:3000"));
    }

    #[test]
    fn non_loopback_http_is_rejected() {
        // Plain HTTP without loopback is the SSRF attack surface.
        assert!(!is_safe_hosted_endpoint("http://attacker.example/mcp"));
        assert!(!is_safe_hosted_endpoint("http://example.com:80/mcp"));
        // Cloud-metadata IPs MUST NOT slip through.
        assert!(!is_safe_hosted_endpoint(
            "http://169.254.169.254/latest/user-data"
        ));
        assert!(!is_safe_hosted_endpoint("http://10.0.0.1/mcp"));
    }

    #[test]
    fn file_scheme_is_rejected() {
        assert!(!is_safe_hosted_endpoint("file:///etc/passwd"));
    }

    #[test]
    fn unparseable_or_empty_is_rejected() {
        assert!(!is_safe_hosted_endpoint("not-a-url"));
        assert!(!is_safe_hosted_endpoint(""));
        assert!(!is_safe_hosted_endpoint("ftp://example.com/mcp"));
    }

    #[test]
    fn lookalike_loopback_is_rejected() {
        // `http://localhost.attacker.example` shares the `http://localhost`
        // prefix but is a different host — the suffix check rejects it.
        assert!(!is_safe_hosted_endpoint(
            "http://localhost.attacker.example/mcp"
        ));
        assert!(!is_safe_hosted_endpoint(
            "http://127.0.0.1.attacker.example/mcp"
        ));
    }

    #[test]
    fn userinfo_credentials_are_rejected() {
        // SAR5-L1 (round-1 security audit): URLs with userinfo
        // (user:pass@host) leak credentials into reqwest error messages.
        // Reject at the validation layer regardless of scheme.
        assert!(!is_safe_hosted_endpoint(
            "https://user:secret@example.com/mcp"
        ));
        assert!(!is_safe_hosted_endpoint("http://user@127.0.0.1/mcp"));
        assert!(!is_safe_hosted_endpoint("http://admin:pwd@localhost/mcp"));
    }

    #[test]
    fn resolved_hosted_endpoint_matrix() {
        // Flag absent, env set → FlagAbsent warning, default returned.
        let (ep, w) = resolved_hosted_endpoint(false, Some("https://attacker.example/mcp"));
        assert_eq!(ep, DEFAULT_HOSTED_ENDPOINT);
        assert_eq!(w, HostedEndpointWarning::FlagAbsent);

        // Flag set, valid env → env returned, no warning.
        let (ep, w) = resolved_hosted_endpoint(true, Some("http://127.0.0.1:3000/mcp"));
        assert_eq!(ep, "http://127.0.0.1:3000/mcp");
        assert_eq!(w, HostedEndpointWarning::None);

        // Flag set, https env → env returned, no warning.
        let (ep, w) = resolved_hosted_endpoint(true, Some("https://test.local/mcp"));
        assert_eq!(ep, "https://test.local/mcp");
        assert_eq!(w, HostedEndpointWarning::None);

        // Flag set, unsafe env → RejectedUnsafe warning, default returned.
        let (ep, w) = resolved_hosted_endpoint(true, Some("http://attacker.example/mcp"));
        assert_eq!(ep, DEFAULT_HOSTED_ENDPOINT);
        assert_eq!(w, HostedEndpointWarning::RejectedUnsafe);

        // Flag set, file:// env → RejectedUnsafe warning.
        let (ep, w) = resolved_hosted_endpoint(true, Some("file:///etc/passwd"));
        assert_eq!(ep, DEFAULT_HOSTED_ENDPOINT);
        assert_eq!(w, HostedEndpointWarning::RejectedUnsafe);

        // Flag set, env unset → default, no warning.
        let (ep, w) = resolved_hosted_endpoint(true, None);
        assert_eq!(ep, DEFAULT_HOSTED_ENDPOINT);
        assert_eq!(w, HostedEndpointWarning::None);

        // Flag absent, env unset → default, no warning.
        let (ep, w) = resolved_hosted_endpoint(false, None);
        assert_eq!(ep, DEFAULT_HOSTED_ENDPOINT);
        assert_eq!(w, HostedEndpointWarning::None);
    }
}
