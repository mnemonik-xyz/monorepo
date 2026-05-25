//! Cross-language interop helper: run the full Rust `identity::ensure()`
//! bootstrap and print a one-line JSON summary so the
//! `tests/cross-lang/keychain.sh` script (and the CI `cross-lang-keychain`
//! job) can compare pubkeys against the Node CLI's `whoami --json` output
//! WITHOUT spinning up the full `mnemonic-mcp` server.
//!
//! Why a dedicated example instead of `cargo run -p mnemonic-mcp`?
//!
//!   - The mcp binary requires the embedder (FastEmbed model download or
//!     `OPENAI_API_KEY`), the pricing engine, the SQLite store, etc. —
//!     orders of magnitude more startup work than the cross-lang test
//!     needs.
//!   - The stdio JSON-RPC entry point reads from stdin until EOF and
//!     panics on dirty EOF in some edge cases on Linux CI runners. That
//!     panic was swallowed by `2>/dev/null` in the previous version of the
//!     script and surfaced only as `exit 1` with no diagnostics.
//!   - This example uses the SAME `identity::ensure()` code path as the
//!     mcp binary's startup, so the interop assertion is unchanged: if the
//!     Rust ensure() writes an entry, the Node `whoami` must read the same
//!     pubkey, and vice versa.
//!
//! Output (stdout):
//! ```json
//! {"pubkey":"3F5q...okYLAqf","storage":"OsKeychain"}
//! ```
//!
//! Exit codes:
//!   0 — ensure() succeeded; JSON written to stdout
//!   1 — ensure() failed; error written to stderr (NOT to stdout)
//!
//! NEVER prints the secret bytes — only the public key + storage label.

fn main() {
    match mnemonic_core::identity::ensure() {
        Ok(identity) => {
            // Compact one-liner so callers can `jq -r .pubkey` cleanly.
            println!(
                r#"{{"pubkey":"{}","storage":"{:?}"}}"#,
                identity.pubkey_base58, identity.storage
            );
        }
        Err(e) => {
            eprintln!("keychain-roundtrip: identity::ensure failed: {e:#}");
            std::process::exit(1);
        }
    }
}
