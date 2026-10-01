//! Generate test-only signed A2A vectors. Never use these keys in production.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use mnemonic_core::codec::a2a::{
    build_x_mnemonic_extension,
    signed::{prepare_signed_a2a, prepare_signed_a2a_stream, verify_signed_a2a, RecipientCard},
};
use serde_json::json;
use solana_sdk::signature::{Keypair, Signer};
fn main() -> anyhow::Result<()> {
    let author = Keypair::new_from_array([1; 32]);
    let reader = Keypair::new_from_array([2; 32]);
    let outsider = Keypair::new_from_array([3; 32]);
    let mut card = json!({"name":"test recipient","url":"https://agent.test", "extensions":[build_x_mnemonic_extension(&reader.pubkey().to_string(),None)]});
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA"}"#);
    let input = format!(
        "{}.{}",
        header,
        URL_SAFE_NO_PAD.encode(serde_jcs::to_vec(&card)?)
    );
    card["signatures"] = json!([{"protected":header,"signature":URL_SAFE_NO_PAD.encode(SigningKey::from_bytes(&[2;32]).sign(input.as_bytes()).to_bytes())}]);
    let recipient = RecipientCard {
        card: card.clone(),
        trusted_card_signer: reader.pubkey().to_string(),
    };
    let payload = json!({"messageId":"fixture-msg","contextId":"fixture-ctx","role":"agent","parts":[{"kind":"text","text":"PRIVATE SEALED E2E SENTINEL"}]});
    let time = "2026-10-01T00:00:00Z";
    let plain = prepare_signed_a2a(
        &author,
        "message",
        payload.clone(),
        "fixture-ctx",
        None,
        time,
        None,
    )?;
    let sealed = prepare_signed_a2a(
        &author,
        "message",
        payload.clone(),
        "fixture-ctx",
        None,
        time,
        Some(std::slice::from_ref(&recipient)),
    )?;
    let stream = prepare_signed_a2a_stream(
        &author,
        "message",
        payload.clone(),
        "fixture-ctx",
        None,
        time,
        &[recipient],
        19,
    )?;
    let jcs = |bytes: &[u8]| -> anyhow::Result<String> {
        Ok(hex::encode(serde_jcs::to_vec(
            &verify_signed_a2a(bytes, None)?.binding,
        )?))
    };
    let key = |kp: &Keypair| json!({"secret":kp.to_bytes().to_vec(),"pubkey_base58":kp.pubkey().to_string()});
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"test_only":true,"author":key(&author),"reader":key(&reader),"outsider":key(&outsider),"card":card,"payload":payload,"created_at":time,"context_id":"fixture-ctx","plain":hex::encode(plain),"sealed":hex::encode(&sealed),"stream":hex::encode(&stream),"sealed_jcs":jcs(&sealed)?,"stream_jcs":jcs(&stream)?})
        )?
    );
    Ok(())
}
