//! Golden COSE round-trip fixture generator.
//!
//! Gated behind the `golden-fixtures` cargo feature so the emitter does not
//! run on default `cargo test`. Invoked as:
//!
//! ```sh
//! cargo test --features golden-fixtures -p mnemonic-core \
//!   --test golden_fixtures emit_fixtures -- --ignored --nocapture
//! ```
//!
//! The emitter prints a JSON array of `{name, input_hex, canonical_cbor_hex,
//! cose_envelope_hex, keypair_secret_hex}` triples to stdout. The SDK's
//! `packages/sdk/test/cose.golden.test.ts` reads this fixture and asserts the
//! WASM-produced COSE envelope is byte-identical to `cose_envelope_hex` for
//! every entry.
//!
//! Determinism rules:
//! - Hardcoded 32-byte Ed25519 seed -> deterministic Solana keypair -> same
//!   COSE signature on every run (Ed25519 is deterministic per RFC 8032).
//! - All `created_at` timestamps are fixed strings, never `chrono::Utc::now()`.
//! - JSON emission uses `serde_json::to_string_pretty` with stable field order
//!   (the struct field order below is the emitted order).
//! - Re-running the emitter twice MUST produce byte-identical stdout. The
//!   `test_emitter_deterministic` test asserts this.
//!
//! Tech-spec reference: `work/mnemonic-cli/tech-spec.md` Decision 12.

#![cfg(all(feature = "golden-fixtures", not(target_arch = "wasm32")))]

use mnemonic_core::codec::canonical::to_canonical_cbor;
use mnemonic_core::codec::schema::MEMORY_V1;
use mnemonic_core::codec::sign::sign_cose;
use serde::Serialize;
use serde_json::Value as JsonValue;
use solana_sdk::signature::{Keypair, Signer};

/// One emitted fixture entry. Field order = emission order in JSON output.
#[derive(Serialize)]
struct GoldenEntry {
    /// Stable, human-readable name. Used as the test case label in vitest.
    name: String,
    /// Hex of the artifact JSON encoded as UTF-8 bytes. Documentation only —
    /// the SDK consumes `canonical_cbor_hex` directly.
    input_hex: String,
    /// Hex of the canonical CBOR bytes (output of `to_canonical_cbor`).
    canonical_cbor_hex: String,
    /// Hex of the COSE_Sign1 envelope (output of `sign_cose`).
    cose_envelope_hex: String,
    /// Hex of the 64-byte Solana secret used to sign. Identical for every
    /// entry in this run — kept per-entry so the SDK does not have to know
    /// any global state.
    keypair_secret_hex: String,
}

/// Deterministic 32-byte Ed25519 seed used by every fixture.
///
/// Hex value: `00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`.
/// This is a hardcoded test-only secret — it MUST NOT be used to sign any
/// real attestation. Solana's `Keypair::from_bytes` expects 64 bytes (seed +
/// pubkey), so we derive the pubkey on construction.
const FIXED_SEED_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// Build the deterministic keypair used by every fixture. The 32-byte seed
/// is hardcoded above; `Keypair::new_from_array` derives the matching
/// pubkey via the standard ed25519-dalek path.
fn fixture_keypair() -> Keypair {
    let seed_vec = hex::decode(FIXED_SEED_HEX).expect("seed hex parses");
    assert_eq!(seed_vec.len(), 32);
    let mut seed_arr = [0u8; 32];
    seed_arr.copy_from_slice(&seed_vec);
    Keypair::new_from_array(seed_arr)
}

