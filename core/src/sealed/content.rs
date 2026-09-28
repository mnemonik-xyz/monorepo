//! Bucket-padded XChaCha20Poly1305 content encryption for sealed memories.
//!
//! ## Padding
//! Memory content is padded to a multiple of [`BUCKET_SIZE`] (256 bytes)
//! before encryption to hide the true content length.  The first 4 bytes of
//! the padded buffer are a little-endian `u32` encoding the original length,
//! followed by the plaintext, followed by zero-padding to the next bucket
//! boundary.
//!
//! ## Encryption
//! [`encrypt_content`] and [`decrypt_content`] wrap XChaCha20Poly1305 (24-byte
//! nonce, 32-byte key, 16-byte Poly1305 tag appended to ciphertext).
//! Authenticated associated data is supported and must match on decrypt.
//!
//! ## Key commitment
//! [`key_commitment`] computes a 32-byte, domain-separated blake3 digest of
//! the content-encryption key so the key can be committed to in a header
//! without revealing the key itself.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305,
};
use thiserror::Error;

/// Granularity of padded plaintext blocks in bytes.
pub const BUCKET_SIZE: usize = 256;

/// Error type for content-layer operations.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ContentError {
    /// Encryption failed (key or nonce invalid, or library error).
    #[error("content encryption failed")]
    EncryptFailed,
    /// Decryption failed (tag mismatch, wrong key, or ciphertext truncated).
    #[error("content decryption failed")]
    DecryptFailed,
    /// Padded buffer is too short to contain the 4-byte length prefix.
    #[error("padded buffer too short")]
    PaddedBufferTooShort,
    /// The length prefix in the padded buffer exceeds the buffer size.
    #[error("length prefix in padded buffer is out of range")]
    LengthOutOfRange,
}

/// Pad `pt` to a multiple of [`BUCKET_SIZE`] bytes.
///
/// Layout: `[u32-LE original-length][plaintext bytes][zero padding]`
///
/// The minimum output length is `BUCKET_SIZE` (for empty input or input
/// that fits in the first bucket minus the 4-byte header).
pub fn pad_to_bucket(pt: &[u8]) -> Vec<u8> {
    let len = pt.len();
    // Total content = 4-byte header + plaintext.
    let content_len = 4 + len;
    // Round up to next multiple of BUCKET_SIZE.
    let padded_len = if content_len.is_multiple_of(BUCKET_SIZE) {
        content_len
    } else {
        (content_len / BUCKET_SIZE + 1) * BUCKET_SIZE
    };
    let padded_len = padded_len.max(BUCKET_SIZE);

    let mut buf = vec![0u8; padded_len];
    let len_u32 = len as u32;
    buf[0..4].copy_from_slice(&len_u32.to_le_bytes());
    buf[4..4 + len].copy_from_slice(pt);
    buf
}

/// Reverse [`pad_to_bucket`]: strip the length prefix and trailing zeros.
///
/// Returns `Err` if the buffer is too short or the embedded length is
/// inconsistent.
pub fn unpad(padded: &[u8]) -> Result<Vec<u8>, ContentError> {
    if padded.len() < 4 {
        return Err(ContentError::PaddedBufferTooShort);
    }
    let len = u32::from_le_bytes(padded[0..4].try_into().unwrap()) as usize;
    if len + 4 > padded.len() {
        return Err(ContentError::LengthOutOfRange);
    }
    Ok(padded[4..4 + len].to_vec())
}

