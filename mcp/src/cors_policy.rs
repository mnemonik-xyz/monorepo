//! CORS allow-origin policy for the public HTTP transport.
//!
//! Hotfix (post-T15): Anthropic's MCP connector, Cursor, and ChatGPT issue
//! preflight `OPTIONS` requests from their own first-party origins
//! (`https://claude.ai`, `https://cursor.sh`, `https://chatgpt.com`, etc.)
//! before completing the OAuth handshake. The previous narrow policy
//! (`allow_origin(["https://mnemonik.xyz"])`) returned an ACAO header that
//! did not match the request origin, so browsers blocked the response and
//! the connector reported "Couldn't reach the MCP server".
//!
//! [`is_allowed_cors_origin`] is consumed by
//! `tower_http::cors::AllowOrigin::predicate` in `main.rs::run_http`. Tests
//! live in `mcp/tests/cors.rs` and exercise this exact predicate via the
//! library facade re-export at `mnemonic_mcp::cors_policy::*`.

/// Returns `true` if the supplied request `Origin` header matches one of the
/// trusted MCP-client root domains. Used by tower-http's
/// `AllowOrigin::predicate` to drive the dynamic CORS allow-list.
///
/// Trust list:
///   - first-party webapp: `https://mnemonik.xyz` (apex + any subdomain)
///   - hosted Universal Paywall: `https://mnemonik-dev.github.io` (exact)
///   - Anthropic: `https://claude.ai` + `*.claude.ai`,
///     `https://anthropic.com` + `*.anthropic.com`
///   - Cursor: `https://cursor.sh` + `*.cursor.sh`, `https://cursor.com` +
///     `*.cursor.com`
///   - OpenAI: `https://chatgpt.com` + `*.chatgpt.com`,
///     `https://chat.openai.com`, `https://openai.com` + `*.openai.com`
///   - WindSurf / Codeium: `https://windsurf.com` + `*.windsurf.com`,
///     `https://codeium.com` + `*.codeium.com` (the Cascade web preview and
///     Codeium dashboard share the OAuth surface)
///
/// All entries are HTTPS-only — `http://` origins return `false` to keep
/// production traffic on TLS. Subdomain suffix-matching preserves a leading
/// dot to defeat the `evilclaude.ai` look-alike attack.
pub fn is_allowed_cors_origin(origin: &[u8]) -> bool {
    let s = match std::str::from_utf8(origin) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Localhost dev convenience — Playwright e2e tests, vite dev server,
    // and CLI smoke scripts run from `http://localhost:*` or
    // `http://127.0.0.1:*`. Loopback HTTP is not exposed to the public
    // internet, so allowing it does not widen the attack surface.
    // Boundary check: after "localhost" / "127.0.0.1" the next char must be
    // `:` (port), `/` (path), or end-of-string. Without this guard,
    // `http://localhost.evil.com` would prefix-match.
    let is_loopback_origin = |prefix: &str| -> bool {
        if let Some(rest) = s.strip_prefix(prefix) {
            rest.is_empty() || rest.starts_with(':') || rest.starts_with('/')
        } else {
            false
        }
    };
    if is_loopback_origin("http://localhost") || is_loopback_origin("http://127.0.0.1") {
        return true;
    }
    // HTTPS-only for non-loopback origins.
    let host = match s.strip_prefix("https://") {
        Some(h) => h,
        None => return false,
    };
    // Drop any port (production origins are 443 implicit).
    let host = host.split(':').next().unwrap_or(host);

    // Exact-match apex domains.
    let exact_allow = [
        "mnemonik.xyz",
        "mnemonik-dev.github.io",
        "claude.ai",
        "anthropic.com",
        "cursor.sh",
        "cursor.com",
        "chatgpt.com",
        "chat.openai.com",
        "openai.com",
        "windsurf.com",
        "codeium.com",
    ];
    if exact_allow.contains(&host) {
        return true;
    }
    // Subdomain matches — leading dot prevents `evilclaude.ai` masquerade.
    let suffix_allow = [
        ".mnemonik.xyz",
        ".claude.ai",
        ".anthropic.com",
        ".cursor.sh",
        ".cursor.com",
        ".chatgpt.com",
        ".openai.com",
        ".windsurf.com",
        ".codeium.com",
    ];
    suffix_allow.iter().any(|suf| host.ends_with(suf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_first_party_webapp() {
        assert!(is_allowed_cors_origin(b"https://mnemonik.xyz"));
        assert!(is_allowed_cors_origin(b"https://app.mnemonik.xyz"));
        assert!(is_allowed_cors_origin(b"https://mnemonik-dev.github.io"));
        assert!(!is_allowed_cors_origin(b"https://evil.github.io"));
    }

    #[test]
    fn allows_anthropic_and_subdomains() {
        assert!(is_allowed_cors_origin(b"https://claude.ai"));
        assert!(is_allowed_cors_origin(b"https://console.claude.ai"));
        assert!(is_allowed_cors_origin(b"https://api.anthropic.com"));
    }

    #[test]
    fn allows_cursor_and_chatgpt() {
        assert!(is_allowed_cors_origin(b"https://cursor.sh"));
        assert!(is_allowed_cors_origin(b"https://www.cursor.com"));
        assert!(is_allowed_cors_origin(b"https://chatgpt.com"));
        assert!(is_allowed_cors_origin(b"https://chat.openai.com"));
        assert!(is_allowed_cors_origin(b"https://platform.openai.com"));
    }

    #[test]
    fn allows_windsurf_and_codeium() {
        assert!(is_allowed_cors_origin(b"https://windsurf.com"));
        assert!(is_allowed_cors_origin(b"https://app.windsurf.com"));
        assert!(is_allowed_cors_origin(b"https://codeium.com"));
        assert!(is_allowed_cors_origin(b"https://www.codeium.com"));
    }

    #[test]
    fn rejects_lookalike_attack() {
        // `evilclaude.ai` has `claude.ai` as a substring but NOT as a
        // dot-separated suffix — the leading-dot guard rejects it.
        assert!(!is_allowed_cors_origin(b"https://evilclaude.ai"));
        assert!(!is_allowed_cors_origin(b"https://notmnemonik.xyz"));
    }

    #[test]
    fn rejects_http_scheme() {
        assert!(!is_allowed_cors_origin(b"http://claude.ai"));
        assert!(!is_allowed_cors_origin(b"http://mnemonik.xyz"));
    }

    #[test]
    fn allows_localhost_dev_origins() {
        // Playwright e2e + vite dev — loopback HTTP is acceptable.
        assert!(is_allowed_cors_origin(b"http://localhost:5173"));
        assert!(is_allowed_cors_origin(b"http://localhost:3000"));
        assert!(is_allowed_cors_origin(b"http://127.0.0.1:5173"));
        // But http on a non-loopback host stays blocked.
        assert!(!is_allowed_cors_origin(b"http://localhost.evil.com"));
    }

    #[test]
    fn rejects_unknown_origin() {
        assert!(!is_allowed_cors_origin(b"https://evil.example.com"));
        assert!(!is_allowed_cors_origin(b"null"));
        assert!(!is_allowed_cors_origin(b""));
    }
}