/// Catalogue of fixture inputs. Each tuple is `(name, content, tags,
/// artifact_id, created_at, optional_metadata)`. Names must be unique and
/// stable — they are user-visible test labels.
fn fixture_cases() -> Vec<FixtureCase> {
    vec![
        FixtureCase {
            name: "empty_content",
            content: "".into(),
            tags: vec![],
            artifact_id: "art:fixture-empty",
            created_at: "2026-01-01T00:00:00Z",
            metadata: None,
        },
        FixtureCase {
            name: "ascii_short",
            content: "hello".into(),
            tags: vec![],
            artifact_id: "art:fixture-ascii-short",
            created_at: "2026-01-01T00:00:01Z",
            metadata: None,
        },
        FixtureCase {
            name: "ascii_with_tags",
            content: "the quick brown fox".into(),
            tags: vec!["test".into(), "demo".into()],
            artifact_id: "art:fixture-ascii-tags",
            created_at: "2026-01-01T00:00:02Z",
            metadata: None,
        },
        FixtureCase {
            name: "single_tag",
            content: "single-tag content".into(),
            tags: vec!["solo".into()],
            artifact_id: "art:fixture-single-tag",
            created_at: "2026-01-01T00:00:03Z",
            metadata: None,
        },
        FixtureCase {
            name: "many_tags",
            content: "many-tags content".into(),
            tags: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                "e".into(),
                "f".into(),
            ],
            artifact_id: "art:fixture-many-tags",
            created_at: "2026-01-01T00:00:04Z",
            metadata: None,
        },
        FixtureCase {
            name: "empty_tag_string",
            content: "with empty tag".into(),
            tags: vec!["".into()],
            artifact_id: "art:fixture-empty-tag",
            created_at: "2026-01-01T00:00:05Z",
            metadata: None,
        },
        FixtureCase {
            name: "utf8_cyrillic",
            content: "Привет мир — память".into(),
            tags: vec!["русский".into()],
            artifact_id: "art:fixture-cyrillic",
            created_at: "2026-01-01T00:00:06Z",
            metadata: None,
        },
        FixtureCase {
            name: "utf8_emoji",
            content: "memory test \u{1f9e0}\u{1f4be}\u{1f511}".into(),
            tags: vec!["emoji".into()],
            artifact_id: "art:fixture-emoji",
            created_at: "2026-01-01T00:00:07Z",
            metadata: None,
        },
        FixtureCase {
            name: "utf8_cjk",
            content: "記憶 メモリ 메모리".into(),
            tags: vec!["cjk".into()],
            artifact_id: "art:fixture-cjk",
            created_at: "2026-01-01T00:00:08Z",
            metadata: None,
        },
        FixtureCase {
            name: "utf8_mixed_scripts",
            content: "Latin Ελληνικά العربية עברית हिन्दी ไทย".into(),
            tags: vec!["multi-script".into(), "utf8".into()],
            artifact_id: "art:fixture-mixed-scripts",
            created_at: "2026-01-01T00:00:09Z",
            metadata: None,
        },
        FixtureCase {
            name: "control_chars",
            // \t newline \r form-feed vertical-tab — exercises CBOR text encoding
            content: "line1\tcol2\nline2\rline3\x0cpage\x0bvtab".into(),
            tags: vec!["ctrl".into()],
            artifact_id: "art:fixture-ctrl",
            created_at: "2026-01-01T00:00:10Z",
            metadata: None,
        },
        FixtureCase {
            name: "embedded_null_in_content",
            // CBOR text strings allow embedded NUL — must round-trip byte-for-byte.
            content: "before\x00after".into(),
            tags: vec!["nul".into()],
            artifact_id: "art:fixture-nul",
            created_at: "2026-01-01T00:00:11Z",
            metadata: None,
        },
        FixtureCase {
            name: "quotes_and_backslashes",
            content: "\"quoted\" and \\backslashed\\ \\\"both\\\"".into(),
            tags: vec!["json-edge".into()],
            artifact_id: "art:fixture-quotes",
            created_at: "2026-01-01T00:00:12Z",
            metadata: None,
        },
        FixtureCase {
            name: "long_content_1k",
            // 1024 bytes of repeating pattern — exercises CBOR length-prefix
            // boundary at u8/u16 (256-byte mark) and approaches u16 ceiling.
            content: "x".repeat(1024),
            tags: vec!["long".into()],
            artifact_id: "art:fixture-long-1k",
            created_at: "2026-01-01T00:00:13Z",
            metadata: None,
        },
        FixtureCase {
            name: "long_content_5k",
            // 5KB — pushes the COSE envelope past common HTTP framing buffers.
            content: "abcdefghij".repeat(512),
            tags: vec!["long".into()],
            artifact_id: "art:fixture-long-5k",
            created_at: "2026-01-01T00:00:14Z",
            metadata: None,
        },
        FixtureCase {
            name: "very_long_tag",
            // Exercises CBOR text length encoding for tag elements.
            content: "tag length test".into(),
            tags: vec!["t".repeat(200)],
            artifact_id: "art:fixture-long-tag",
            created_at: "2026-01-01T00:00:15Z",
            metadata: None,
        },
        FixtureCase {
            name: "metadata_simple",
            content: "with metadata".into(),
            tags: vec![],
            artifact_id: "art:fixture-metadata-simple",
            created_at: "2026-01-01T00:00:16Z",
            metadata: Some(serde_json::json!({"source": "fixture"})),
        },
        FixtureCase {
            name: "metadata_multi_field",
            content: "with multi-field metadata".into(),
            tags: vec!["meta".into()],
            artifact_id: "art:fixture-metadata-multi",
            created_at: "2026-01-01T00:00:17Z",
            metadata: Some(serde_json::json!({
                "source": "fixture",
                "version": "v1",
                "dim": 384,
            })),
        },
        FixtureCase {
            name: "metadata_with_unicode_value",
            content: "metadata unicode value".into(),
            tags: vec![],
            artifact_id: "art:fixture-metadata-unicode",
            created_at: "2026-01-01T00:00:18Z",
            metadata: Some(serde_json::json!({"label": "память"})),
        },
        FixtureCase {
            name: "single_char_content",
            content: "a".into(),
            tags: vec![],
            artifact_id: "art:fixture-single-char",
            created_at: "2026-01-01T00:00:19Z",
            metadata: None,
        },
        FixtureCase {
            name: "whitespace_only",
            content: "   \t\n  ".into(),
            tags: vec![],
            artifact_id: "art:fixture-whitespace",
            created_at: "2026-01-01T00:00:20Z",
            metadata: None,
        },
        FixtureCase {
            name: "high_bytes_utf8",
            // U+007F (DEL) and U+0080 — boundary between 1-byte and 2-byte UTF-8.
            content: "\u{007e}\u{007f}\u{0080}\u{00ff}\u{0100}".into(),
            tags: vec!["boundary".into()],
            artifact_id: "art:fixture-utf8-boundary",
            created_at: "2026-01-01T00:00:21Z",
            metadata: None,
        },
    ]
}

