//! Performance benchmark: CBOR canonicalization + COSE signing overhead.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ciborium::Value as CborValue;
use coset::{iana, CborSerializable, CoseSign1Builder, HeaderBuilder};
use solana_sdk::signature::{Keypair, Signer};

const MEMORY_V1_FIELD_ORDER: &[&str] = &[
    "artifact_id", "type", "schema_version", "content",
    "metadata", "parents", "tags", "created_at", "producer",
];

fn sample_artifact(content_size: usize) -> serde_json::Value {
    let content: String = "x".repeat(content_size);
    serde_json::json!({
        "artifact_id": "art:bench",
        "type": "memory",
        "schema_version": 1,
        "content": content,
        "producer": "did:sol:7xKXtg2CabcdefghijklmnopqrstuvwxyzABCDEFGH",
        "created_at": "2026-04-14T12:00:00Z",
        "tags": ["bench", "perf"],
        "metadata": {"source": "benchmark"},
    })
}

fn to_canonical_cbor(artifact: &serde_json::Value) -> Vec<u8> {
    let obj = artifact.as_object().unwrap();
    let mut entries: Vec<(CborValue, CborValue)> = Vec::new();
    for &field in MEMORY_V1_FIELD_ORDER {
        if let Some(v) = obj.get(field) {
            if !v.is_null() {
                entries.push((CborValue::Text(field.into()), json_to_cbor(v)));
            }
        }
    }
    let mut buf = Vec::new();
    ciborium::into_writer(&CborValue::Map(entries), &mut buf).unwrap();
    buf
}

fn json_to_cbor(j: &serde_json::Value) -> CborValue {
    match j {
        serde_json::Value::Null => CborValue::Null,
        serde_json::Value::Bool(b) => CborValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { CborValue::Integer(i.into()) }
            else { CborValue::Text(n.to_string()) }
        }
        serde_json::Value::String(s) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                CborValue::Tag(1, Box::new(CborValue::Integer(dt.timestamp().into())))
            } else { CborValue::Text(s.clone()) }
        }
        serde_json::Value::Array(a) => CborValue::Array(a.iter().map(json_to_cbor).collect()),
        serde_json::Value::Object(o) => {
            let mut e: Vec<_> = o.iter()
                .map(|(k, v)| (CborValue::Text(k.clone()), json_to_cbor(v)))
                .collect();
            e.sort_by(|a, b| {
                let ka = if let CborValue::Text(s) = &a.0 { s.as_str() } else { "" };
                let kb = if let CborValue::Text(s) = &b.0 { s.as_str() } else { "" };
                ka.cmp(kb)
            });
            CborValue::Map(e)
        }
    }
}

fn bench_cbor_canonicalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("cbor_canonicalize");

    for size in [100, 500, 2000, 10000] {
        let artifact = sample_artifact(size);
        group.bench_function(&format!("{size}B_content"), |b| {
            b.iter(|| to_canonical_cbor(black_box(&artifact)))
        });
    }
    group.finish();
}

fn bench_blake3_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_hash");

    for size in [100, 500, 2000, 10000] {
        let artifact = sample_artifact(size);
        let cbor = to_canonical_cbor(&artifact);
        group.bench_function(&format!("{size}B_content"), |b| {
            b.iter(|| blake3::hash(black_box(&cbor)))
        });
    }
    group.finish();
}

fn bench_cose_sign(c: &mut Criterion) {
    let kp = Keypair::new();
    let mut group = c.benchmark_group("cose_sign");

    for size in [100, 500, 2000] {
        let artifact = sample_artifact(size);
        let cbor = to_canonical_cbor(&artifact);
        group.bench_function(&format!("{size}B_content"), |b| {
            b.iter(|| {
                let protected = HeaderBuilder::new()
                    .algorithm(iana::Algorithm::EdDSA)
                    .content_type("application/cbor".to_string())
                    .build();
                let kid = kp.pubkey().to_string().into_bytes();
                let unprotected = HeaderBuilder::new().key_id(kid).build();
                let unsigned = CoseSign1Builder::new()
                    .protected(protected.clone())
                    .unprotected(unprotected.clone())
                    .payload(cbor.clone())
                    .build();
                let tbs = unsigned.tbs_data(&[]);
                let sig = kp.sign_message(&tbs);
                CoseSign1Builder::new()
                    .protected(protected)
                    .unprotected(unprotected)
                    .payload(cbor.clone())
                    .signature(sig.as_ref().to_vec())
                    .build()
                    .to_vec().unwrap()
            })
        });
    }
    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let kp = Keypair::new();
    let mut group = c.benchmark_group("full_pipeline");

    for size in [100, 500, 2000] {
        let artifact = sample_artifact(size);
        group.bench_function(&format!("{size}B_content"), |b| {
            b.iter(|| {
                let cbor = to_canonical_cbor(black_box(&artifact));
                let _hash = blake3::hash(&cbor);
                let protected = HeaderBuilder::new()
                    .algorithm(iana::Algorithm::EdDSA)
                    .build();
                let kid = kp.pubkey().to_string().into_bytes();
                let unprotected = HeaderBuilder::new().key_id(kid).build();
                let unsigned = CoseSign1Builder::new()
                    .protected(protected.clone())
                    .unprotected(unprotected.clone())
                    .payload(cbor.clone())
                    .build();
                let tbs = unsigned.tbs_data(&[]);
                let sig = kp.sign_message(&tbs);
                CoseSign1Builder::new()
                    .protected(protected)
                    .unprotected(unprotected)
                    .payload(cbor)
                    .signature(sig.as_ref().to_vec())
                    .build()
                    .to_vec().unwrap()
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_cbor_canonicalization,
    bench_blake3_hash,
    bench_cose_sign,
    bench_full_pipeline,
);
criterion_main!(benches);
