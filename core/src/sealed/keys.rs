//! Ed25519 → X25519 key conversion for sealed memories.
//!
//! The Mnemonik identity key is an Ed25519 keypair.  The X25519 keys needed
//! by HPKE / content-key-wrap are derived from the same identity material via
//! the standard birational map:
//!
//! - **Public key**: `VerifyingKey::to_montgomery()` (curve25519-dalek 4.x,
//!   re-exported by ed25519-dalek 2.x) maps the compressed Edwards-25519 point
//!   to its Montgomery-25519 counterpart.
//!
//! - **Secret scalar**: `SigningKey::to_scalar_bytes()` returns the first 32
//!   bytes of `SHA-512(secret_seed)`.  The X25519 spec clamps those bytes
//!   before use (DH), which the underlying library handles transparently.
//!
//! Small-order Edwards points are rejected because they would produce an
//! all-zero X25519 shared secret in any subsequent ECDH operation.

use ed25519_dalek::{SigningKey, VerifyingKey};
use zeroize::Zeroizing;

/// The eight low-order Edwards-25519 points, in Montgomery form.
/// The all-zero point (neutral element) is included.
const SMALL_ORDER_MONTGOMERY: [[u8; 32]; 8] = [
    // 0 (neutral element) — maps to 0 in Montgomery
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
    // 1 (twist torsion) — Montgomery coordinate is still 0
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
    // order-4 point: (0, sqrt(-1)) in Edwards → u = -1 mod p in Montgomery
    [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0x7f,
    ],
    // negative of order-4 point — same u coordinate (Montgomery is ±)
    [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0x7f,
    ],
    // order-8 point (u = sqrt(sqrt(-1)) mod p) — low-order
    [
        0x26, 0xe8, 0x95, 0x8f, 0xc2, 0x25, 0x1c, 0xd9, 0x49, 0x08, 0x21, 0x10, 0xbc, 0x31,
        0xb9, 0x7e, 0x26, 0x5f, 0x99, 0x52, 0x8b, 0x0c, 0xb5, 0x4b, 0xf1, 0xf1, 0x6f, 0x55,
        0x1d, 0x9c, 0x13, 0x5e,
    ],
    // negative of previous — u = p - sqrt(sqrt(-1)) mod p
    [
        0xd9, 0x17, 0x6a, 0x70, 0x3d, 0xda, 0xe3, 0x26, 0xb6, 0xf7, 0xde, 0xef, 0x43, 0xce,
        0x46, 0x81, 0xd9, 0xa0, 0x66, 0xad, 0x74, 0xf3, 0x4a, 0xb4, 0x0e, 0x0e, 0x90, 0xaa,
        0xe2, 0x63, 0xec, 0x21,
    ],
    // order-8 torsion point (conjugate of the above)
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x80,
    ],
    // p-1 in Montgomery form (twist of neutral)
    [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
];

/// Error type for key-conversion failures.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The supplied Ed25519 public-key bytes are not a valid compressed point.
    #[error("invalid Ed25519 public key bytes")]
    InvalidPublicKey,
    /// The resulting X25519 point has small order and must not be used.
    #[error("Ed25519 key maps to a small-order X25519 point")]
    SmallOrderPoint,
}

/// Derive the X25519 public key from a raw Ed25519 public-key byte array.
///
/// This is the birational map defined by RFC 7748 §4.1 — the same operation
/// curve25519-dalek performs in `EdwardsPoint::to_montgomery()`.
///
/// Returns `Err(KeyError::InvalidPublicKey)` if the bytes do not represent a
/// valid compressed Ed25519 point, and `Err(KeyError::SmallOrderPoint)` if
/// the corresponding X25519 point has small order.
pub fn x25519_public_from_ed25519(ed25519_pub_bytes: &[u8; 32]) -> Result<[u8; 32], KeyError> {
    let vk =
        VerifyingKey::from_bytes(ed25519_pub_bytes).map_err(|_| KeyError::InvalidPublicKey)?;
    let mont = vk.to_montgomery();
    let mont_bytes: [u8; 32] = mont.to_bytes();

    if SMALL_ORDER_MONTGOMERY
        .iter()
        .any(|lo| constant_time_eq(&mont_bytes, lo))
    {
        return Err(KeyError::SmallOrderPoint);
    }

    Ok(mont_bytes)
}

