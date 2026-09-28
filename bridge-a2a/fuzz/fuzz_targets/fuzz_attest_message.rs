//! Cargo-fuzz target: feed arbitrary JSON to the A2A attestation pipeline and
//! assert no panic, stack overflow, or OOM occurs (STRIDE threat A6).
//!
//! # Purpose
//!
//! This target exercises the codec primitives that underpin A2A attestation:
//!
//! 1. Attempt to parse the input as a JSON value (`serde_json::from_slice`).
//! 2. Feed the raw bytes through `sign_cose` — which is the inner signing step
//!    used by `attest_*` once the A2A codec (`core::codec::a2a`) is added in
//!    Task 2.  Treats the input as a pre-canonicalized payload.
//! 3. Verify the resulting COSE_Sign1 envelope with `verify_artifact`.
//! 4. Exercise common A2A field-access paths on arbitrary JSON objects.
//!
//! When `core::codec::a2a` (Task 2) lands, extend this target:
//!   - `attest_message(&store, &msg, &kp, None)`
//!   - `attest_task(&store, &task, &kp, None)`
//!   - `attest_artifact(&store, &art, "fuzz-ctx", &kp, None)`
//!
//! Errors returned from any function are expected and acceptable; panics, stack
//! overflows, and OOMs are not.
//!
//! # CI invocation
//!
//! ```sh
//! # Triggered automatically on PRs touching core/src/codec/ or work/a2a-bridge/.
//! cargo +nightly fuzz run fuzz_attest_message -- -max_total_time=1800
//! ```
//!
//! The `-max_total_time=1800` flag limits the run to 30 minutes as required by
//! Task 8.  The seed corpus lives in `bridge-a2a/fuzz/corpus/fuzz_attest_message/`.
//!
//! # Threat model reference
//!
//! See `.claude/skills/project-knowledge/references/threat-model.md` row A6:
//! "Elevation of privilege — crafted A2A object triggers core panic or OOM".

#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use solana_sdk::signature::Keypair;

use mnemonic_core::codec::sign::{sign_cose, verify_artifact};

// ── Deterministic fixed keypair (same seed as golden-fixtures) ────────────────

fn fuzz_keypair() -> Keypair {
    let seed: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
        0x1d, 0x1e, 0x1f, 0x20,
    ];
    Keypair::new_from_array(seed)
}

// ── Fuzz entry point ──────────────────────────────────────────────────────────

fuzz_target!(|data: &[u8]| {
    // ── Step 1: JSON parse ────────────────────────────────────────────────────
    // Non-JSON or invalid UTF-8 input is not interesting for this target:
    // the A2A pipeline receives JSON-RPC request bodies, not binary blobs.
    let json_val: Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return,
    };

    let kp = fuzz_keypair();

    // ── Step 2: sign raw bytes with COSE_Sign1 ────────────────────────────────
    // Treat the raw input bytes as a pre-canonicalized payload.  In production,
    // `sign_cose` receives JCS-canonicalized A2A bytes; here we send arbitrary
    // valid-UTF-8 JSON to stress-test the COSE encoding / size limits.
    let cose_result = sign_cose(data, &kp);

    // ── Step 3: verify round-trip ─────────────────────────────────────────────
    // If signing succeeded, verify the envelope.  The verifier must not panic
    // regardless of payload content.
    if let Ok(ref cose_bytes) = cose_result {
        let _ = verify_artifact(cose_bytes, None);
    }

    // ── Step 4: A2A field-access stress ──────────────────────────────────────
    // Exercise the field-access patterns used by A2A bridge helpers on
    // arbitrary JSON structure.  None of these must panic.
    let obj = json_val.as_object();
    let _ = obj.and_then(|o| o.get("role")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("messageId")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("id")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("contextId")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("status")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("parts")).and_then(|v| v.as_array());
    let _ = obj.and_then(|o| o.get("artifacts")).and_then(|v| v.as_array());
    let _ = obj.and_then(|o| o.get("method")).and_then(|v| v.as_str());
    let _ = obj.and_then(|o| o.get("params")).and_then(|v| v.as_object())
                .and_then(|p| p.get("message")).and_then(|v| v.as_object());

    // ── Step 5: JSON round-trip ───────────────────────────────────────────────
    // Serialise back to bytes and reparse to catch any serde inconsistencies
    // with unusual JSON values (NaN-in-string, very long numbers, etc.).
    if let Ok(serialised) = serde_json::to_vec(&json_val) {
        let _: Result<Value, _> = serde_json::from_slice(&serialised);
    }
});
