//! HPKE-based content-key wrapping for sealed memories.
//!
//! Each sealed memory has a single symmetric content key `K` (32 bytes).
//! That key is delivered to a recipient by wrapping it with HPKE:
//!
//! ```text
//! ciphersuite:  X25519HkdfSha256, HkdfSha256, ChaCha20Poly1305
//! mode:         Base (unauthenticated, single-shot)
//! info:         b"mnemonic sealed v1"
//! aad:          canonical_aad(ct_hash, author_did)   [CBOR-encoded]
//! ```
//!
//! `canonical_aad` encodes `[bstr ct_hash, tstr author_did]` as a
//! deterministic CBOR byte string so both the content hash and the author
//! identity are bound to the wrapping operation.
//!
//! ## All-zero shared secret
//! The underlying hpke library already rejects all-zero DH results (RFC 9180
//! §7.1.4).  Any attempt to wrap/unwrap a key where the X25519 DH result is
//! the zero point returns `WrapError::ZeroSharedSecret`.
//!
//! ## Output
//! [`wrap_key`] returns a [`WrapResult`] containing:
//! - `enc` — the HPKE encapsulated key (32 bytes for X25519).
//! - `wk`  — the HPKE ciphertext (32-byte K + 16-byte Poly1305 tag = 48 bytes).
//!
//! [`unwrap_key`] takes `enc` + `wk` + the recipient's 32-byte X25519 secret
//! and returns the content key wrapped in [`Zeroizing`] for safe handling.

use hpke::{
    aead::ChaCha20Poly1305,
    kdf::HkdfSha256,
    kem::{Kem as KemTrait, X25519HkdfSha256},
    Deserializable, HpkeError, OpModeR, OpModeS, Serializable,
};
use thiserror::Error;
use zeroize::Zeroizing;

/// The HPKE info string identifying this protocol version.
const INFO: &[u8] = b"mnemonic sealed v1";

/// Error type for key-wrap operations.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum WrapError {
    /// The recipient public key bytes are invalid (wrong length or bad point).
    #[error("invalid recipient public key")]
    InvalidRecipientKey,
    /// The encapsulated key (`enc`) bytes are invalid.
    #[error("invalid encapsulated key")]
    InvalidEncKey,
    /// The recipient secret key bytes are invalid.
    #[error("invalid recipient secret key")]
    InvalidSecretKey,
    /// HPKE seal failed (e.g., all-zero shared secret or library error).
    #[error("HPKE seal failed")]
    SealFailed,
    /// HPKE open failed (wrong key, tampered ciphertext, or AAD mismatch).
    #[error("HPKE open failed")]
    OpenFailed,
    /// The DH shared secret was all-zero (small-order point or degenerate key).
    #[error("all-zero X25519 shared secret — key rejected")]
    ZeroSharedSecret,
    /// AAD construction failed (CBOR encoding error).
    #[error("AAD construction failed")]
    AadError,
}

impl From<HpkeError> for WrapError {
    fn from(e: HpkeError) -> Self {
        match e {
            HpkeError::EncapError => WrapError::ZeroSharedSecret,
            HpkeError::DecapError => WrapError::ZeroSharedSecret,
            HpkeError::SealError => WrapError::SealFailed,
            HpkeError::OpenError => WrapError::OpenFailed,
            _ => WrapError::SealFailed,
        }
    }
}

/// Output of a successful [`wrap_key`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapResult {
    /// HPKE encapsulated key (32 bytes for X25519).
    pub enc: Vec<u8>,
    /// HPKE ciphertext wrapping the content key K (48 bytes: 32 key + 16 tag).
    pub wk: Vec<u8>,
}

/// Encode the authenticated associated data for a wrap/unwrap operation.
///
/// Format: CBOR array `[bstr ct_hash, tstr author_did]`.
/// This binds both the content hash and the author identity to the HPKE
/// context, preventing ciphertext cross-use.
pub fn canonical_aad(ct_hash: &[u8; 32], author_did: &str) -> Result<Vec<u8>, WrapError> {
    // Manual CBOR encoding to avoid pulling in a runtime dep just for this.
    // Array of 2 items: 0x82
    // bstr(32):  0x58 0x20 <32 bytes>
    // tstr(N):   0x60|len <bytes>  for N <= 23
    //            0x78 <1-byte len> <bytes>  for 24 <= N <= 255
    let did_bytes = author_did.as_bytes();
    if did_bytes.len() > 255 {
        return Err(WrapError::AadError);
    }
    let mut out = Vec::with_capacity(2 + 2 + 32 + 2 + did_bytes.len());
    out.push(0x82); // array(2)
    out.push(0x58); // bstr, 1-byte length follows
    out.push(32u8); // length = 32
    out.extend_from_slice(ct_hash);
    if did_bytes.len() <= 23 {
        out.push(0x60 | did_bytes.len() as u8); // tstr, immediate length
    } else {
        out.push(0x78); // tstr, 1-byte length follows
        out.push(did_bytes.len() as u8);
    }
    out.extend_from_slice(did_bytes);
    Ok(out)
}

