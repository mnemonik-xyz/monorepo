//! Reconstruct `StepRecord` / `VerdictRecord` from a **client-signed** COSE
//! envelope — the non-custodial path. The server never signs; it verifies the
//! signature, reads the producing/judging identity from the COSE `kid`, and
//! parses the signed CBOR payload for the typed fields. If the signature is
//! invalid the bytes are rejected before anything is stored.

use anyhow::{anyhow, bail};
use ciborium::value::Value as CborValue;

use super::store::{StepRecord, VerdictRecord};
use super::VerdictStatus;
use crate::codec::sign::verify_artifact;

fn decode_map(payload: &[u8]) -> anyhow::Result<Vec<(CborValue, CborValue)>> {
    match ciborium::de::from_reader(payload).map_err(|e| anyhow!("cbor decode: {e}"))? {
        CborValue::Map(m) => Ok(m),
        _ => bail!("signed payload is not a CBOR map"),
    }
}

fn text(map: &[(CborValue, CborValue)], key: &str) -> Option<String> {
    map.iter().find_map(|(k, v)| match (k, v) {
        (CborValue::Text(t), CborValue::Text(s)) if t == key => Some(s.clone()),
        _ => None,
    })
}

fn uint(map: &[(CborValue, CborValue)], key: &str) -> Option<u64> {
    map.iter().find_map(|(k, v)| match (k, v) {
        (CborValue::Text(t), CborValue::Integer(i)) if t == key => {
            let n: i128 = (*i).into();
            u64::try_from(n).ok()
        }
        _ => None,
    })
}

/// Verify a client-signed STEP envelope and reconstruct its record.
/// `producer` is taken from the COSE signer (`kid`), never from a server key.
pub fn step_from_cose(cose_bytes: &[u8]) -> anyhow::Result<StepRecord> {
    let vr = verify_artifact(cose_bytes, None).map_err(|e| anyhow!(e))?;
    if !vr.valid {
        bail!("invalid COSE signature on step");
    }
    let map = decode_map(&vr.payload)?;
    if text(&map, "type").as_deref() != Some("step") {
        bail!("envelope is not a step artifact");
    }
    Ok(StepRecord {
        trajectory_id: text(&map, "trajectory_id")
            .ok_or_else(|| anyhow!("missing trajectory_id"))?,
        seq: uint(&map, "seq").ok_or_else(|| anyhow!("missing seq"))?,
        prev_hash: text(&map, "prev_hash"),
        producer: vr.signer,
        content_hash: vr.content_hash,
        cose_bytes: cose_bytes.to_vec(),
        canonical_cbor: vr.payload,
    })
}

/// Verify a client-signed VERDICT envelope and reconstruct its record.
/// `judge` is taken from the COSE signer (`kid`).
pub fn verdict_from_cose(cose_bytes: &[u8]) -> anyhow::Result<VerdictRecord> {
    let vr = verify_artifact(cose_bytes, None).map_err(|e| anyhow!(e))?;
    if !vr.valid {
        bail!("invalid COSE signature on verdict");
    }
    let map = decode_map(&vr.payload)?;
    if text(&map, "type").as_deref() != Some("verdict") {
        bail!("envelope is not a verdict artifact");
    }
    let status = text(&map, "status")
        .and_then(|s| VerdictStatus::from_str(&s))
        .ok_or_else(|| anyhow!("missing/invalid status"))?;
    Ok(VerdictRecord {
        step_hash: text(&map, "step_hash").ok_or_else(|| anyhow!("missing step_hash"))?,
        status,
        judge: vr.signer,
        content_hash: vr.content_hash,
        cose_bytes: cose_bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{build_step, build_verdict, StepInput, VerdictInput};
    use solana_sdk::signature::{Keypair, Signer};

    #[test]
    fn step_roundtrips_from_cose() {
        let kp = Keypair::new();
        let signed = build_step(
            &StepInput {
                trajectory_id: "t",
                seq: 3,
                content: "act",
                prev_hash: Some("abc123"),
                created_at: "2026-06-27T00:00:00Z",
            },
            &kp,
        )
        .unwrap();
        let rec = step_from_cose(&signed.cose_bytes).unwrap();
        assert_eq!(rec.trajectory_id, "t");
        assert_eq!(rec.seq, 3);
        assert_eq!(rec.prev_hash.as_deref(), Some("abc123"));
        assert_eq!(rec.producer, kp.pubkey().to_string());
        assert_eq!(rec.content_hash, signed.content_hash);
    }

    #[test]
    fn genesis_step_has_no_prev_hash() {
        let kp = Keypair::new();
        let signed = build_step(
            &StepInput {
                trajectory_id: "t",
                seq: 0,
                content: "plan",
                prev_hash: None,
                created_at: "2026-06-27T00:00:00Z",
            },
            &kp,
        )
        .unwrap();
        let rec = step_from_cose(&signed.cose_bytes).unwrap();
        assert_eq!(rec.seq, 0);
        assert!(rec.prev_hash.is_none());
    }

    #[test]
    fn verdict_roundtrips_with_judge_identity() {
        let judge = Keypair::new();
        let signed = build_verdict(
            &VerdictInput {
                step_hash: "deadbeef",
                status: VerdictStatus::Pass,
                score: Some(0.9),
                proof_ref: None,
                proof_kind: Some("prm"),
                rationale: None,
                created_at: "2026-06-27T00:00:00Z",
            },
            &judge,
        )
        .unwrap();
        let rec = verdict_from_cose(&signed.cose_bytes).unwrap();
        assert_eq!(rec.step_hash, "deadbeef");
        assert_eq!(rec.status, VerdictStatus::Pass);
        assert_eq!(rec.judge, judge.pubkey().to_string());
    }

    #[test]
    fn tampered_envelope_rejected() {
        let kp = Keypair::new();
        let signed = build_step(
            &StepInput {
                trajectory_id: "t",
                seq: 0,
                content: "plan",
                prev_hash: None,
                created_at: "2026-06-27T00:00:00Z",
            },
            &kp,
        )
        .unwrap();
        let mut bytes = signed.cose_bytes.clone();
        let n = bytes.len();
        bytes[n - 1] ^= 0xff; // corrupt the signature
        assert!(step_from_cose(&bytes).is_err());
    }
}
