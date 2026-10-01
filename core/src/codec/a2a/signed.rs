//! Client-signed A2A transport. The server verifies bytes and never opens them.
use super::sealed_part::{build_sealed_data_part, extract_sealed_data_part};
use crate::codec::{
    canonical::{from_canonical_cbor, to_canonical_cbor},
    schema::GRANT_V1,
    sign::{sign_cose, verify_artifact},
};
use anyhow::{bail, ensure, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_sdk::signature::{Keypair, Signer};

pub const MAX_A2A_BYTES: usize = 1024 * 1024;

/// A trusted key must come from the caller's trust configuration, not the card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientCard {
    pub card: Value,
    pub trusted_card_signer: String,
}

/// Metadata is signed together with the public A2A payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct A2aBinding {
    pub protocol: String,
    pub kind: String,
    pub context_id: String,
    pub prev_id: Option<String>,
    pub created_at: String,
    pub sealed: bool,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<crate::sealed::stream::SealedStream>,
}

#[derive(Debug)]
pub struct VerifiedBinding {
    pub binding: A2aBinding,
    pub signer: String,
    pub content_hash: String,
    pub sealed_cose: Option<Vec<u8>>,
    /// Recipient identity, signed grant bytes, and sealed memory hash.
    pub grants: Vec<(String, Vec<u8>, String)>,
}

fn signed_payload(bytes: &[u8], expected: Option<&str>) -> Result<(Vec<u8>, String)> {
    ensure!(bytes.len() <= MAX_A2A_BYTES, "A2A envelope too large");
    let v = verify_artifact(bytes, None).map_err(anyhow::Error::msg)?;
    ensure!(v.valid, "invalid COSE signature or algorithm");
    ensure!(expected.is_none_or(|s| s == v.signer), "unexpected signer");
    Ok((v.payload, v.signer))
}

/// Verify detached AgentCard JWS against a caller-pinned Ed25519 signing key.
/// JWS payload is JCS of the complete card without its signatures field.
pub fn recipient_from_verified_card(recipient: &RecipientCard) -> Result<(String, [u8; 32])> {
    let mut unsigned = recipient.card.clone();
    let signatures = unsigned
        .as_object_mut()
        .context("card must be an object")?
        .remove("signatures")
        .context("unsigned AgentCard")?;
    let canonical = serde_jcs::to_vec(&unsigned)?;
    ensure!(canonical.len() <= MAX_A2A_BYTES / 2, "AgentCard too large");
    let extensions = unsigned["extensions"].as_array().context("extensions")?;
    ensure!(
        extensions
            .iter()
            .filter(|e| e["uri"] == super::EXTENSION_URI)
            .count()
            == 1,
        "ambiguous x-mnemonic extension"
    );
    let trusted = bs58::decode(&recipient.trusted_card_signer).into_vec()?;
    let trusted: [u8; 32] = trusted
        .try_into()
        .map_err(|_| anyhow::anyhow!("bad trusted card key"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&trusted)?;
    ensure!(!key.is_weak(), "weak card signing key");
    let covered = signatures
        .as_array()
        .context("signatures must be an array")?
        .iter()
        .any(|s| {
            let verify = || -> Result<()> {
                let header = s
                    .get("protected")
                    .and_then(Value::as_str)
                    .context("missing protected")?;
                let h: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header)?)?;
                ensure!(
                    h["alg"] == "EdDSA" && h.get("crit").is_none() && h.get("b64").is_none(),
                    "unsupported JWS header"
                );
                let sig = URL_SAFE_NO_PAD.decode(
                    s.get("signature")
                        .and_then(Value::as_str)
                        .context("missing signature")?,
                )?;
                let input = format!("{}.{}", header, URL_SAFE_NO_PAD.encode(&canonical));
                key.verify_strict(
                    input.as_bytes(),
                    &ed25519_dalek::Signature::from_slice(&sig)?,
                )?;
                Ok(())
            };
            verify().is_ok()
        });
    ensure!(covered, "AgentCard JWS does not verify against trusted key");
    let ext = super::extract_x_mnemonic(&unsigned)?.context("missing x-mnemonic")?;
    let identity = ext.ed25519_pubkey_base58;
    let ext_json = unsigned["extensions"]
        .as_array()
        .context("extensions")?
        .iter()
        .find(|e| e["uri"] == super::EXTENSION_URI)
        .context("extension")?;
    let enc_key = ext_json.get("enc_key");
    let public = if let Some(enc) = enc_key {
        let raw = STANDARD.decode(
            enc.as_str()
                .context("enc_key must be base64 X25519 bytes")?,
        )?;
        raw.try_into()
            .map_err(|_| anyhow::anyhow!("enc_key must be 32 bytes"))?
    } else {
        super::recipient_x25519_from_agent_card(&unsigned)?
    };
    let identity_bytes = bs58::decode(&identity).into_vec()?;
    let identity_bytes: [u8; 32] = identity_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid recipient identity"))?;
    ensure!(
        !ed25519_dalek::VerifyingKey::from_bytes(&identity_bytes)?.is_weak(),
        "weak recipient identity"
    );
    ensure!(public != [0; 32], "invalid encryption key");
    Ok((identity, public))
}

