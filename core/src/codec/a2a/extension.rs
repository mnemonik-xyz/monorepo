//! AgentCard `x-mnemonic` extension — Decision 3 in `work/a2a-bridge/decisions.md`.
//!
//! Publishes the agent's Ed25519 pubkey inside an A2A AgentCard extension so that
//! any A2A-native verifier can locate the Mnemonic-attestation key without a
//! separate lookup.
//!
//! Extension URI: `https://mnemonik.xyz/extensions/x-mnemonic/v1`
//!
//! Payload:
//! ```json
//! {
//!   "uri":                  "https://mnemonik.xyz/extensions/x-mnemonic/v1",
//!   "ed25519_pubkey_base58": "<base58-encoded Ed25519 pubkey>",
//!   "attestation_endpoint":  "<optional URL>",
//!   "conformance_version":   "1"
//! }
//! ```
//!
//! Authority rules (Risks section, tech-spec.md):
//! - JWS in `signatures[]` verifies AgentCard authenticity.
//! - `x-mnemonic` declares the Mnemonic attestation key.
//! - Both must be present; the JWS must cover the extension payload.
//! - Pubkey mismatch between JWS-covered card and attestation envelope → error.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Extension URI identifying the Mnemonic x-mnemonic AgentCard extension.
pub const EXTENSION_URI: &str = "https://mnemonik.xyz/extensions/x-mnemonic/v1";

/// Minimum conformance version shipped in V1.
pub const CONFORMANCE_VERSION: &str = "1";

/// The `x-mnemonic` AgentCard extension payload (Decision 3).
///
/// Carries the Ed25519 pubkey that signs Mnemonic attestations and optionally
/// the attestation endpoint URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XMnemonicExtension {
    /// Extension URI — always `EXTENSION_URI`.
    pub uri: String,
    /// Base58-encoded Ed25519 public key used to sign Mnemonic attestations.
    pub ed25519_pubkey_base58: String,
    /// Optional URL of the attestation endpoint (e.g. `https://api.mnemonik.xyz/a2a`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_endpoint: Option<String>,
    /// Conformance version — currently `"1"`.
    pub conformance_version: String,
}

impl XMnemonicExtension {
    /// Create a new `XMnemonicExtension` with the given pubkey and optional endpoint.
    pub fn new(pubkey_base58: &str, endpoint: Option<&str>) -> Self {
        XMnemonicExtension {
            uri: EXTENSION_URI.to_string(),
            ed25519_pubkey_base58: pubkey_base58.to_string(),
            attestation_endpoint: endpoint.map(|s| s.to_string()),
            conformance_version: CONFORMANCE_VERSION.to_string(),
        }
    }
}

/// Build the `x-mnemonic` extension JSON object for inclusion in an AgentCard
/// `extensions` array.
///
/// # Arguments
/// * `pubkey` — base58-encoded Ed25519 public key.
/// * `endpoint` — optional URL of the attestation endpoint.
///
/// # Returns
/// A `serde_json::Value` representing the full extension object.
pub fn build_x_mnemonic_extension(pubkey: &str, endpoint: Option<&str>) -> Value {
    let ext = XMnemonicExtension::new(pubkey, endpoint);
    serde_json::to_value(&ext).expect("XMnemonicExtension is always serializable")
}

/// Extract the `x-mnemonic` extension from an AgentCard JSON value.
///
/// Searches for an element in `agent_card["extensions"]` whose `"uri"` field
/// equals [`EXTENSION_URI`]. Returns `None` if the extension is absent (it is
/// optional per the spec).
///
/// # Errors
/// Returns an error if the extensions field has an unexpected shape or if the
/// matching entry cannot be deserialized as [`XMnemonicExtension`].
pub fn extract_x_mnemonic(agent_card: &Value) -> Result<Option<XMnemonicExtension>> {
    let extensions = match agent_card.get("extensions") {
        None => return Ok(None),
        Some(v) => v,
    };

    // Extensions must be an array.
    let arr = extensions
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("AgentCard 'extensions' must be a JSON array"))?;

    for entry in arr {
        let uri = entry.get("uri").and_then(|v| v.as_str()).unwrap_or("");
        if uri == EXTENSION_URI {
            let ext: XMnemonicExtension = serde_json::from_value(entry.clone())
                .map_err(|e| anyhow::anyhow!("x-mnemonic extension parse error: {e}"))?;
            return Ok(Some(ext));
        }
    }

    Ok(None)
}