/// Encrypt `plaintext` with XChaCha20Poly1305.
///
/// - `key` — 32-byte encryption key.
/// - `nonce` — 24-byte unique nonce (must not be reused with the same key).
/// - `ad` — authenticated associated data (not encrypted, not secret).
///
/// Returns the ciphertext (with 16-byte Poly1305 tag appended).
pub fn encrypt_content(
    key: &[u8; 32],
    nonce: &[u8; 24],
    ad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, ContentError> {
    use chacha20poly1305::aead::Payload;

    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce_ga: &chacha20poly1305::XNonce = nonce.into();

    cipher
        .encrypt(nonce_ga, Payload { msg: plaintext, aad: ad })
        .map_err(|_| ContentError::EncryptFailed)
}

/// Decrypt `ciphertext` with XChaCha20Poly1305.
///
/// The ciphertext must have been produced by [`encrypt_content`] with the same
/// key, nonce, and associated data.  Returns the plaintext on success.
pub fn decrypt_content(
    key: &[u8; 32],
    nonce: &[u8; 24],
    ad: &[u8],
    ct: &[u8],
) -> Result<Vec<u8>, ContentError> {
    use chacha20poly1305::aead::Payload;

    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce_ga: &chacha20poly1305::XNonce = nonce.into();

    cipher
        .decrypt(nonce_ga, Payload { msg: ct, aad: ad })
        .map_err(|_| ContentError::DecryptFailed)
}

/// Compute a 32-byte domain-separated commitment to `key`.
///
/// Uses `blake3::derive_key("mnemonic sealed v1 key commitment", key)`.
/// The commitment can be stored in a sealed-memory header so a verifier can
/// check that the wrapped key matches the content key without having access
/// to the key itself.
pub fn key_commitment(key: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("mnemonic sealed v1 key commitment", key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    fn make_nonce() -> [u8; 24] {
        [0xAAu8; 24]
    }

    // ── Padding tests ──────────────────────────────────────────────────────

    #[test]
    fn padding_round_trip_sizes() {
        for &sz in &[0usize, 1, 255, 256, 257, 512] {
            let input = vec![0x7Fu8; sz];
            let padded = pad_to_bucket(&input);
            // Must be a multiple of BUCKET_SIZE.
            assert_eq!(padded.len() % BUCKET_SIZE, 0, "size {} not aligned", sz);
            // Must be at least BUCKET_SIZE.
            assert!(padded.len() >= BUCKET_SIZE, "size {} too small", sz);
            let recovered = unpad(&padded).expect("unpad failed for size {sz}");
            assert_eq!(recovered, input, "round-trip failed for size {sz}");
        }
    }

    #[test]
    fn padding_length_header_correct() {
        let input = b"hello";
        let padded = pad_to_bucket(input);
        let stored_len = u32::from_le_bytes(padded[0..4].try_into().unwrap()) as usize;
        assert_eq!(stored_len, input.len());
    }

    #[test]
    fn unpad_too_short_returns_error() {
        assert_eq!(unpad(&[0x01, 0x02, 0x03]), Err(ContentError::PaddedBufferTooShort));
    }

    #[test]
    fn unpad_length_overflow_returns_error() {
        // Length field claims 1000 bytes but buffer is only 8 bytes.
        let mut buf = vec![0u8; 8];
        let len: u32 = 1000;
        buf[0..4].copy_from_slice(&len.to_le_bytes());
        assert_eq!(unpad(&buf), Err(ContentError::LengthOutOfRange));
    }

    // ── Encryption round-trip ──────────────────────────────────────────────

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = make_key();
        let nonce = make_nonce();
        let ad = b"associated data";
        let plaintext = b"secret memory content";

        let ct = encrypt_content(&key, &nonce, ad, plaintext).expect("encrypt failed");
        let pt = decrypt_content(&key, &nonce, ad, &ct).expect("decrypt failed");
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn wrong_key_decrypt_fails() {
        let key = make_key();
        let nonce = make_nonce();
        let ad = b"ad";
        let ct = encrypt_content(&key, &nonce, ad, b"data").expect("encrypt");

        let wrong_key = [0x00u8; 32];
        assert_eq!(
            decrypt_content(&wrong_key, &nonce, ad, &ct),
            Err(ContentError::DecryptFailed)
        );
    }

    #[test]
    fn tampered_ciphertext_decrypt_fails() {
        let key = make_key();
        let nonce = make_nonce();
        let ad = b"ad";
        let mut ct = encrypt_content(&key, &nonce, ad, b"data").expect("encrypt");

        ct[0] ^= 0xFF; // flip bits in first byte
        assert_eq!(
            decrypt_content(&key, &nonce, ad, &ct),
            Err(ContentError::DecryptFailed)
        );
    }

    #[test]
    fn wrong_nonce_decrypt_fails() {
        let key = make_key();
        let nonce = make_nonce();
        let ad = b"ad";
        let ct = encrypt_content(&key, &nonce, ad, b"data").expect("encrypt");

        let wrong_nonce = [0x00u8; 24];
        assert_eq!(
            decrypt_content(&key, &wrong_nonce, ad, &ct),
            Err(ContentError::DecryptFailed)
        );
    }

    #[test]
    fn tampered_ad_decrypt_fails() {
        let key = make_key();
        let nonce = make_nonce();
        let ct = encrypt_content(&key, &nonce, b"original ad", b"data").expect("encrypt");

        assert_eq!(
            decrypt_content(&key, &nonce, b"tampered ad", &ct),
            Err(ContentError::DecryptFailed)
        );
    }

    // ── Key commitment ─────────────────────────────────────────────────────

    #[test]
    fn key_commitment_stable() {
        let key = [0x01u8; 32];
        let c1 = key_commitment(&key);
        let c2 = key_commitment(&key);
        assert_eq!(c1, c2);
        assert_ne!(c1, key); // commitment != key
    }

    #[test]
    fn key_commitment_different_keys() {
        let c1 = key_commitment(&[0x01u8; 32]);
        let c2 = key_commitment(&[0x02u8; 32]);
        assert_ne!(c1, c2);
    }
}
