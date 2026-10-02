//! Cross-ecosystem parity test: Rust keccak256 == TypeScript keccak256
//!
//! Reads the reproducible fixture produced in Wave 3
//! (`packages/sdk/test/fixtures/erc8004-feedback-v1.json`) and recomputes
//! both `feedbackHash = keccak256(JCS(document))` and the ABI selector using
//! `alloy_primitives` — already a non-optional dependency of `mnemonic-mcp`.
//!
//! Zero new dependencies. The fixture is language-neutral; the two-command
//! shell check documented in the fixture header is equivalent to this test:
//!
//!   echo -n '<documentJson>' | cast keccak  →  must equal feedbackHash
//!
//! Any future Rust path that produces `MNEMONIC_FEEDBACK_V1` documents MUST
//! keep its output byte-for-byte consistent with the TypeScript `prepareFeedback`.

use alloy_primitives::keccak256;

/// Path to the fixture, relative to this crate's manifest directory (`mcp/`).
/// The workspace root is one level up from `mcp/`.
const FIXTURE_RELATIVE: &str = "../packages/sdk/test/fixtures/erc8004-feedback-v1.json";

/// The expected 4-byte selector for `giveFeedback`.
/// Source: `docs/spec/erc8004-feedback-v1.md`, pinned in Wave 1.
const EXPECTED_SELECTOR: [u8; 4] = [0x3c, 0x03, 0x6a, 0x7e];

/// `giveFeedback` canonical function signature.
const GIVE_FEEDBACK_SIG: &str =
    "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)";

// ── helpers ──────────────────────────────────────────────────────────────────

/// Parse a `0x`-prefixed lowercase hex string into bytes.
fn parse_hex(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex byte"))
        .collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Resolve the fixture path relative to the crate manifest directory.
/// `CARGO_MANIFEST_DIR` is set by Cargo at compile time to the directory
/// containing `mcp/Cargo.toml`, so `../packages/sdk/...` reaches the workspace.
fn fixture_path() -> std::path::PathBuf {
    let manifest =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo test");
    std::path::Path::new(&manifest).join(FIXTURE_RELATIVE)
}

#[test]
fn erc8004_feedback_hash_parity() {
    // 1. Load the fixture.
    let path = fixture_path();
    let fixture_json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));

    let fixture: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("fixture is valid JSON");

    // 2. Extract the pre-serialised document bytes (`documentJson` is already
    //    the JCS-canonical JSON string — no re-serialisation needed).
    let document_json = fixture["documentJson"]
        .as_str()
        .expect("fixture.documentJson must be a string");

    // 3. Recompute feedbackHash = keccak256(UTF-8 bytes of documentJson).
    let computed_hash = keccak256(document_json.as_bytes());
    let computed_hex = format!("0x{}", hex::encode(computed_hash.as_slice()));

    // 4. Compare with the pinned value in the fixture.
    let expected_hex = fixture["feedbackHash"]
        .as_str()
        .expect("fixture.feedbackHash must be a string");

    assert_eq!(
        computed_hex, expected_hex,
        "Rust keccak256(documentJson) must equal TypeScript feedbackHash.\n\
         Computed: {computed_hex}\n\
         Expected: {expected_hex}\n\
         This means the TS prepareFeedback and the Rust path are not byte-for-byte identical."
    );
}

#[test]
fn erc8004_payload_hash_parity() {
    // The fixture also pins payloadHash = keccak256(JCS(document.feedback)).
    // `documentJson` is the whole document; we can verify payloadHash by
    // parsing the document and re-serialising with JCS (sorted-key JSON).
    let path = fixture_path();
    let fixture_json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));

    let fixture: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("fixture is valid JSON");

    // Parse the full document from documentJson.
    let document: serde_json::Value = serde_json::from_str(
        fixture["documentJson"]
            .as_str()
            .expect("documentJson is a string"),
    )
    .expect("documentJson is valid JSON");

    // Extract the feedback object and re-serialise with sorted keys (JCS).
    let feedback = &document["feedback"];
    let feedback_jcs = jcs_serialize(feedback);

    let computed_hash = keccak256(feedback_jcs.as_bytes());
    let computed_hex = format!("0x{}", hex::encode(computed_hash.as_slice()));

    let expected_hex = fixture["payloadHash"]
        .as_str()
        .expect("fixture.payloadHash must be a string");

    assert_eq!(
        computed_hex, expected_hex,
        "Rust keccak256(JCS(feedback)) must equal TypeScript payloadHash.\n\
         Computed: {computed_hex}\n\
         Expected: {expected_hex}"
    );
}

#[test]
fn erc8004_give_feedback_selector_parity() {
    // Assert the 4-byte selector pinned in the fixture matches keccak256 of
    // the canonical function signature.
    let hash = keccak256(GIVE_FEEDBACK_SIG.as_bytes());
    let actual_selector: [u8; 4] = hash[..4].try_into().unwrap();

    assert_eq!(
        actual_selector,
        EXPECTED_SELECTOR,
        "giveFeedback selector mismatch.\n\
         Computed: 0x{}\n\
         Expected: 0x{}\n\
         Check that GIVE_FEEDBACK_SIG matches the pinned ABI.",
        hex::encode(actual_selector),
        hex::encode(EXPECTED_SELECTOR),
    );

    // Also assert it matches what the fixture says.
    let path = fixture_path();
    let fixture_json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&fixture_json).unwrap();
    let fixture_selector = fixture["onchain"]["selector"]
        .as_str()
        .expect("fixture.onchain.selector is a string");
    let fixture_bytes = parse_hex(fixture_selector);
    assert_eq!(
        actual_selector.as_ref(),
        fixture_bytes.as_slice(),
        "selector in fixture does not match keccak256 of GIVE_FEEDBACK_SIG"
    );
}

// ── JCS (RFC 8785) serialiser — minimal, sufficient for the MNEMONIC_FEEDBACK_V1 value space ──
//
// The MNEMONIC_FEEDBACK_V1 schema restricts values to strings, safe-range
// integers, booleans, objects and arrays — no floats, no null.  Under those
// constraints JCS is equivalent to sorting object keys recursively and
// calling `serde_json::to_string` with no whitespace.  We verify this
// assumption against the pinned fixture rather than assuming it.

fn jcs_serialize(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort(); // JCS: UTF-16 code-unit order = ASCII order for our restricted alphabet
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    let v = jcs_serialize(&map[*k]);
                    format!("\"{}\":{}", k, v)
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(jcs_serialize).collect();
            format!("[{}]", inner.join(","))
        }
        // Strings: serde_json's serialisation is JCS-compliant for the restricted
        // value space (no lone surrogates, no non-standard escapes).
        serde_json::Value::String(s) => {
            serde_json::to_string(s).expect("string serialisation cannot fail")
        }
        // Integers and booleans serialise identically in JCS and serde_json.
        other => serde_json::to_string(other).expect("primitive serialisation cannot fail"),
    }
}