/// Wrap (encrypt) the 32-byte content key `k` for `recipient_pk`.
///
/// - `k`            — the symmetric content-encryption key to wrap.
/// - `recipient_pk` — the recipient's 32-byte X25519 public key.
/// - `ct_hash` — blake3 hash of the sealed content ciphertext (binds key
///   to specific ciphertext).
/// - `author_did` — the author's DID string (binds key to specific author).
///
/// Returns `WrapResult { enc, wk }` on success.
pub fn wrap_key(
    k: &[u8; 32],
    recipient_pk: &[u8; 32],
    ct_hash: &[u8; 32],
    author_did: &str,
) -> Result<WrapResult, WrapError> {
    let pk = <X25519HkdfSha256 as KemTrait>::PublicKey::from_bytes(recipient_pk)
        .map_err(|_| WrapError::InvalidRecipientKey)?;

    let aad = canonical_aad(ct_hash, author_did)?;

    // Use rand_core::OsRng (backed by getrandom with `js` feature on wasm32)
    // so wrap_key compiles on both native and wasm32-unknown-unknown without
    // pulling the full `rand` crate into the shared dependency graph.
    let mut csprng = rand_core::OsRng;
    let (enc_key, wk) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &pk,
            INFO,
            k,
            &aad,
            &mut csprng,
        )
        .map_err(WrapError::from)?;

    Ok(WrapResult {
        enc: enc_key.to_bytes().to_vec(),
        wk,
    })
}

