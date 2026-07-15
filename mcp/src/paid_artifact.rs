//! Versioned binding for a paid, client-signed artifact.
//!
//! A payment must commit to the exact COSE_Sign1 envelope that the client
//! produced, rather than to editor content or to an unsigned CBOR payload.
//! The domain separator makes this hash unambiguous and leaves room for a
//! future envelope format without changing the meaning of existing receipts.

/// Current paid-artifact binding format.
pub const PAID_ARTIFACT_BINDING_VERSION: u8 = 1;

const DOMAIN_SEPARATOR: &[u8] = b"mnemonic:paid-artifact:v1\0";

/// Return the canonical `artifact_hash` for an exact-payment binding.
///
/// `cose_sign1` must be the exact envelope returned by the client's signing
/// key. Hashing the envelope (rather than just its payload) commits to the
/// signature, protected headers, and signer key identifier as well as the
/// canonical artifact bytes.
pub fn hash_client_signed_cose(cose_sign1: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_SEPARATOR);
    hasher.update(cose_sign1);
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_is_deterministic_and_domain_separated() {
        let cose = b"a client-signed cose envelope";
        assert_eq!(hash_client_signed_cose(cose), hash_client_signed_cose(cose));
        assert_ne!(
            hash_client_signed_cose(cose),
            blake3::hash(cose).to_hex().to_string()
        );
    }

    #[test]
    fn any_signed_envelope_change_invalidates_the_binding() {
        assert_ne!(
            hash_client_signed_cose(b"cose-envelope-a"),
            hash_client_signed_cose(b"cose-envelope-b")
        );
    }
}
