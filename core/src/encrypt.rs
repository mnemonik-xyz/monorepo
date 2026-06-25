//! X25519 ECIES sealing for shared / private memories (Wave 6, design §17).
//!
//! In the non-custodial target, "private" / "shared" can no longer be a SQL
//! access-control flag — once Arweave is the source of truth, you cannot
//! WHERE-clause your way around public, immutable storage (design §17). So
//! confidential memories must be **encrypted** before they leave the client:
//!
//!   - **Public** (default, unchanged): plaintext on Arweave.
//!   - **Private**: encrypted to the author's own X25519 key.
//!   - **Shared-to-N**: encrypted to an explicit recipient set (e.g. an
//!     ERC-8183 job's `{client, provider, evaluator}`).
//!
//! This module is the pure, chain-independent core: it seals a plaintext to a
//! set of X25519 recipient public keys and opens it with any one recipient's
//! secret key. Recall over encrypted memories is therefore **client-side after
//! decryption** (the operator only ever stores ciphertext). Mapping the
//! Mnemonic Ed25519 identity to its X25519 recipient key (the standard
//! birational conversion, design §18) is the client-side key-management
//! adapter that supplies the keys to this layer.
//!
//! ## Scheme (ECIES per recipient)
//! For each recipient an ephemeral X25519 keypair is generated and the
//! plaintext is sealed with `SalsaBox(recipient_pub, ephemeral_secret)`
//! (X25519 ECDH → XSalsa20-Poly1305), matching the `crypto_box` primitive
//! already used elsewhere in the codebase. The envelope carries one entry per
//! recipient; a recipient decrypts the entry addressed to them. Forward
//! secrecy of the *operator* is irrelevant (it never has a key); the property
//! that matters is that **only** a holder of a listed recipient secret can
//! recover the plaintext.

use crypto_box::{
    aead::{generic_array::GenericArray, Aead, AeadCore, OsRng},
    PublicKey, SalsaBox, SecretKey,
};
use serde::{Deserialize, Serialize};

/// One recipient's sealed copy of the content key material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientEntry {
    /// The recipient's X25519 public key (32 bytes) this entry is addressed to.
    /// Lets a client locate its entry without trial-decrypting every one.
    pub recipient_pub: [u8; 32],
    /// Per-entry ephemeral X25519 public key (32 bytes).
    pub ephemeral_pub: [u8; 32],
    /// XSalsa20-Poly1305 nonce (24 bytes).
    pub nonce: [u8; 24],
    /// Sealed ciphertext (XSalsa20-Poly1305 over the plaintext).
    pub ciphertext: Vec<u8>,
}

/// A memory's content sealed to one or more X25519 recipients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedEnvelope {
    /// Envelope format version (forward-compat).
    pub v: u8,
    /// One entry per recipient. Order is not significant.
    pub recipients: Vec<RecipientEntry>,
}

/// Errors from sealing / opening.
#[derive(Debug, PartialEq, Eq)]
pub enum EncryptError {
    /// `seal_to_recipients` called with an empty recipient set.
    NoRecipients,
    /// AEAD encryption failed (should not happen for valid keys).
    SealFailed,
    /// No entry in the envelope could be opened with the supplied secret.
    NotARecipient,
}

impl std::fmt::Display for EncryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EncryptError::NoRecipients => "no recipients supplied",
            EncryptError::SealFailed => "AEAD seal failed",
            EncryptError::NotARecipient => "secret key does not match any recipient entry",
        };
        f.write_str(s)
    }
}

impl std::error::Error for EncryptError {}

/// Seal `plaintext` to every recipient in `recipient_pubs` (each a 32-byte
/// X25519 public key). Returns an envelope with one ECIES entry per recipient.
pub fn seal_to_recipients(
    plaintext: &[u8],
    recipient_pubs: &[[u8; 32]],
) -> Result<SealedEnvelope, EncryptError> {
    if recipient_pubs.is_empty() {
        return Err(EncryptError::NoRecipients);
    }
    let mut recipients = Vec::with_capacity(recipient_pubs.len());
    for rp in recipient_pubs {
        let recipient_pub = PublicKey::from(*rp);
        // Fresh ephemeral keypair per recipient.
        let eph_secret = SecretKey::generate(&mut OsRng);
        let eph_pub = eph_secret.public_key();
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let sbox = SalsaBox::new(&recipient_pub, &eph_secret);
        let ciphertext = sbox
            .encrypt(&nonce, plaintext)
            .map_err(|_| EncryptError::SealFailed)?;
        let nonce_bytes: [u8; 24] = nonce
            .as_slice()
            .try_into()
            .expect("XSalsa20 nonce is 24 bytes");
        recipients.push(RecipientEntry {
            recipient_pub: *rp,
            ephemeral_pub: eph_pub.to_bytes(),
            nonce: nonce_bytes,
            ciphertext,
        });
    }
    Ok(SealedEnvelope { v: 1, recipients })
}

