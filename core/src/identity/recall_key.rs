//! Per-owner recall key (RK) primitives for hosted recall sessions (Task 13).
//!
//! The recall key is a random 32-byte symmetric key generated once per owner
//! and HPKE-wrapped to their X25519 public key for durable storage.  The
//! wrapped form lives in `owner_recall_keys` and is never stored in plaintext.
//!
//! At session start the server receives the wrapped RK blob (`enc` + `wk`),
//! unwraps it with the owner's X25519 secret, and holds the plaintext RK
//! in RAM for the TTL duration.  The RK itself never touches SQLite, logs,
//! or metrics.
//!
//! # HPKE parameters
//! Reuses the same ciphersuite already used for content-key wrapping in
//! `sealed::wrap`:
//! ```text
//! ciphersuite:  X25519HkdfSha256, HkdfSha256, ChaCha20Poly1305
//! mode:         Base (unauthenticated, single-shot)
//! info:         b"mnemonic recall key v1"
//! aad:          b"" (no binding — the owner identity is implicit in the X25519 key)
//! ```

use hpke::{
    aead::ChaCha20Poly1305,
    kdf::HkdfSha256,
    kem::{Kem as KemTrait, X25519HkdfSha256},
    Deserializable, OpModeR, OpModeS, Serializable,
};
use zeroize::Zeroizing;

/// HPKE info string for recall-key wrapping.
const INFO: &[u8] = b"mnemonic recall key v1";

/// Error type for recall-key operations.
#[derive(Debug, thiserror::Error)]
pub enum RecallKeyError {
    /// Could not deserialize the recipient public key.
    #[error("invalid recipient public key")]
    InvalidRecipientKey,
    /// Could not deserialize the encapsulated key.
    #[error("invalid encapsulated key")]
    InvalidEncKey,
    /// Could not deserialize the recipient secret key.
    #[error("invalid recipient secret key")]
    InvalidSecretKey,
    /// HPKE seal failed.
    #[error("HPKE seal failed")]
    SealFailed,
    /// HPKE open failed (wrong key or tampered ciphertext).
    #[error("HPKE open failed")]
    OpenFailed,
}

impl From<hpke::HpkeError> for RecallKeyError {
    fn from(e: hpke::HpkeError) -> Self {
        match e {
            hpke::HpkeError::EncapError | hpke::HpkeError::DecapError => RecallKeyError::SealFailed,
            hpke::HpkeError::SealError => RecallKeyError::SealFailed,
            hpke::HpkeError::OpenError => RecallKeyError::OpenFailed,
            _ => RecallKeyError::SealFailed,
        }
    }
}

/// Result of wrapping a recall key for storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapResult {
    /// HPKE encapsulated key (32 bytes for X25519).
    pub enc: Vec<u8>,
    /// HPKE ciphertext of the wrapped RK (32 + 16 tag = 48 bytes).
    pub wk: Vec<u8>,
}

/// Generate a fresh random recall key (32 cryptographically random bytes).
pub fn generate_rk() -> [u8; 32] {
    use rand_core::RngCore;
    let mut rk = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut rk);
    rk
}

/// Wrap a recall key for storage under the owner's X25519 public key.
///
/// - `rk`           — the 32-byte recall key to wrap.
/// - `owner_x25519_pk` — the owner's 32-byte X25519 public key.
pub fn wrap_rk(rk: &[u8; 32], owner_x25519_pk: &[u8; 32]) -> Result<WrapResult, RecallKeyError> {
    let pk = <X25519HkdfSha256 as KemTrait>::PublicKey::from_bytes(owner_x25519_pk)
        .map_err(|_| RecallKeyError::InvalidRecipientKey)?;

    let mut csprng = rand_core::OsRng;
    let (enc_key, wk) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &pk,
            INFO,
            rk,
            b"", // empty AAD — owner identity is implicit in the key
            &mut csprng,
        )
        .map_err(RecallKeyError::from)?;

    Ok(WrapResult {
        enc: enc_key.to_bytes().to_vec(),
        wk,
    })
}

/// Unwrap a recall key using the owner's X25519 secret key.
///
/// - `enc` — HPKE encapsulated key bytes from [`WrapResult::enc`].
/// - `wk`  — HPKE ciphertext from [`WrapResult::wk`].
/// - `owner_x25519_sk` — the owner's 32-byte X25519 secret key.
///
/// Returns the plaintext 32-byte RK wrapped in [`Zeroizing`].
pub fn unwrap_rk(
    enc: &[u8],
    wk: &[u8],
    owner_x25519_sk: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, RecallKeyError> {
    let enc_key = <X25519HkdfSha256 as KemTrait>::EncappedKey::from_bytes(enc)
        .map_err(|_| RecallKeyError::InvalidEncKey)?;

    let sk = <X25519HkdfSha256 as KemTrait>::PrivateKey::from_bytes(owner_x25519_sk)
        .map_err(|_| RecallKeyError::InvalidSecretKey)?;

    let plaintext = hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &sk,
        &enc_key,
        INFO,
        wk,
        b"", // must match AAD used during seal
    )
    .map_err(|_| RecallKeyError::OpenFailed)?;

    if plaintext.len() != 32 {
        return Err(RecallKeyError::OpenFailed);
    }

    let mut rk = [0u8; 32];
    rk.copy_from_slice(&plaintext);
    Ok(Zeroizing::new(rk))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_x25519_keypair() -> ([u8; 32], [u8; 32]) {
        use hpke::Serializable;
        let (sk, pk) = X25519HkdfSha256::gen_keypair(&mut rand_core::OsRng);
        (sk.to_bytes().into(), pk.to_bytes().into())
    }

    #[test]
    fn generate_rk_is_32_bytes() {
        let rk = generate_rk();
        assert_eq!(rk.len(), 32);
    }

    #[test]
    fn generate_rk_is_random() {
        let a = generate_rk();
        let b = generate_rk();
        // Astronomically unlikely to collide.
        assert_ne!(a, b);
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let (sk, pk) = random_x25519_keypair();
        let rk = generate_rk();
        let result = wrap_rk(&rk, &pk).expect("wrap_rk failed");
        assert_eq!(result.enc.len(), 32);
        assert_eq!(result.wk.len(), 48); // 32-byte plaintext + 16-byte AEAD tag

        let recovered = unwrap_rk(&result.enc, &result.wk, &sk).expect("unwrap_rk failed");
        assert_eq!(*recovered, rk);
    }

    #[test]
    fn wrong_secret_fails() {
        let (_, pk) = random_x25519_keypair();
        let (wrong_sk, _) = random_x25519_keypair();
        let rk = generate_rk();
        let result = wrap_rk(&rk, &pk).unwrap();
        let err = unwrap_rk(&result.enc, &result.wk, &wrong_sk);
        assert!(err.is_err());
    }

    #[test]
    fn tampered_wk_fails() {
        let (sk, pk) = random_x25519_keypair();
        let rk = generate_rk();
        let mut result = wrap_rk(&rk, &pk).unwrap();
        result.wk[0] ^= 0xFF;
        let err = unwrap_rk(&result.enc, &result.wk, &sk);
        assert!(err.is_err());
    }
}
