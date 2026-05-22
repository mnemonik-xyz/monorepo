//! Tiny helper for the macOS cross-language collision smoke test.
//!
//! Reads the `xyz.mnemonik.identity/default` entry from the host OS keychain
//! via `OsKeyStore` (i.e. the same code path `mnemonic-mcp` uses at startup)
//! and prints a one-line JSON summary to stdout:
//!
//! ```json
//! {"available":true,"present":true,"pubkey_base58":"..."}
//! ```
//!
//! - `available` reflects whether the platform keychain backend is reachable
//!   (D-Bus on Linux, Apple Keychain on macOS, Credential Manager on Windows).
//! - `present` is true iff an entry with the canonical service+account exists.
//! - `pubkey_base58` is included only when `present` is true.
//!
//! The companion Node-side reading is just `mnemonic identity status --json`.
//! Cross-language collision = `pubkey_base58` strings byte-equal.
//!
//! Exit codes:
//!   0  → keychain reachable, entry possibly absent (still success — caller
//!         can distinguish via `present`)
//!   2  → keychain unavailable (PlatformUnavailable / no backend)
//!   3  → unexpected error (I/O, JSON, etc.) — prints `{"error":"..."}` to stdout
//!
//! Invocation:
//! ```sh
//! cargo run --release --example keychain-read 2>/dev/null
//! ```
//!
//! NEVER prints the secret bytes, even if asked. Only the pubkey.

use mnemonic_core::identity::{KeyStore, OsKeyStore};

fn main() {
    let store = OsKeyStore::new();

    let available = match store.available() {
        Ok(true) => true,
        Ok(false) => {
            println!(r#"{{"available":false,"present":false}}"#);
            std::process::exit(2);
        }
        Err(e) => {
            println!(r#"{{"error":"available probe failed: {e}"}}"#);
            std::process::exit(3);
        }
    };

    match store.get() {
        Ok(Some(entry)) => {
            println!(
                r#"{{"available":{available},"present":true,"pubkey_base58":"{}"}}"#,
                entry.pubkey_base58
            );
            std::process::exit(0);
        }
        Ok(None) => {
            println!(r#"{{"available":{available},"present":false}}"#);
            std::process::exit(0);
        }
        Err(e) => {
            println!(r#"{{"error":"get failed: {e}"}}"#);
            std::process::exit(3);
        }
    }
}
