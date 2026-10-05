#![cfg(feature = "a2a-experimental")]

use mnemonic_core::codec::a2a::signed::{
    open_signed_a2a, open_signed_a2a_full, verify_a2a_inner, verify_signed_a2a,
};
use mnemonic_core::codec::sign::sign_cose;
use serde_json::Value;
use solana_sdk::signature::{Keypair, Signer};

#[test]
fn committed_native_vectors_verify_open_and_reproduce_signed_bytes() {
    let v: Value = serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a.json"
    ))
    .unwrap();
    let author = Keypair::new_from_array([1; 32]);
    let reader = Keypair::new_from_array([2; 32]);
    let outsider = Keypair::new_from_array([3; 32]);
    let expected = author.pubkey().to_string();
    for kind in ["plain", "sealed", "stream"] {
        let bytes = hex::decode(v[kind].as_str().unwrap()).unwrap();
        let verified = verify_signed_a2a(&bytes, Some(&expected)).unwrap();
        let jcs = serde_jcs::to_vec(&verified.binding).unwrap();
        assert_eq!(sign_cose(&jcs, &author).unwrap(), bytes);
        assert_eq!(
            open_signed_a2a(&bytes, &reader, &expected, None).unwrap(),
            v["payload"]
        );
        if kind != "plain" {
            let opened = open_signed_a2a_full(&bytes, &reader, &expected, None).unwrap();
            let inner = verify_a2a_inner(&opened.inner_signed.unwrap(), &expected).unwrap();
            assert_eq!(inner.recipients, vec![reader.pubkey().to_string()]);
            assert_eq!(hex::encode(jcs), v[format!("{kind}_jcs")].as_str().unwrap());
            assert!(open_signed_a2a(&bytes, &outsider, &expected, None).is_err());
        }
    }
}

/// V1 sealed envelopes predate sign-encrypt-sign. They still verify and open,
/// but carry no inner signature over the plaintext and recipients.
#[test]
fn legacy_v1_sealed_vectors_still_open_without_inner_binding() {
    use mnemonic_core::codec::a2a::signed::PROTOCOL_V1;
    let v: Value = serde_json::from_str(include_str!(
        "../../packages/sdk/test/fixtures/sealed-a2a-v1-legacy.json"
    ))
    .unwrap();
    let author = Keypair::new_from_array([1; 32]);
    let reader = Keypair::new_from_array([2; 32]);
    let expected = author.pubkey().to_string();
    for kind in ["sealed", "stream"] {
        let bytes = hex::decode(v[kind].as_str().unwrap()).unwrap();
        let verified = verify_signed_a2a(&bytes, Some(&expected)).unwrap();
        assert_eq!(verified.binding.protocol, PROTOCOL_V1);
        let opened = open_signed_a2a_full(&bytes, &reader, &expected, None).unwrap();
        assert_eq!(opened.payload, v["payload"]);
        assert!(opened.inner_signed.is_none());
    }
}