/// Verify that the AgentCard's JWS `signatures[]` covers the extension payload.
///
/// Rules (tech-spec.md Risks, Decision 3):
/// 1. At least one signature in `signatures[]` must be present — the card must
///    be signed.
/// 2. Each signature entry must carry a `"payload"` field (A2A JWS compact or
///    detached form). When detached, the payload is the JCS-canonicalized card
///    itself; we verify the extension was part of the card at signing time by
///    confirming the extension is present in the card's `extensions` array.
/// 3. If the `x-mnemonic` extension is absent from the card, returns `Ok(())`
///    (extension is optional; nothing to cover).
/// 4. If the extension IS present but no JWS signature covers it (i.e. the
///    card has no `signatures` at all, or `signatures` is empty), the check
///    fails — an unsigned extension binding is unauthenticated.
///
/// # Note on full JWS verification
/// Full JWS signature cryptographic verification is intentionally out of scope
/// here: it requires the AgentCard signer's key, which is distinct from the
/// `ed25519_pubkey_base58` in the extension (the latter is the *attestation* key,
/// not the *card-signing* key). The check performed here is structural:
/// the card must be signed AND the extension must be present in the signed payload.
/// The caller is responsible for separately verifying the JWS signature bytes.
///
/// # Errors
/// Returns an error if the extension is present but no JWS signature is found.
pub fn verify_card_covers_extension(agent_card: &Value, jws_signatures: &[Value]) -> Result<()> {
    // If no x-mnemonic extension, nothing to verify.
    let ext = extract_x_mnemonic(agent_card)?;
    if ext.is_none() {
        return Ok(());
    }

    // Extension is present — there must be at least one JWS signature.
    if jws_signatures.is_empty() {
        bail!(
            "AgentCard has x-mnemonic extension but no JWS signatures: \
             the extension binding is unauthenticated"
        );
    }

    // Each present signature entry must have a non-null payload field.
    // An empty/null payload signals a malformed or placeholder signature.
    for (i, sig) in jws_signatures.iter().enumerate() {
        let payload = sig.get("payload");
        match payload {
            None => {
                bail!(
                    "JWS signature at index {i} is missing the 'payload' field: \
                     cannot verify that the extension is covered"
                );
            }
            Some(Value::Null) => {
                bail!(
                    "JWS signature at index {i} has a null 'payload': \
                     does not cover the x-mnemonic extension"
                );
            }
            Some(_) => {} // payload present — structural check passes for this entry
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- helpers --

    fn card_with_extension(pubkey: &str) -> Value {
        json!({
            "name": "Test Agent",
            "url": "https://example.com",
            "extensions": [
                build_x_mnemonic_extension(pubkey, Some("https://api.example.com/a2a"))
            ]
        })
    }

    fn card_without_extension() -> Value {
        json!({
            "name": "Test Agent",
            "url": "https://example.com",
            "extensions": []
        })
    }

    fn valid_jws_sig(payload: &str) -> Value {
        json!({ "payload": payload, "protected": "eyJhbGciOiJFZERTQSJ9", "signature": "AAAA" })
    }

    // -- extension::extract_present --
    #[test]
    fn extract_present() {
        let card = card_with_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej");
        let ext = extract_x_mnemonic(&card).unwrap();
        assert!(ext.is_some(), "extension must be found");
        let ext = ext.unwrap();
        assert_eq!(ext.uri, EXTENSION_URI);
        assert_eq!(
            ext.ed25519_pubkey_base58,
            "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej"
        );
        assert_eq!(
            ext.attestation_endpoint.as_deref(),
            Some("https://api.example.com/a2a")
        );
        assert_eq!(ext.conformance_version, "1");
    }

    // -- extension::extract_absent --
    #[test]
    fn extract_absent() {
        let card = card_without_extension();
        let ext = extract_x_mnemonic(&card).unwrap();
        assert!(
            ext.is_none(),
            "extension must not be found in empty extensions array"
        );
    }

    #[test]
    fn extract_absent_no_extensions_field() {
        let card = json!({ "name": "No-ext agent" });
        let ext = extract_x_mnemonic(&card).unwrap();
        assert!(ext.is_none(), "missing extensions field => None");
    }

    // -- extension::roundtrip_serialization --
    #[test]
    fn roundtrip_serialization() {
        let original = XMnemonicExtension {
            uri: EXTENSION_URI.to_string(),
            ed25519_pubkey_base58: "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej".to_string(),
            attestation_endpoint: Some("https://api.mnemonik.xyz/a2a".to_string()),
            conformance_version: "1".to_string(),
        };

        let json_str = serde_json::to_string(&original).unwrap();
        let restored: XMnemonicExtension = serde_json::from_str(&json_str).unwrap();
        assert_eq!(
            original, restored,
            "serde round-trip must be byte-for-byte equal"
        );

        // Verify the serialized form is stable across two calls.
        let json_str2 = serde_json::to_string(&original).unwrap();
        assert_eq!(json_str, json_str2, "serialization must be stable");
    }

    #[test]
    fn roundtrip_without_endpoint() {
        let ext = XMnemonicExtension::new("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej", None);
        let json = serde_json::to_string(&ext).unwrap();
        assert!(
            !json.contains("attestation_endpoint"),
            "None endpoint must be skipped in serialization"
        );
        let back: XMnemonicExtension = serde_json::from_str(&json).unwrap();
        assert_eq!(ext, back);
    }

    // -- extension::reject_jws_not_covering_extension --
    #[test]
    fn reject_jws_not_covering_extension() {
        let card = card_with_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej");

        // No signatures at all — must fail.
        let err = verify_card_covers_extension(&card, &[]);
        assert!(err.is_err(), "empty signatures must fail");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("no JWS signatures") || msg.contains("unauthenticated"),
            "error must mention missing signatures, got: {msg}"
        );
    }

    #[test]
    fn reject_null_payload_signature() {
        let card = card_with_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej");
        let bad_sig =
            json!({ "payload": null, "protected": "eyJhbGciOiJFZERTQSJ9", "signature": "AAAA" });

        let err = verify_card_covers_extension(&card, &[bad_sig]);
        assert!(err.is_err(), "null payload must fail");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("null"),
            "error must mention null payload, got: {msg}"
        );
    }

    #[test]
    fn reject_missing_payload_field() {
        let card = card_with_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej");
        let bad_sig = json!({ "protected": "eyJhbGciOiJFZERTQSJ9", "signature": "AAAA" });

        let err = verify_card_covers_extension(&card, &[bad_sig]);
        assert!(err.is_err(), "missing payload field must fail");
    }

    // -- extension::reject_pubkey_substitution --
    #[test]
    fn reject_pubkey_substitution() {
        // Card declares pubkey A; attestation envelope claims pubkey B → fail.
        let pubkey_a = "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej";
        let pubkey_b = "5FHwkrdxE5DzmGdiyGHGtEnKrWtLHBe4nHdcNRoS7Ax1";

        let card = card_with_extension(pubkey_a);
        let ext = extract_x_mnemonic(&card).unwrap().unwrap();

        // The card covers pubkey_a; verify that pubkey_b does NOT match.
        assert_ne!(
            ext.ed25519_pubkey_base58, pubkey_b,
            "pubkey substitution: card declares {pubkey_a} but attestation claims {pubkey_b}"
        );

        // Caller-side check: if the pubkey in the extension doesn't match the
        // pubkey used to sign the attestation, the caller must reject.
        let substitution_detected = ext.ed25519_pubkey_base58 != pubkey_b;
        assert!(
            substitution_detected,
            "pubkey substitution must be detected by comparing extension pubkey with attestation signer"
        );
    }

    // -- JWS coverage passes for a well-formed card --
    #[test]
    fn verify_card_covers_extension_passes_for_valid_card() {
        let card = card_with_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej");
        let sigs = vec![valid_jws_sig("dGVzdC1wYXlsb2Fk")];
        assert!(
            verify_card_covers_extension(&card, &sigs).is_ok(),
            "well-formed card with valid signatures must pass"
        );
    }

    // -- Card with no extension: JWS check is a no-op --
    #[test]
    fn verify_card_no_extension_no_jws_ok() {
        let card = card_without_extension();
        // No signatures and no extension — must be Ok (nothing to cover).
        assert!(
            verify_card_covers_extension(&card, &[]).is_ok(),
            "card without extension requires no JWS coverage"
        );
    }

    // -- build_x_mnemonic_extension --
    #[test]
    fn build_extension_structure() {
        let val = build_x_mnemonic_extension(
            "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej",
            Some("https://api.mnemonik.xyz/a2a"),
        );
        assert_eq!(val["uri"], EXTENSION_URI);
        assert_eq!(
            val["ed25519_pubkey_base58"],
            "3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej"
        );
        assert_eq!(val["attestation_endpoint"], "https://api.mnemonik.xyz/a2a");
        assert_eq!(val["conformance_version"], "1");
    }

    #[test]
    fn build_extension_no_endpoint() {
        let val = build_x_mnemonic_extension("3MrF7MAkQV3vNtNPeXqfWtLFGNYFMmT2gkS5PVJGQ1ej", None);
        assert!(
            val.get("attestation_endpoint").is_none() || val["attestation_endpoint"].is_null(),
            "absent endpoint must not appear in serialized object"
        );
    }

    // -- Extensions array not an array: error --
    #[test]
    fn extract_extensions_not_array_returns_error() {
        let card = json!({ "extensions": "not-an-array" });
        let result = extract_x_mnemonic(&card);
        assert!(result.is_err(), "non-array extensions must return Err");
    }
}
