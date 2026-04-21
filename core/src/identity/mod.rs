use anyhow::Context;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::pubkey::Pubkey;
use std::path::Path;

/// Load keypair from JSON file, or generate and save a new one.
pub fn load_or_create_keypair(path: &Path) -> anyhow::Result<Keypair> {
    if path.exists() {
        let data = std::fs::read_to_string(path).context("reading keypair file")?;
        let bytes: Vec<u8> = serde_json::from_str(&data).context("parsing keypair JSON")?;
        Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow::anyhow!("invalid keypair: {e}"))
    } else {
        let kp = Keypair::new();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating keypair directory")?;
        }
        let bytes: Vec<u8> = kp.to_bytes().to_vec();
        std::fs::write(path, serde_json::to_string(&bytes)?).context("writing keypair")?;
        Ok(kp)
    }
}

/// Base58-encoded public key.
pub fn pubkey_base58(kp: &Keypair) -> String {
    kp.pubkey().to_string()
}

/// did:sol:<base58_pubkey>
pub fn did_sol(kp: &Keypair) -> String {
    format!("did:sol:{}", kp.pubkey())
}

/// did:key:z<base58btc(multicodec_ed25519 + raw_pubkey)>
pub fn did_key(kp: &Keypair) -> String {
    let raw = kp.pubkey().to_bytes();
    // Ed25519 multicodec prefix: 0xed01
    let mut mc = vec![0xed, 0x01];
    mc.extend_from_slice(&raw);
    let encoded = bs58::encode(&mc).into_string();
    format!("did:key:z{encoded}")
}

/// Sign arbitrary bytes with Ed25519.
pub fn sign_bytes(kp: &Keypair, message: &[u8]) -> Vec<u8> {
    kp.sign_message(message).as_ref().to_vec()
}

/// Verify an Ed25519 signature.
pub fn verify_signature(pubkey: &Pubkey, message: &[u8], signature: &[u8]) -> bool {
    if signature.len() != 64 {
        return false;
    }
    let sig = solana_sdk::signature::Signature::from(<[u8; 64]>::try_from(signature).unwrap());
    sig.verify(pubkey.as_ref(), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_did_sol() {
        let kp = Keypair::new();
        let d = did_sol(&kp);
        assert!(d.starts_with("did:sol:"));
        assert!(d.contains(&kp.pubkey().to_string()));
    }

    #[test]
    fn test_did_key() {
        let kp = Keypair::new();
        let d = did_key(&kp);
        assert!(d.starts_with("did:key:z"));
        // Deterministic
        assert_eq!(d, did_key(&kp));
    }

    #[test]
    fn test_sign_verify() {
        let kp = Keypair::new();
        let msg = b"hello mnemonic";
        let sig = sign_bytes(&kp, msg);
        assert_eq!(sig.len(), 64);
        assert!(verify_signature(&kp.pubkey(), msg, &sig));
        assert!(!verify_signature(&kp.pubkey(), b"wrong", &sig));
    }

    #[test]
    fn test_keypair_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_id.json");

        let kp1 = load_or_create_keypair(&path).unwrap();
        let kp2 = load_or_create_keypair(&path).unwrap();
        assert_eq!(kp1.pubkey(), kp2.pubkey());
    }
}
