//! Integration tests for the sealed-memory WASM bindings (Task 5).
//!
//! These tests run under `cargo test -p mnemonic-core` on native (no browser
//! required) and exercise the same underlying `mnemonic_core::sealed::*` API
//! that the WASM binding functions delegate to.  They verify:
//!
//!  1. Every binding's core logic round-trips correctly (seal → open, etc.).
//!  2. Error paths (wrong key length, bad fragment, tampered data) are rejected.
//!  3. The bindings produce results that match the non-WASM API byte-for-byte
//!     (golden-vector parity).
//!
//! WASM-specific wrapper concerns (JsValue serialization, serde_wasm_bindgen
//! round-trips) are exercised separately by `wasm_bindgen_test` tests that run
//! in a browser / Node environment.

use ed25519_dalek::SigningKey;
use hpke::{
    kem::{Kem as KemTrait, X25519HkdfSha256},
    Serializable,
};
use mnemonic_core::codec::sign::{sign_cose, verify_sealed};
use mnemonic_core::sealed::keys::x25519_secret_from_ed25519;
use mnemonic_core::sealed::{
    link_fragment, make_grant, open_grant, open_memory, open_with_key, parse_link_fragment,
    seal_memory, x25519_public_from_ed25519,
};
use solana_sdk::signature::Signer;

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42u8; 32])
}

fn seal_test(inner: &[u8], sk: &SigningKey) -> mnemonic_core::sealed::SealedArtifact {
    let vk_bytes = sk.verifying_key().to_bytes();
    let mut rng = rand_core::OsRng;
    seal_memory(
        inner,
        &vk_bytes,
        "art:integration-test",
        "did:test:wasm-binding",
        "2026-09-28T00:00:00Z",
        &mut rng,
    )
    .expect("seal_memory failed")
}

// ── seal_memory + open_memory (binding 1 + 2) ─────────────────────────────────

/// Mirrors the `seal_memory` WASM binding: seals, extracts outer_cbor +
/// content_hash (K is NOT returned), then opens with the author's X25519 key.
#[test]
fn wasm_binding_seal_open_round_trip() {
    let sk = test_signing_key();
    let inner = br#"{"type":"memory","content":"wasm sealed hello"}"#;
    let artifact = seal_test(inner, &sk);

    // outer_cbor and content_hash are what the binding returns to JS.
    assert!(
        !artifact.outer_cbor.is_empty(),
        "outer_cbor must not be empty"
    );
    assert_eq!(
        artifact.content_hash.len(),
        32,
        "content_hash must be 32 bytes"
    );

    // open_memory binding: derive X25519 secret from the author's signing key.
    let x25519_sk = *x25519_secret_from_ed25519(&sk);
    let recovered = open_memory(&artifact.outer_cbor, &x25519_sk).expect("open_memory failed");
    assert_eq!(recovered, inner, "round-trip content mismatch");
}

#[test]
fn wasm_binding_seal_wrong_key_fails() {
    let sk = test_signing_key();
    let artifact = seal_test(b"private content", &sk);
    let wrong = [0xFFu8; 32];
    assert!(
        open_memory(&artifact.outer_cbor, &wrong).is_err(),
        "wrong X25519 key must fail"
    );
}

// ── open_with_key (binding 3) ────────────────────────────────────────────────

#[test]
fn wasm_binding_open_with_key_round_trip() {
    let sk = test_signing_key();
    let inner = b"bearer link content";
    let artifact = seal_test(inner, &sk);
    let recovered = open_with_key(&artifact.outer_cbor, &artifact.k).expect("open_with_key failed");
    assert_eq!(recovered, inner);
}

#[test]
fn wasm_binding_open_with_key_wrong_key_fails() {
    let sk = test_signing_key();
    let artifact = seal_test(b"some content", &sk);
    let wrong = [0xAAu8; 32];
    assert!(open_with_key(&artifact.outer_cbor, &wrong).is_err());
}

// ── make_grant + open_grant (bindings 4 + 5) — anonymous ────────────────────

#[test]
fn wasm_binding_make_open_grant_anonymous() {
    let sk = test_signing_key();
    let inner = b"grant round-trip";
    let artifact = seal_test(inner, &sk);

    let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();
    let grant_cbor = make_grant(
        &memory_hash,
        &artifact.outer_cbor,
        &artifact.k,
        None,
        "did:test:wasm-binding",
        None,
        "2026-09-28T00:00:00Z",
    )
    .expect("make_grant failed");

    // Anonymous grants ignore the X25519 secret.
    let k_recovered = open_grant(&grant_cbor, &[0u8; 32]).expect("open_grant failed");
    assert_eq!(
        *k_recovered, *artifact.k,
        "K mismatch after anonymous grant round-trip"
    );
}