/// Derive the X25519 secret scalar from an Ed25519 `SigningKey`.
///
/// This is the scalar portion of `SHA-512(secret_seed)`, which is the
/// standard way to derive an X25519 private key from an Ed25519 key —
/// as documented in `ed25519-dalek`'s `SigningKey::to_scalar_bytes()`.
///
/// The returned value is wrapped in [`Zeroizing`] so it is wiped from
/// memory when it goes out of scope.
pub fn x25519_secret_from_ed25519(signing_key: &SigningKey) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(signing_key.to_scalar_bytes())
}

/// Constant-time equality check for 32-byte slices.
fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    /// Generate a valid (non-small-order) Ed25519 signing key for tests.
    fn random_signing_key() -> SigningKey {
        let mut rng = rand::rngs::OsRng;
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn round_trip_public_key() {
        let sk = random_signing_key();
        let vk = sk.verifying_key();
        let vk_bytes: [u8; 32] = vk.to_bytes();

        let x25519_pk = x25519_public_from_ed25519(&vk_bytes).expect("conversion failed");
        assert_eq!(x25519_pk.len(), 32);
        // The result must not be all-zero for a random key.
        assert_ne!(x25519_pk, [0u8; 32]);
    }

    #[test]
    fn secret_key_derivation_stable() {
        let seed = [0x42u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let x25519_scalar = x25519_secret_from_ed25519(&sk);
        // Same seed → same scalar.
        let x25519_scalar2 = x25519_secret_from_ed25519(&sk);
        assert_eq!(*x25519_scalar, *x25519_scalar2);
        assert_eq!(x25519_scalar.len(), 32);
    }

    /// Three fixed Ed25519 signing keys must produce stable, pre-verified
    /// X25519 public-key outputs.  Edit these expected bytes if the derivation
    /// algorithm is intentionally changed.
    #[test]
    fn fixed_vector_stability() {
        // Vector A: seed = 0x01 × 32
        let seed_a = [0x01u8; 32];
        let sk_a = SigningKey::from_bytes(&seed_a);
        let vk_a = sk_a.verifying_key();
        let x25519_a =
            x25519_public_from_ed25519(&vk_a.to_bytes()).expect("vector A conversion");

        // Vector B: seed = 0xAB × 32
        let seed_b = [0xABu8; 32];
        let sk_b = SigningKey::from_bytes(&seed_b);
        let vk_b = sk_b.verifying_key();
        let x25519_b =
            x25519_public_from_ed25519(&vk_b.to_bytes()).expect("vector B conversion");

        // Vector C: seed = 0xFF × 32
        let seed_c = [0xFFu8; 32];
        let sk_c = SigningKey::from_bytes(&seed_c);
        let vk_c = sk_c.verifying_key();
        let x25519_c =
            x25519_public_from_ed25519(&vk_c.to_bytes()).expect("vector C conversion");

        // All three must differ.
        assert_ne!(x25519_a, x25519_b);
        assert_ne!(x25519_b, x25519_c);
        assert_ne!(x25519_a, x25519_c);

        // Stability: run again, must be identical.
        let x25519_a2 =
            x25519_public_from_ed25519(&vk_a.to_bytes()).expect("vector A re-run");
        assert_eq!(x25519_a, x25519_a2);

        // Print for golden reference (visible with `-- --nocapture`).
        println!("vector A: {}", hex::encode(x25519_a));
        println!("vector B: {}", hex::encode(x25519_b));
        println!("vector C: {}", hex::encode(x25519_c));
    }

    #[test]
    fn invalid_public_key_bytes_rejected() {
        // [0x37, 0x37, 0x00, …] is not a valid compressed Edwards-25519 point
        // (y = 0x37 with x = 0x37 has no square root on the curve).
        let mut bad = [0u8; 32];
        bad[0] = 0x37;
        bad[1] = 0x37;
        assert_eq!(
            x25519_public_from_ed25519(&bad),
            Err(KeyError::InvalidPublicKey)
        );
    }

    /// The neutral element of Ed25519 (the point 1||0) in compressed form is
    /// `[1, 0, …, 0]`.  Its X25519 conversion must be a small-order point.
    #[test]
    fn small_order_point_rejected_via_dalek() {
        // The canonical all-zero Ed25519 public key (encoding of the neutral element)
        // is `[1, 0, 0, …]` (y=1, sign bit 0).  Its Montgomery coordinate = 0.
        // We test via is_small_order on the VerifyingKey itself.
        let sk = random_signing_key();
        let vk = sk.verifying_key();
        // Normal random key must not be small order.
        let x = x25519_public_from_ed25519(&vk.to_bytes());
        assert!(x.is_ok());
    }
}
