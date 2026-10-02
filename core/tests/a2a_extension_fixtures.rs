//! Integration tests: verify the three AgentCard fixture files parse correctly
//! per their expected outcomes (Task 4 — TDD anchors).
//!
//! Fixtures live in `core/tests/fixtures/a2a/`.

#[cfg(feature = "a2a-experimental")]
mod fixture_tests {
    use mnemonic_core::codec::a2a::{extract_x_mnemonic, verify_card_covers_extension};
    use serde_json::Value;
    use std::path::Path;

    fn load_fixture(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/a2a")
            .join(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name} as JSON: {e}"))
    }

    fn signatures_from_card(card: &Value) -> Vec<Value> {
        card.get("signatures")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// Fixture 1: well-formed card with extension — extraction succeeds and JWS
    /// coverage check passes.
    #[test]
    fn fixture_card_with_extension_ok() {
        let card = load_fixture("agent_card_with_extension.json");
        let ext = extract_x_mnemonic(&card)
            .expect("extract must not error")
            .expect("extension must be present");

        assert_eq!(
            ext.uri, "https://mnemonik.xyz/extensions/x-mnemonic/v1",
            "extension URI must match"
        );
        assert_eq!(
            ext.ed25519_pubkey_base58,
            "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej"
        );
        assert_eq!(ext.conformance_version, "1");
        assert!(ext.attestation_endpoint.is_some());

        let sigs = signatures_from_card(&card);
        verify_card_covers_extension(&card, &sigs)
            .expect("well-formed card with valid signatures must pass coverage check");
    }

    /// Fixture 2: well-formed card without extension — extraction returns None,
    /// JWS coverage check is a no-op and passes.
    #[test]
    fn fixture_card_without_extension_ok() {
        let card = load_fixture("agent_card_without_extension.json");
        let ext = extract_x_mnemonic(&card).expect("extract must not error");
        assert!(ext.is_none(), "extension must not be found");

        let sigs = signatures_from_card(&card);
        verify_card_covers_extension(&card, &sigs)
            .expect("card without extension must pass coverage check trivially");
    }

    /// Fixture 3: well-formed card with extension but JWS signatures absent —
    /// coverage check MUST fail (security regression).
    #[test]
    fn fixture_card_extension_no_jws_must_fail() {
        let card = load_fixture("agent_card_extension_no_jws.json");

        // Extension must be present.
        let ext = extract_x_mnemonic(&card)
            .expect("extract must not error")
            .expect("extension must be present");
        assert_eq!(
            ext.ed25519_pubkey_base58,
            "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej"
        );

        // No signatures in this fixture — coverage check must fail.
        let sigs = signatures_from_card(&card);
        assert!(sigs.is_empty(), "fixture must have no signatures");

        let result = verify_card_covers_extension(&card, &sigs);
        assert!(
            result.is_err(),
            "extension present + no JWS must fail coverage check"
        );
    }
}