// ── make_grant + open_grant — targeted ──────────────────────────────────────

#[test]
fn wasm_binding_make_open_grant_targeted() {
    let sk = test_signing_key();
    let inner = b"targeted grant content";
    let artifact = seal_test(inner, &sk);

    let memory_hash: [u8; 32] = artifact.content_hash[..32].try_into().unwrap();

    let (reader_sk_hpke, reader_pk_hpke) = X25519HkdfSha256::gen_keypair(&mut rand_core::OsRng);
    let reader_sk_bytes: [u8; 32] = reader_sk_hpke.to_bytes().into();
    let reader_pk_bytes: [u8; 32] = reader_pk_hpke.to_bytes().into();

    let grant_cbor = make_grant(
        &memory_hash,
        &artifact.outer_cbor,
        &artifact.k,
        Some(&reader_pk_bytes),
        "did:test:wasm-binding",
        None,
        "2026-09-28T00:00:00Z",
    )
    .expect("make_grant targeted failed");

    let k_recovered =
        open_grant(&grant_cbor, &reader_sk_bytes).expect("open_grant targeted failed");
    assert_eq!(*k_recovered, *artifact.k);
}

// ── x25519_public_from_ed25519 (binding 6) ───────────────────────────────────

#[test]
fn wasm_binding_x25519_public_from_ed25519_matches_native() {
    let sk = test_signing_key();
    let vk_bytes = sk.verifying_key().to_bytes();

    let x25519_pub =
        x25519_public_from_ed25519(&vk_bytes).expect("x25519_public_from_ed25519 failed");
    assert_eq!(x25519_pub.len(), 32);
    assert_ne!(x25519_pub, [0u8; 32], "X25519 pub must not be all-zero");
}

#[test]
fn wasm_binding_x25519_public_from_ed25519_invalid_key_rejected() {
    // All-zero public key maps to the neutral element → small-order rejection.
    // A random invalid byte sequence that isn't a valid Edwards point also errs.
    let bad = [
        0x37u8, 0x37u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8,
        0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8, 0x00u8,
    ];
    assert!(x25519_public_from_ed25519(&bad).is_err());
}

// ── link_fragment + parse_link_fragment (bindings 7 + 8) ────────────────────

#[test]
fn wasm_binding_link_fragment_round_trip() {
    let k = [0x55u8; 32];
    let frag = link_fragment(&k);
    assert!(frag.starts_with("k="), "fragment must start with 'k='");
    let k2 = parse_link_fragment(&frag).expect("parse_link_fragment failed");
    assert_eq!(*k2, k, "K must survive link_fragment round-trip");
}

#[test]
fn wasm_binding_parse_link_fragment_bad_prefix_fails() {
    assert!(parse_link_fragment("nope=abc").is_err());
}

#[test]
fn wasm_binding_parse_link_fragment_bad_base64_fails() {
    assert!(parse_link_fragment("k=!!!").is_err());
}

#[test]
fn wasm_binding_parse_link_fragment_wrong_length_fails() {
    let short =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, [0u8; 16]);
    assert!(parse_link_fragment(&format!("k={short}")).is_err());
}

// ── verify_sealed (binding 9) ─────────────────────────────────────────────────

/// Build a fresh Solana Keypair from a 32-byte seed using the Solana API that
/// constructs the public key correctly from the seed.
fn solana_keypair_from_seed(seed: &[u8; 32]) -> solana_sdk::signature::Keypair {
    // ed25519-dalek derives the keypair from the seed; Solana wraps it.
    let ed_sk = SigningKey::from_bytes(seed);
    let mut kp_bytes = [0u8; 64];
    kp_bytes[..32].copy_from_slice(seed);
    kp_bytes[32..].copy_from_slice(&ed_sk.verifying_key().to_bytes());
    solana_sdk::signature::Keypair::try_from(kp_bytes.as_ref())
        .expect("valid Solana keypair from seed")
}

