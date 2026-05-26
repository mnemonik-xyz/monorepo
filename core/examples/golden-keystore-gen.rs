//! Generator for the cross-language golden byte-equality fixture.
//!
//! Prints the canonical JSON representation of a `KeystoreEntry` built from
//! a deterministic 32-byte seed (`[0x42; 32]`) to stdout. The output must be
//! hardcoded verbatim into both the Rust test
//! (`core/src/identity/keystore.rs::golden_json_bytes_cross_language_canonical`)
//! and the Node test
//! (`packages/cli/test/identity/keystore.test.ts::cross-language canonical golden`).
//!
//! Invocation:
//! ```sh
//! cargo run --example golden-keystore-gen 2>/dev/null
//! ```
//!
//! Re-run this whenever the seed convention changes or the JSON field order is
//! updated. Commit the resulting string update alongside the test changes.
//!
//! Seed convention: Ed25519 key = `Keypair::new_from_array([0x42u8; 32])`.
//! The 64-byte `secret` field = `kp.to_bytes()` = first-32-bytes-seed ||
//! next-32-bytes-pubkey (Solana/ed25519_dalek layout). The `pubkey_base58`
//! field = `kp.pubkey().to_string()` (base58-encoded 32-byte public key).
//!
//! Field order: `secret` first, then `pubkey_base58` (Decision 13).

use mnemonic_core::identity::keystore::KeystoreEntry;
use solana_sdk::signature::{Keypair, Signer};

fn main() {
    let seed = [0x42u8; 32];
    let kp = Keypair::new_from_array(seed);
    let secret64 = kp.to_bytes();
    let pubkey = kp.pubkey().to_string();

    let entry = KeystoreEntry {
        secret: secret64,
        pubkey_base58: pubkey,
    };

    let json = serde_json::to_string(&entry).expect("serialization failed");
    println!("{}", json);
}