fn validate_object(kind: &str, payload: &Value, context: &str) -> Result<()> {
    ensure!(payload.is_object(), "payload must be an object");
    let id = match kind {
        "task" => "id",
        "message" => "messageId",
        "artifact" => "artifactId",
        _ => bail!("invalid A2A kind"),
    };
    ensure!(
        payload[id].as_str().is_some_and(|s| !s.is_empty()),
        "missing A2A object id"
    );
    if let Some(ctx) = payload.get("contextId") {
        ensure!(ctx.as_str() == Some(context), "context mismatch");
    }
    if kind == "message" {
        ensure!(
            matches!(payload["role"].as_str(), Some("user" | "agent")),
            "invalid message role"
        );
    }
    if kind != "task" {
        ensure!(payload["parts"].is_array(), "missing parts");
    }
    Ok(())
}

/// Create all signatures in the client. K never crosses this API boundary.
pub fn prepare_signed_a2a(
    keypair: &Keypair,
    kind: &str,
    payload: Value,
    context_id: &str,
    prev_id: Option<String>,
    created_at: &str,
    recipients: Option<&[RecipientCard]>,
) -> Result<Vec<u8>> {
    prepare_signed_a2a_impl(
        keypair, kind, payload, context_id, prev_id, created_at, recipients, None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_signed_a2a_stream(
    keypair: &Keypair,
    kind: &str,
    payload: Value,
    context_id: &str,
    prev_id: Option<String>,
    created_at: &str,
    recipients: &[RecipientCard],
    chunk_size: usize,
) -> Result<Vec<u8>> {
    ensure!((1..=65536).contains(&chunk_size), "invalid chunk size");
    prepare_signed_a2a_impl(
        keypair,
        kind,
        payload,
        context_id,
        prev_id,
        created_at,
        Some(recipients),
        Some(chunk_size),
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_signed_a2a_impl(
    keypair: &Keypair,
    kind: &str,
    payload: Value,
    context_id: &str,
    prev_id: Option<String>,
    created_at: &str,
    recipients: Option<&[RecipientCard]>,
    chunk_size: Option<usize>,
) -> Result<Vec<u8>> {
    ensure!(!context_id.is_empty(), "empty context");
    validate_object(kind, &payload, context_id)?;
    let sealed = recipients.is_some();
    let mut stream = None;
    let payload = if let Some(recipients) = recipients {
        ensure!(
            kind != "task",
            "sealed payloads support message and artifact"
        );
        ensure!(
            !recipients.is_empty() && recipients.len() <= 64,
            "expected 1..64 recipients"
        );
        let keys = recipients
            .iter()
            .map(recipient_from_verified_card)
            .collect::<Result<Vec<_>>>()?;
        let inner = zeroize::Zeroizing::new(serde_jcs::to_vec(&payload)?);
        ensure!(inner.len() <= MAX_A2A_BYTES / 2, "payload too large");
        let author = keypair.pubkey().to_string();
        let descriptor = serde_json::to_vec(
            &json!({"stream":true,"payload_hash":blake3::hash(&inner).to_hex().to_string()}),
        )?;
        let encrypted = if chunk_size.is_some() {
            descriptor.as_slice()
        } else {
            inner.as_slice()
        };
        let artifact = crate::sealed::seal_memory(
            encrypted,
            &keypair.pubkey().to_bytes(),
            &format!("art:{}", uuid::Uuid::new_v4()),
            &author,
            created_at,
            &mut rand_core::OsRng,
        )?;
        if let Some(size) = chunk_size {
            let chunks =
                zeroize::Zeroizing::new(inner.chunks(size).map(|c| c.to_vec()).collect::<Vec<_>>());
            stream = Some(crate::sealed::stream::seal_stream(
                &chunks,
                &artifact.k,
                &uuid::Uuid::new_v4().to_string(),
                &hex::encode(&artifact.content_hash),
                &mut rand_core::OsRng,
            )?);
        }
        let sealed_cose = sign_cose(&artifact.outer_cbor, keypair).map_err(anyhow::Error::msg)?;
        let memory_hash: [u8; 32] = artifact.content_hash.as_slice().try_into()?;
        let mut grants = Vec::new();
        for (reader, public) in keys {
            let grant = crate::sealed::make_grant(
                &memory_hash,
                &artifact.outer_cbor,
                &artifact.k,
                Some(&public),
                &author,
                None,
                created_at,
            )?;
            let mut g = from_canonical_cbor(&grant).map_err(anyhow::Error::msg)?;
            g["reader"] = Value::String(reader);
            let cbor = to_canonical_cbor(&g, &GRANT_V1).map_err(anyhow::Error::msg)?;
            grants.push(sign_cose(&cbor, keypair).map_err(anyhow::Error::msg)?);
        }
        let part = build_sealed_data_part(&sealed_cose, &grants);
        // No names, plaintext parts, or other private fields survive in the carrier.
        match kind {
            "message" => {
                json!({"messageId":payload["messageId"], "role":payload["role"], "contextId":context_id, "parts":[part]})
            }
            _ => json!({"artifactId":payload["artifactId"], "parts":[part]}),
        }
    } else {
        payload
    };
    let binding = A2aBinding {
        protocol: "mnemonic.a2a.signed.v1".into(),
        kind: kind.into(),
        context_id: context_id.into(),
        prev_id,
        created_at: created_at.into(),
        sealed,
        payload,
        stream,
    };
    let jcs = serde_jcs::to_vec(&binding)?;
    ensure!(jcs.len() <= MAX_A2A_BYTES - 256, "A2A envelope too large");
    sign_cose(&jcs, keypair).map_err(anyhow::Error::msg)
}

/// Verify authorship, canonical bytes, context, and signed sealed/grant binding.
pub fn verify_signed_a2a(bytes: &[u8], expected_author: Option<&str>) -> Result<VerifiedBinding> {
    let (jcs, signer) = signed_payload(bytes, expected_author)?;
    let binding: A2aBinding = serde_json::from_slice(&jcs)?;
    ensure!(
        serde_jcs::to_vec(&binding)? == jcs,
        "noncanonical A2A binding"
    );
    ensure!(
        binding.protocol == "mnemonic.a2a.signed.v1",
        "unsupported binding"
    );
    ensure!(!binding.context_id.is_empty(), "empty context");
    chrono::DateTime::parse_from_rfc3339(&binding.created_at)?;
    ensure!(
        binding.prev_id.as_ref().is_none_or(|s| !s.is_empty()),
        "empty parent"
    );
    validate_object(&binding.kind, &binding.payload, &binding.context_id)?;
    let mut grants_out = Vec::new();
    let sealed_cose = if binding.sealed {
        ensure!(binding.kind != "task", "sealed task unsupported");
        let parts = binding.payload["parts"]
            .as_array()
            .context("missing sealed parts")?;
        ensure!(
            parts.len() == 1,
            "sealed carrier must contain exactly one part"
        );
        // Reject extra plaintext metadata on a sealed carrier.
        let allowed: &[&str] = if binding.kind == "message" {
            &["messageId", "role", "contextId", "parts"]
        } else {
            &["artifactId", "parts"]
        };
        ensure!(
            binding
                .payload
                .as_object()
                .context("object")?
                .keys()
                .all(|k| allowed.contains(&k.as_str())),
            "private metadata in carrier"
        );
        ensure!(
            parts[0]
                .as_object()
                .context("sealed part")?
                .keys()
                .all(|k| ["kind", "data", "mimeType"].contains(&k.as_str())),
            "private metadata in sealed part"
        );
        ensure!(
            parts[0]["data"]
                .as_object()
                .context("sealed data")?
                .keys()
                .all(|k| ["sealed", "grants"].contains(&k.as_str())),
            "private metadata in sealed data"
        );
        let part = serde_json::from_value(parts[0].clone())?;
        let (sealed, grants) = extract_sealed_data_part(&part)?;
        ensure!(
            !grants.is_empty() && grants.len() <= 64,
            "expected targeted grants"
        );
        let (outer, _) = signed_payload(&sealed, Some(&signer))?;
        let s = from_canonical_cbor(&outer).map_err(anyhow::Error::msg)?;
        ensure!(
            s["type"] == "sealed" && s["producer"] == signer && s["schema_version"] == 1,
            "invalid sealed author/schema"
        );
        let memory_hash = blake3::hash(&outer);
        let expected_hash = STANDARD.encode(memory_hash.as_bytes());
        if let Some(stream) = &binding.stream {
            crate::sealed::stream::verify_stream_chain(stream, memory_hash.to_hex().as_ref())?;
        }
        for grant in grants {
            let (g_cbor, _) = signed_payload(&grant, Some(&signer))?;
            let g = from_canonical_cbor(&g_cbor).map_err(anyhow::Error::msg)?;
            ensure!(
                g["type"] == "grant"
                    && g["schema_version"] == 1
                    && g["producer"] == signer
                    && g["memory_hash"] == expected_hash,
                "grant binding mismatch"
            );
            ensure!(
                g.get("perms").is_none_or(Value::is_null),
                "grant permissions are unsupported in A2A V1"
            );
            let reader = g["reader"]
                .as_str()
                .context("targeted reader required")?
                .to_owned();
            ensure!(
                bs58::decode(&reader).into_vec()?.len() == 32,
                "bad reader identity"
            );
            ensure!(
                !STANDARD
                    .decode(g["wk"].as_str().context("missing wrapped key")?)?
                    .is_empty(),
                "bearer grant forbidden"
            );
            grants_out.push((reader, grant, memory_hash.to_hex().to_string()));
        }
        Some(sealed)
    } else {
        ensure!(binding.stream.is_none(), "unsealed stream unsupported");
        None
    };
    Ok(VerifiedBinding {
        binding,
        signer,
        content_hash: blake3::hash(&jcs).to_hex().to_string(),
        sealed_cose,
        grants: grants_out,
    })
}

/// A signed parent reference is not a general context write capability.
pub fn verify_parent_link(child: &VerifiedBinding, parent: &VerifiedBinding) -> Result<()> {
    ensure!(
        child.binding.prev_id.as_deref() == Some(format!("a2a:{}", parent.content_hash).as_str()),
        "parent hash mismatch"
    );
    ensure!(child.content_hash != parent.content_hash, "self link");
    ensure!(
        child.binding.context_id == parent.binding.context_id,
        "parent context mismatch"
    );
    ensure!(
        child.signer == parent.signer
            || parent
                .grants
                .iter()
                .any(|(reader, _, _)| reader == &child.signer),
        "parent link not eligible"
    );
    Ok(())
}

/// Verify then decrypt locally. A trusted author is mandatory at the client.
pub fn open_signed_a2a(
    bytes: &[u8],
    keypair: &Keypair,
    expected_author: &str,
    encryption_secret: Option<&[u8; 32]>,
) -> Result<Value> {
    let v = verify_signed_a2a(bytes, Some(expected_author))?;
    if !v.binding.sealed {
        return Ok(v.binding.payload);
    }
    let sealed = v.sealed_cose.context("missing sealed envelope")?;
    let (outer, _) = signed_payload(&sealed, Some(expected_author))?;
    let derived = crate::sealed::x25519_secret_from_solana_keypair(keypair);
    let secret = encryption_secret.unwrap_or(&derived);
    let reader = keypair.pubkey().to_string();
    let open = |k: &[u8; 32]| -> Result<Vec<u8>> {
        let bytes = crate::sealed::open_with_key(&outer, k)?;
        if let Some(stream) = &v.binding.stream {
            let descriptor: Value = serde_json::from_slice(&bytes)?;
            let chunks = zeroize::Zeroizing::new(crate::sealed::stream::open_stream(
                stream,
                k,
                blake3::hash(&outer).to_hex().as_ref(),
            )?);
            let mut joined = zeroize::Zeroizing::new(chunks.concat());
            ensure!(
                descriptor["stream"] == true
                    && descriptor["payload_hash"] == blake3::hash(&joined).to_hex().to_string(),
                "stream payload mismatch"
            );
            Ok(std::mem::take(&mut *joined))
        } else {
            Ok(bytes)
        }
    };
    let inner = if reader == v.signer {
        let s = from_canonical_cbor(&outer).map_err(anyhow::Error::msg)?;
        let wrap = s["wraps"]
            .as_array()
            .context("missing author wrap")?
            .first()
            .context("missing author wrap")?;
        let enc = STANDARD.decode(wrap["enc"].as_str().context("enc")?)?;
        let wk = STANDARD.decode(wrap["wk"].as_str().context("wk")?)?;
        let ct = STANDARD.decode(s["ct"].as_str().context("ct")?)?;
        let k = crate::sealed::unwrap_key(
            &enc,
            &wk,
            &derived,
            blake3::hash(&ct).as_bytes(),
            &v.signer,
        )?;
        open(&k)?
    } else {
        let mut opened = None;
        for (kid, grant, _) in v.grants {
            if kid != reader {
                continue;
            }
            let (cbor, _) = signed_payload(&grant, Some(expected_author))?;
            if let Ok(k) = crate::sealed::open_grant(&cbor, secret) {
                opened = Some(open(&k)?);
                break;
            }
        }
        opened.context("not a recipient or invalid wrapped key")?
    };
    let inner = zeroize::Zeroizing::new(inner);
    let payload: Value = serde_json::from_slice(&inner)?;
    validate_object(&v.binding.kind, &payload, &v.binding.context_id)?;
    let id = if v.binding.kind == "message" {
        "messageId"
    } else {
        "artifactId"
    };
    ensure!(
        payload[id] == v.binding.payload[id],
        "inner/outer object id mismatch"
    );
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn recipient(seed: u8) -> (Keypair, RecipientCard) {
        let kp = Keypair::new_from_array([seed; 32]);
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let mut card = json!({"name":"recipient","url":"https://agent.test", "extensions":[super::super::build_x_mnemonic_extension(&kp.pubkey().to_string(),None)]});
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA"}"#);
        let input = format!(
            "{}.{}",
            header,
            URL_SAFE_NO_PAD.encode(serde_jcs::to_vec(&card).unwrap())
        );
        let signature = URL_SAFE_NO_PAD.encode(signing.sign(input.as_bytes()).to_bytes());
        card["signatures"] = json!([{"protected":header,"signature":signature}]);
        let recipient = RecipientCard {
            card,
            trusted_card_signer: kp.pubkey().to_string(),
        };
        (kp, recipient)
    }

    fn payload(kind: &str) -> Value {
        if kind == "message" {
            json!({"messageId":"msg","contextId":"ctx","role":"agent","parts":[{"kind":"text","text":"PRIVATE SENTINEL"}]})
        } else {
            json!({"artifactId":"art","name":"PRIVATE NAME","parts":[{"kind":"text","text":"PRIVATE SENTINEL"}]})
        }
    }

    #[test]
    fn client_sealed_message_artifact_and_stream_round_trip() {
        let author = Keypair::new_from_array([1; 32]);
        let (reader, card) = recipient(2);
        let outsider = Keypair::new_from_array([3; 32]);
        for kind in ["message", "artifact"] {
            for streaming in [false, true] {
                let inner = payload(kind);
                let signed = if streaming {
                    prepare_signed_a2a_stream(
                        &author,
                        kind,
                        inner.clone(),
                        "ctx",
                        None,
                        "2026-10-01T00:00:00Z",
                        std::slice::from_ref(&card),
                        17,
                    )
                    .unwrap()
                } else {
                    prepare_signed_a2a(
                        &author,
                        kind,
                        inner.clone(),
                        "ctx",
                        None,
                        "2026-10-01T00:00:00Z",
                        Some(std::slice::from_ref(&card)),
                    )
                    .unwrap()
                };
                let verified =
                    verify_signed_a2a(&signed, Some(&author.pubkey().to_string())).unwrap();
                assert!(!serde_json::to_string(&verified.binding)
                    .unwrap()
                    .contains("PRIVATE"));
                assert_eq!(
                    open_signed_a2a(&signed, &reader, &author.pubkey().to_string(), None).unwrap(),
                    inner
                );
                assert_eq!(
                    open_signed_a2a(&signed, &author, &author.pubkey().to_string(), None).unwrap(),
                    inner
                );
                assert!(
                    open_signed_a2a(&signed, &outsider, &author.pubkey().to_string(), None)
                        .is_err()
                );
                assert!(
                    open_signed_a2a(&signed, &reader, &outsider.pubkey().to_string(), None)
                        .is_err()
                );
                let mut tampered = signed.clone();
                let n = tampered.len();
                tampered[n - 1] ^= 1;
                assert!(verify_signed_a2a(&tampered, None).is_err());
            }
        }
    }

    #[test]
    fn verified_card_is_required_and_covers_enc_key() {
        let (kp, mut card) = recipient(2);
        assert!(recipient_from_verified_card(&card).is_ok());
        card.card["extensions"][0]["enc_key"] = Value::String(STANDARD.encode([9; 32]));
        assert!(recipient_from_verified_card(&card).is_err());
        let mut unsigned = card.card.clone();
        unsigned.as_object_mut().unwrap().remove("signatures");
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA"}"#);
        let input = format!(
            "{}.{}",
            header,
            URL_SAFE_NO_PAD.encode(serde_jcs::to_vec(&unsigned).unwrap())
        );
        let signature = SigningKey::from_bytes(&[2; 32]).sign(input.as_bytes());
        card.card["signatures"] =
            json!([{"protected":header,"signature":URL_SAFE_NO_PAD.encode(signature.to_bytes())}]);
        assert_eq!(recipient_from_verified_card(&card).unwrap().1, [9; 32]);
        card.trusted_card_signer = Keypair::new_from_array([8; 32]).pubkey().to_string();
        assert!(recipient_from_verified_card(&card).is_err());
        card.trusted_card_signer = kp.pubkey().to_string();
        card.card.as_object_mut().unwrap().remove("signatures");
        assert!(recipient_from_verified_card(&card).is_err());
    }

    #[test]
    fn substitution_and_truncated_stream_fail_even_when_transport_is_resigned() {
        let author = Keypair::new_from_array([1; 32]);
        let (_, card) = recipient(2);
        let signed = prepare_signed_a2a_stream(
            &author,
            "message",
            payload("message"),
            "ctx",
            None,
            "2026-10-01T00:00:00Z",
            &[card],
            10,
        )
        .unwrap();
        let mut v = verify_signed_a2a(&signed, None).unwrap();
        v.binding.stream.as_mut().unwrap().chunks.pop();
        let resigned = sign_cose(&serde_jcs::to_vec(&v.binding).unwrap(), &author).unwrap();
        assert!(verify_signed_a2a(&resigned, None).is_err());
        let mut v = verify_signed_a2a(&signed, None).unwrap();
        v.binding.context_id = "different".into();
        let resigned = sign_cose(&serde_jcs::to_vec(&v.binding).unwrap(), &author).unwrap();
        assert!(verify_signed_a2a(&resigned, None).is_err());
    }
    fn resign_card(card: &mut RecipientCard, seed: u8) {
        let mut unsigned = card.card.clone();
        unsigned.as_object_mut().unwrap().remove("signatures");
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA"}"#);
        let input = format!(
            "{}.{}",
            header,
            URL_SAFE_NO_PAD.encode(serde_jcs::to_vec(&unsigned).unwrap())
        );
        let signature = SigningKey::from_bytes(&[seed; 32]).sign(input.as_bytes());
        card.card["signatures"] =
            json!([{"protected":header,"signature":URL_SAFE_NO_PAD.encode(signature.to_bytes())}]);
    }

    #[test]
    fn separate_signed_encryption_key_and_duplicate_extension() {
        let author = Keypair::new_from_array([1; 32]);
        let (reader, mut card) = recipient(2);
        let separate = Keypair::new_from_array([9; 32]);
        let secret = crate::sealed::x25519_secret_from_solana_keypair(&separate);
        let public =
            crate::sealed::x25519_public_from_ed25519(&separate.pubkey().to_bytes()).unwrap();
        card.card["extensions"][0]["enc_key"] = json!(STANDARD.encode(public));
        resign_card(&mut card, 2);
        let signed = prepare_signed_a2a(
            &author,
            "message",
            payload("message"),
            "ctx",
            None,
            "2026-10-01T00:00:00Z",
            Some(std::slice::from_ref(&card)),
        )
        .unwrap();
        assert_eq!(
            open_signed_a2a(
                &signed,
                &reader,
                &author.pubkey().to_string(),
                Some(&secret)
            )
            .unwrap(),
            payload("message")
        );
        assert!(open_signed_a2a(&signed, &reader, &author.pubkey().to_string(), None).is_err());
        let ext = card.card["extensions"][0].clone();
        card.card["extensions"].as_array_mut().unwrap().push(ext);
        resign_card(&mut card, 2);
        assert!(recipient_from_verified_card(&card).is_err());
    }

    #[test]
    fn signed_grant_substitution_and_wrapped_key_tampering_fail() {
        let author = Keypair::new_from_array([1; 32]);
        let (reader, card) = recipient(2);
        let signed = prepare_signed_a2a(
            &author,
            "message",
            payload("message"),
            "ctx",
            None,
            "2026-10-01T00:00:00Z",
            Some(&[card]),
        )
        .unwrap();
        for field in ["wk", "memory_hash"] {
            let mut v = verify_signed_a2a(&signed, None).unwrap();
            let (grant_bytes, _) = signed_payload(&v.grants[0].1, None).unwrap();
            let mut g = from_canonical_cbor(&grant_bytes).unwrap();
            let mut bytes = STANDARD.decode(g[field].as_str().unwrap()).unwrap();
            bytes[0] ^= 1;
            g[field] = json!(STANDARD.encode(bytes));
            let grant = sign_cose(&to_canonical_cbor(&g, &GRANT_V1).unwrap(), &author).unwrap();
            v.binding.payload["parts"][0]["data"]["grants"][0] = json!(STANDARD.encode(grant));
            let resigned = sign_cose(&serde_jcs::to_vec(&v.binding).unwrap(), &author).unwrap();
            if field == "memory_hash" {
                assert!(verify_signed_a2a(&resigned, None).is_err());
            } else {
                assert!(verify_signed_a2a(&resigned, None).is_ok());
            }
            assert!(
                open_signed_a2a(&resigned, &reader, &author.pubkey().to_string(), None).is_err()
            );
        }
    }
}