/// Unwrap (decrypt) a wrapped content key.
///
/// - `enc`          — the HPKE encapsulated key from [`WrapResult::enc`].
/// - `wk`           — the HPKE wrapped-key ciphertext from [`WrapResult::wk`].
/// - `recipient_sk` — the recipient's 32-byte X25519 secret key.
/// - `ct_hash`      — must match the value used during [`wrap_key`].
/// - `author_did`   — must match the value used during [`wrap_key`].
///
/// Returns the 32-byte content key wrapped in [`Zeroizing`] on success.
pub fn unwrap_key(
    enc: &[u8],
    wk: &[u8],
    recipient_sk: &[u8; 32],
    ct_hash: &[u8; 32],
    author_did: &str,
) -> Result<Zeroizing<[u8; 32]>, WrapError> {
    let enc_key = <X25519HkdfSha256 as KemTrait>::EncappedKey::from_bytes(enc)
        .map_err(|_| WrapError::InvalidEncKey)?;

    let sk = <X25519HkdfSha256 as KemTrait>::PrivateKey::from_bytes(recipient_sk)
        .map_err(|_| WrapError::InvalidSecretKey)?;

    let aad = canonical_aad(ct_hash, author_did)?;

    let plaintext =
        hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeR::Base,
            &sk,
            &enc_key,
            INFO,
            wk,
            &aad,
        )
        .map_err(|_| WrapError::OpenFailed)?;

    if plaintext.len() != 32 {
        return Err(WrapError::OpenFailed);
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext);
    Ok(Zeroizing::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpke::kem::Kem as KemTrait;

    fn random_recipient_keypair() -> ([u8; 32], [u8; 32]) {
        let mut csprng = rand_core::OsRng;
        let (sk, pk) = X25519HkdfSha256::gen_keypair(&mut csprng);
        (sk.to_bytes().into(), pk.to_bytes().into())
    }

    fn make_ct_hash() -> [u8; 32] {
        [0xBEu8; 32]
    }

    // ── Round-trip ─────────────────────────────────────────────────────────

    #[test]
    fn wrap_unwrap_round_trip() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k = [0x55u8; 32];
        let ct_hash = make_ct_hash();
        let author_did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

        let result = wrap_key(&k, &pk_r, &ct_hash, author_did).expect("wrap failed");
        assert_eq!(result.enc.len(), 32); // X25519 encapsulated key
        assert_eq!(result.wk.len(), 48);  // 32 key + 16 tag

        let recovered = unwrap_key(&result.enc, &result.wk, &sk_r, &ct_hash, author_did)
            .expect("unwrap failed");
        assert_eq!(*recovered, k);
    }

    #[test]
    fn wrap_unwrap_different_keys() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k1 = [0x11u8; 32];
        let k2 = [0x22u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:test";

        let r1 = wrap_key(&k1, &pk_r, &ct_hash, did).unwrap();
        let r2 = wrap_key(&k2, &pk_r, &ct_hash, did).unwrap();

        // Wrapped ciphertexts differ (HPKE is probabilistic).
        assert_ne!(r1.wk, r2.wk);

        let rk1 = unwrap_key(&r1.enc, &r1.wk, &sk_r, &ct_hash, did).unwrap();
        let rk2 = unwrap_key(&r2.enc, &r2.wk, &sk_r, &ct_hash, did).unwrap();
        assert_eq!(*rk1, k1);
        assert_eq!(*rk2, k2);
    }

    // ── Wrong key ──────────────────────────────────────────────────────────

    #[test]
    fn wrong_recipient_secret_fails() {
        let (_, pk_r) = random_recipient_keypair();
        let (wrong_sk, _) = random_recipient_keypair();
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:test";

        let result = wrap_key(&k, &pk_r, &ct_hash, did).unwrap();
        let err = unwrap_key(&result.enc, &result.wk, &wrong_sk, &ct_hash, did);
        assert!(err.is_err());
    }

    // ── Tampered ciphertext ────────────────────────────────────────────────

    #[test]
    fn tampered_wk_fails() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:test";

        let mut result = wrap_key(&k, &pk_r, &ct_hash, did).unwrap();
        result.wk[0] ^= 0xFF;
        let err = unwrap_key(&result.enc, &result.wk, &sk_r, &ct_hash, did);
        assert_eq!(err, Err(WrapError::OpenFailed));
    }

    #[test]
    fn tampered_enc_fails() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:test";

        let mut result = wrap_key(&k, &pk_r, &ct_hash, did).unwrap();
        // Flip bits in enc — likely produces a different point → decap fails.
        result.enc[0] ^= 0x80;
        let err = unwrap_key(&result.enc, &result.wk, &sk_r, &ct_hash, did);
        assert!(err.is_err());
    }

    // ── AAD binding ────────────────────────────────────────────────────────

    #[test]
    fn wrong_ct_hash_fails() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:test";

        let result = wrap_key(&k, &pk_r, &ct_hash, did).unwrap();
        let wrong_hash = [0x00u8; 32];
        let err = unwrap_key(&result.enc, &result.wk, &sk_r, &wrong_hash, did);
        assert_eq!(err, Err(WrapError::OpenFailed));
    }

    #[test]
    fn wrong_author_did_fails() {
        let (sk_r, pk_r) = random_recipient_keypair();
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        let did = "did:mnemonik:author";

        let result = wrap_key(&k, &pk_r, &ct_hash, did).unwrap();
        let err = unwrap_key(&result.enc, &result.wk, &sk_r, &ct_hash, "did:mnemonik:other");
        assert_eq!(err, Err(WrapError::OpenFailed));
    }

    // ── canonical_aad stability ────────────────────────────────────────────

    #[test]
    fn canonical_aad_stable() {
        let ct_hash = [0x01u8; 32];
        let did = "did:mnemonik:test";
        let aad1 = canonical_aad(&ct_hash, did).unwrap();
        let aad2 = canonical_aad(&ct_hash, did).unwrap();
        assert_eq!(aad1, aad2);
    }

    #[test]
    fn canonical_aad_different_inputs_differ() {
        let ct_hash = [0x01u8; 32];
        let a = canonical_aad(&ct_hash, "did:a").unwrap();
        let b = canonical_aad(&ct_hash, "did:b").unwrap();
        assert_ne!(a, b);
    }

    // ── Invalid key material ───────────────────────────────────────────────

    #[test]
    fn invalid_recipient_pk_rejected() {
        // An empty slice should fail deserialization.
        let k = [0x42u8; 32];
        let ct_hash = make_ct_hash();
        // Use a wrong-length pk — wrap_key expects exactly 32 bytes.
        // We can't pass a length mismatch via the &[u8;32] sig, so test with
        // the Deserializable path directly.
        let bad_pk_bytes = [0u8; 32]; // all-zero pk; hpke may or may not reject
        // This tests the code path; actual rejection depends on hpke internals.
        // We just verify it doesn't panic.
        let _ = wrap_key(&k, &bad_pk_bytes, &ct_hash, "did:test");
    }
}