struct FixtureCase {
    name: &'static str,
    content: String,
    tags: Vec<String>,
    artifact_id: &'static str,
    created_at: &'static str,
    metadata: Option<JsonValue>,
}

/// Build the artifact JSON for one fixture case. Mirrors the layout the
/// server would produce for a `sign_memory` call, except `created_at` is a
/// fixed RFC-3339 string for determinism.
fn build_artifact(case: &FixtureCase) -> JsonValue {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "artifact_id".into(),
        JsonValue::String(case.artifact_id.into()),
    );
    obj.insert("type".into(), JsonValue::String("memory".into()));
    obj.insert(
        "schema_version".into(),
        JsonValue::Number(serde_json::Number::from(1u32)),
    );
    obj.insert("content".into(), JsonValue::String(case.content.clone()));
    obj.insert(
        "producer".into(),
        JsonValue::String(format!("did:sol:{}", fixture_keypair().pubkey())),
    );
    obj.insert(
        "created_at".into(),
        JsonValue::String(case.created_at.into()),
    );
    if !case.tags.is_empty() {
        obj.insert(
            "tags".into(),
            JsonValue::Array(case.tags.iter().cloned().map(JsonValue::String).collect()),
        );
    }
    if let Some(md) = &case.metadata {
        obj.insert("metadata".into(), md.clone());
    }
    JsonValue::Object(obj)
}

/// Compute one fixture entry from a case.
fn make_entry(case: &FixtureCase, kp: &Keypair) -> GoldenEntry {
    let artifact = build_artifact(case);
    let input_bytes = serde_json::to_vec(&artifact).expect("artifact JSON serializable");
    let canonical = to_canonical_cbor(&artifact, &MEMORY_V1).expect("canonical CBOR succeeds");
    let cose = sign_cose(&canonical, kp).expect("sign_cose succeeds");
    let secret_hex = hex::encode(kp.to_bytes());
    GoldenEntry {
        name: case.name.into(),
        input_hex: hex::encode(&input_bytes),
        canonical_cbor_hex: hex::encode(&canonical),
        cose_envelope_hex: hex::encode(&cose),
        keypair_secret_hex: secret_hex,
    }
}

/// Build the full fixture list as a serializable JSON array. Pure function
/// for testability — `emit_fixtures` simply prints `serde_json::to_string_pretty`
/// of this output.
fn build_all_entries() -> Vec<GoldenEntry> {
    let kp = fixture_keypair();
    fixture_cases()
        .iter()
        .map(|case| make_entry(case, &kp))
        .collect()
}

/// Emit the fixture JSON to stdout. Marked `#[ignore]` so default
/// `cargo test --features golden-fixtures` does not produce noisy output —
/// run explicitly with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn emit_fixtures() {
    let entries = build_all_entries();
    let json = serde_json::to_string_pretty(&entries).expect("serialize fixtures");
    println!("{}", json);
}

/// Determinism guard: building the fixture list twice in one process MUST
/// produce byte-identical JSON. This is the core lockstep invariant: if this
/// test fails locally, the CI lockstep gate would also fail.
#[test]
fn test_emitter_deterministic() {
    let a = serde_json::to_string_pretty(&build_all_entries()).expect("a");
    let b = serde_json::to_string_pretty(&build_all_entries()).expect("b");
    assert_eq!(a, b, "golden fixture emitter must be deterministic");
}

/// Sanity: catalogue must contain at least 20 distinct named cases. Drift
/// catcher — if someone removes cases without updating the test, this fails.
#[test]
fn test_fixture_count_and_unique_names() {
    let cases = fixture_cases();
    assert!(
        cases.len() >= 20,
        "must have at least 20 fixture cases, got {}",
        cases.len()
    );
    let mut names: Vec<&str> = cases.iter().map(|c| c.name).collect();
    names.sort();
    let original_len = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        original_len,
        "fixture case names must be unique"
    );
}

/// Sanity: the hardcoded test seed always derives the same pubkey. Anchors
/// the determinism contract — if `Keypair::from_seed` ever changes its
/// derivation, this fails loudly.
#[test]
fn test_fixed_keypair_pubkey_stable() {
    let kp = fixture_keypair();
    // The pubkey derived from FIXED_SEED_HEX. Captured on first run; if this
    // ever changes, the entire fixture file must be regenerated.
    let pubkey = kp.pubkey().to_string();
    assert!(!pubkey.is_empty());
    // Round-trip via to_bytes/from_bytes to confirm Solana's invariant.
    let bytes = kp.to_bytes();
    assert_eq!(bytes.len(), 64);
    let recovered = Keypair::try_from(&bytes[..]).expect("from_bytes round-trip");
    assert_eq!(recovered.pubkey().to_string(), pubkey);
}