/// Open a sealed envelope with one recipient's X25519 secret key (32 bytes).
/// Tries the entry addressed to the matching public key first, then falls back
/// to trying every entry (robust to a caller that doesn't know its own listed
/// pubkey). Returns the recovered plaintext, or [`EncryptError::NotARecipient`].
pub fn open(
    envelope: &SealedEnvelope,
    recipient_secret: &[u8; 32],
) -> Result<Vec<u8>, EncryptError> {
    let secret = SecretKey::from(*recipient_secret);
    let my_pub = secret.public_key().to_bytes();

    // Prefer the entry explicitly addressed to us.
    let ordered = envelope
        .recipients
        .iter()
        .filter(|e| e.recipient_pub == my_pub)
        .chain(
            envelope
                .recipients
                .iter()
                .filter(|e| e.recipient_pub != my_pub),
        );

    for entry in ordered {
        let eph_pub = PublicKey::from(entry.ephemeral_pub);
        let sbox = SalsaBox::new(&eph_pub, &secret);
        let nonce = GenericArray::from(entry.nonce);
        if let Ok(pt) = sbox.decrypt(&nonce, entry.ciphertext.as_slice()) {
            return Ok(pt);
        }
    }
    Err(EncryptError::NotARecipient)
}

/// Convenience: derive the X25519 public key (32 bytes) for a secret key, so
/// callers can build a recipient set from their own secrets in tests / tools.
pub fn public_from_secret(recipient_secret: &[u8; 32]) -> [u8; 32] {
    SecretKey::from(*recipient_secret).public_key().to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a fresh (secret_bytes, public_bytes) X25519 keypair.
    fn keypair() -> ([u8; 32], [u8; 32]) {
        let sk = SecretKey::generate(&mut OsRng);
        (sk.to_bytes(), sk.public_key().to_bytes())
    }

    #[test]
    fn round_trip_single_recipient() {
        let (sk, pk) = keypair();
        let msg = b"confidential deliverable";
        let env = seal_to_recipients(msg, &[pk]).unwrap();
        assert_eq!(env.recipients.len(), 1);
        assert_eq!(open(&env, &sk).unwrap(), msg);
    }

    #[test]
    fn shared_to_n_every_recipient_can_open() {
        // ERC-8183 job parties: {client, provider, evaluator}.
        let (client_sk, client_pk) = keypair();
        let (provider_sk, provider_pk) = keypair();
        let (evaluator_sk, evaluator_pk) = keypair();
        let msg = b"shared validation evidence";

        let env = seal_to_recipients(msg, &[client_pk, provider_pk, evaluator_pk]).unwrap();
        assert_eq!(env.recipients.len(), 3);

        // Each listed recipient recovers the same plaintext.
        for sk in [&client_sk, &provider_sk, &evaluator_sk] {
            assert_eq!(open(&env, sk).unwrap(), msg);
        }
    }

    #[test]
    fn non_recipient_cannot_open() {
        let (_sk, pk) = keypair();
        let (outsider_sk, _outsider_pk) = keypair();
        let env = seal_to_recipients(b"secret", &[pk]).unwrap();
        assert_eq!(open(&env, &outsider_sk), Err(EncryptError::NotARecipient));
    }

    #[test]
    fn ciphertext_is_not_plaintext_and_differs_per_recipient() {
        let (_a_sk, a_pk) = keypair();
        let (_b_sk, b_pk) = keypair();
        let msg = b"the same plaintext to two recipients";
        let env = seal_to_recipients(msg, &[a_pk, b_pk]).unwrap();
        for e in &env.recipients {
            assert_ne!(
                e.ciphertext.as_slice(),
                msg,
                "ciphertext must not equal plaintext"
            );
        }
        // Independent ephemeral keys + nonces → the two sealed copies differ.
        assert_ne!(
            env.recipients[0].ciphertext, env.recipients[1].ciphertext,
            "per-recipient seals must not be identical"
        );
    }

    #[test]
    fn tampered_ciphertext_fails_to_open() {
        let (sk, pk) = keypair();
        let mut env = seal_to_recipients(b"integrity matters", &[pk]).unwrap();
        env.recipients[0].ciphertext[0] ^= 0xff;
        assert_eq!(open(&env, &sk), Err(EncryptError::NotARecipient));
    }

    #[test]
    fn empty_recipient_set_is_rejected() {
        assert_eq!(
            seal_to_recipients(b"x", &[]),
            Err(EncryptError::NoRecipients)
        );
    }

    #[test]
    fn envelope_serde_round_trips() {
        let (sk, pk) = keypair();
        let env = seal_to_recipients(b"persist me", &[pk]).unwrap();
        let json = serde_json::to_vec(&env).unwrap();
        let back: SealedEnvelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(env, back);
        // Still decryptable after a serialize/deserialize cycle (e.g. stored
        // on Arweave and fetched back).
        assert_eq!(open(&back, &sk).unwrap(), b"persist me");
    }

    #[test]
    fn private_memory_is_just_shared_to_self() {
        // "Private" = sealed to the author's own key only.
        let (sk, pk) = keypair();
        let env = seal_to_recipients(b"my private note", &[pk]).unwrap();
        assert_eq!(public_from_secret(&sk), pk);
        assert_eq!(open(&env, &sk).unwrap(), b"my private note");
    }
}