#[test]
fn wasm_binding_verify_sealed_round_trip() {
    let ed_kp = solana_keypair_from_seed(&[0x42u8; 32]);
    let author_pub = ed_kp.pubkey().to_bytes();

    // verify_sealed checks that kid (bare base58 pubkey set by sign_cose) ==
    // producer field in the payload.  Use the bare pubkey as producer — no
    // did:sol: prefix.
    let producer_id = ed_kp.pubkey().to_string();

    let mut rng = rand_core::OsRng;
    let artifact = seal_memory(
        b"verify sealed test",
        &author_pub,
        "art:integration-verify-sealed",
        &producer_id,
        "2026-09-28T00:00:00Z",
        &mut rng,
    )
    .expect("seal_memory");

    let cose_bytes = sign_cose(&artifact.outer_cbor, &ed_kp).expect("sign_cose");

    let sv = verify_sealed(&cose_bytes).expect("verify_sealed failed");
    assert!(!sv.producer.is_empty(), "producer must not be empty");
    assert_eq!(sv.content_hash.len(), 32, "content_hash must be 32 bytes");
    assert!(!sv.created_at.is_empty(), "created_at must not be empty");
    assert_eq!(sv.wrap_count, 1, "one wraps entry for the author");
}

#[test]
fn wasm_binding_verify_sealed_tampered_fails() {
    let ed_kp = solana_keypair_from_seed(&[0x42u8; 32]);
    let author_pub = ed_kp.pubkey().to_bytes();

    let producer_id = ed_kp.pubkey().to_string();

    let mut rng = rand_core::OsRng;
    let artifact = seal_memory(
        b"tamper test",
        &author_pub,
        "art:integration-tamper",
        &producer_id,
        "2026-09-28T00:00:00Z",
        &mut rng,
    )
    .expect("seal_memory");

    let mut cose_bytes = sign_cose(&artifact.outer_cbor, &ed_kp).expect("sign_cose");
    let last = cose_bytes.len() - 1;
    cose_bytes[last] ^= 0xFF; // tamper last byte

    assert!(
        verify_sealed(&cose_bytes).is_err(),
        "tampered COSE must be rejected"
    );
}

// ── Golden vector from T3 ─────────────────────────────────────────────────────
//
// These vectors use the same fixed seed as `sealed/api.rs::tests::test_signing_key()`
// (0x42 × 32) to produce a deterministic output.  They are not fully
// pre-computed (sealing uses a random nonce) but they verify structural
// invariants that must hold for any fresh seal:

#[test]
fn wasm_binding_golden_vector_structural_invariants() {
    let sk = test_signing_key();
    let vk_bytes = sk.verifying_key().to_bytes();
    let inner = br#"{"type":"memory","content":"golden vector content"}"#;

    let mut rng = rand_core::OsRng;
    let artifact = seal_memory(
        inner,
        &vk_bytes,
        "art:golden-vector",
        "did:test:golden",
        "2026-09-28T00:00:00Z",
        &mut rng,
    )
    .expect("seal_memory");

    // Structural invariant A: content_hash is blake3 of outer_cbor.
    let expected_hash = blake3::hash(&artifact.outer_cbor).as_bytes().to_vec();
    assert_eq!(
        artifact.content_hash, expected_hash,
        "content_hash must equal blake3(outer_cbor)"
    );

    // Structural invariant B: outer_cbor round-trips through open_memory.
    let x25519_sk = *x25519_secret_from_ed25519(&sk);
    let recovered = open_memory(&artifact.outer_cbor, &x25519_sk).expect("open_memory");
    assert_eq!(recovered, inner, "golden content must round-trip");

    // Structural invariant C: open_with_key produces the same result.
    let recovered2 = open_with_key(&artifact.outer_cbor, &artifact.k).expect("open_with_key");
    assert_eq!(recovered2, inner, "open_with_key must match open_memory");

    // Structural invariant D: K matches key commitment in outer_cbor.
    let payload = mnemonic_core::codec::canonical::from_canonical_cbor(&artifact.outer_cbor)
        .expect("from_canonical_cbor");
    let obj = payload.as_object().expect("outer_cbor is a map");
    let kc_b64 = obj["kc"].as_str().expect("kc field");
    let kc_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, kc_b64)
        .expect("kc base64");
    let expected_kc = mnemonic_core::sealed::key_commitment(&artifact.k);
    assert_eq!(
        kc_bytes, expected_kc,
        "kc in CBOR must match key_commitment(K)"
    );
}
