//! Per-owner Merkle commitments over memory `content_hash`es (Wave 5 —
//! verifiable recall, design §16).
//!
//! The non-custodial target demotes SQLite from a *trusted store* to a
//! *rebuildable cache*: the source of truth is Arweave (content) + a
//! Solana-anchored, per-owner Merkle root (completeness). `recall` returns
//! results **plus inclusion proofs**; a client checks each proof against the
//! anchored root, so an operator that omits or tampers with a row is
//! **detectable**. This module is the pure, chain-independent core of that
//! scheme — it computes the root and the proofs; anchoring the root on Solana
//! and distributing it to clients is the operator-relay integration layer.
//!
//! ## Construction (RFC 6962-style, with set semantics)
//! - Leaves are the 32-byte blake3 `content_hash`es of an owner's memories.
//! - The leaf set is **sorted** before the tree is built, so the root is a
//!   canonical commitment to the *set* (insertion-order-independent) — anyone
//!   can rebuild the identical root from Arweave without knowing write order.
//! - Domain separation guards against second-preimage attacks: a leaf is
//!   `blake3(0x00 || leaf)`, an internal node is `blake3(0x01 || left ||
//!   right)`. The two prefixes make a leaf hash never collide with an
//!   internal-node hash (CVE-2012-2459 class).
//! - Odd node at a level is **promoted** unchanged to the next level (no
//!   duplicate-the-last, which is the source of the Bitcoin merkle-malleability
//!   bug).
//!
//! All functions are deterministic and allocation-bounded; no I/O, no chain.

/// Domain-separation prefix for leaf hashing.
const LEAF_PREFIX: u8 = 0x00;
/// Domain-separation prefix for internal-node hashing.
const NODE_PREFIX: u8 = 0x01;

/// Root of the empty set — `blake3(0x02)`. A fixed sentinel distinct from any
/// leaf or internal node (which use prefixes 0x00 / 0x01), so an empty
/// commitment can never be confused with a single-element one.
pub fn empty_root() -> [u8; 32] {
    *blake3::hash(&[0x02u8]).as_bytes()
}

/// Hash a raw leaf value (a 32-byte `content_hash`) into a domain-separated
/// leaf node.
pub fn leaf_hash(leaf: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[LEAF_PREFIX]);
    h.update(leaf);
    *h.finalize().as_bytes()
}

/// Hash two child nodes into a domain-separated internal node.
fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[NODE_PREFIX]);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

/// Parse a 64-char lowercase-hex `content_hash` into 32 raw bytes.
/// Returns `None` on a malformed string (wrong length / non-hex).
pub fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Lowercase-hex encode a 32-byte digest.
pub fn to_hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Build the sorted, deduplicated leaf-hash vector from a set of hex
/// `content_hash`es. Invalid hex entries are skipped. The result is the
/// canonical leaf ordering used by both [`commitment_root`] and [`prove`].
fn sorted_leaf_hashes(content_hashes: &[String]) -> Vec<[u8; 32]> {
    let mut raw: Vec<[u8; 32]> = content_hashes
        .iter()
        .filter_map(|h| parse_hex32(h))
        .collect();
    raw.sort_unstable();
    raw.dedup();
    raw.iter().map(leaf_hash).collect()
}

/// One step in an inclusion proof: a sibling hash and whether it sits to the
/// **right** of the running hash at that level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofStep {
    pub sibling: [u8; 32],
    pub sibling_is_right: bool,
}

/// Fold one tree level into the next, promoting a trailing odd node.
fn fold_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next = Vec::with_capacity(level.len().div_ceil(2));
    let mut i = 0;
    while i + 1 < level.len() {
        next.push(node_hash(&level[i], &level[i + 1]));
        i += 2;
    }
    if i < level.len() {
        // Odd node out — promote unchanged.
        next.push(level[i]);
    }
    next
}

/// Compute the Merkle root over the set of hex `content_hash`es. Order- and
/// duplicate-independent (the set is sorted + deduped first). The empty set
/// commits to [`empty_root`].
pub fn commitment_root(content_hashes: &[String]) -> [u8; 32] {
    let mut level = sorted_leaf_hashes(content_hashes);
    if level.is_empty() {
        return empty_root();
    }
    while level.len() > 1 {
        level = fold_level(&level);
    }
    level[0]
}

/// Generate an inclusion proof for `target_hex` within the owner's set.
/// Returns `(root, proof)` so the caller can anchor/return the root alongside
/// the steps. `None` if `target_hex` is malformed or absent from the set.
pub fn prove(content_hashes: &[String], target_hex: &str) -> Option<([u8; 32], Vec<ProofStep>)> {
    let target_leaf = leaf_hash(&parse_hex32(target_hex)?);
    let mut level = sorted_leaf_hashes(content_hashes);
    let mut idx = level.iter().position(|h| *h == target_leaf)?;

    let mut proof = Vec::new();
    while level.len() > 1 {
        // Sibling within the current level (if any). A promoted odd node has
        // no sibling at this level — it simply carries up.
        if idx % 2 == 0 {
            if idx + 1 < level.len() {
                proof.push(ProofStep {
                    sibling: level[idx + 1],
                    sibling_is_right: true,
                });
            }
        } else {
            proof.push(ProofStep {
                sibling: level[idx - 1],
                sibling_is_right: false,
            });
        }
        idx /= 2;
        level = fold_level(&level);
    }
    Some((level[0], proof))
}

/// Verify an inclusion proof: recompute the root from `leaf_hex` + `proof` and
/// compare to `expected_root`. Returns `false` on malformed input.
pub fn verify(leaf_hex: &str, proof: &[ProofStep], expected_root: &[u8; 32]) -> bool {
    let Some(raw) = parse_hex32(leaf_hex) else {
        return false;
    };
    let mut running = leaf_hash(&raw);
    for step in proof {
        running = if step.sibling_is_right {
            node_hash(&running, &step.sibling)
        } else {
            node_hash(&step.sibling, &running)
        };
    }
    running == *expected_root
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo content_hash from a seed byte (64 hex chars).
    fn ch(seed: u8) -> String {
        to_hex32(blake3::hash(&[seed]).as_bytes())
    }

    fn set(seeds: &[u8]) -> Vec<String> {
        seeds.iter().map(|s| ch(*s)).collect()
    }

    #[test]
    fn empty_set_commits_to_empty_root() {
        assert_eq!(commitment_root(&[]), empty_root());
        // Empty root is distinct from any single-leaf root.
        assert_ne!(commitment_root(&set(&[1])), empty_root());
    }

    #[test]
    fn root_is_order_and_duplicate_independent() {
        let a = commitment_root(&set(&[1, 2, 3, 4, 5]));
        let b = commitment_root(&set(&[5, 4, 3, 2, 1]));
        let c = commitment_root(&[ch(3), ch(1), ch(2), ch(2), ch(4), ch(5), ch(3)]);
        assert_eq!(a, b, "root must be insertion-order independent");
        assert_eq!(a, c, "root must be duplicate independent (set semantics)");
    }

    #[test]
    fn root_changes_when_a_member_changes() {
        let a = commitment_root(&set(&[1, 2, 3]));
        let b = commitment_root(&set(&[1, 2, 4]));
        assert_ne!(a, b);
    }

    #[test]
    fn proof_verifies_for_every_member_across_sizes() {
        // Exercise odd/even level widths (promotion path) for n = 1..=9.
        for n in 1u8..=9 {
            let seeds: Vec<u8> = (1..=n).collect();
            let hashes = set(&seeds);
            for s in &seeds {
                let target = ch(*s);
                let (root, proof) = prove(&hashes, &target).expect("member must prove");
                assert_eq!(
                    root,
                    commitment_root(&hashes),
                    "proof root matches commitment"
                );
                assert!(
                    verify(&target, &proof, &root),
                    "n={n}: inclusion proof for member {s} must verify"
                );
            }
        }
    }

    #[test]
    fn non_member_has_no_proof() {
        let hashes = set(&[1, 2, 3]);
        assert!(prove(&hashes, &ch(99)).is_none());
        // Malformed target.
        assert!(prove(&hashes, "not-hex").is_none());
    }

    #[test]
    fn proof_fails_against_wrong_root_or_tampered_step() {
        let hashes = set(&[1, 2, 3, 4]);
        let target = ch(2);
        let (root, proof) = prove(&hashes, &target).unwrap();
        assert!(verify(&target, &proof, &root));

        // Wrong root (different set).
        let other_root = commitment_root(&set(&[7, 8, 9]));
        assert!(!verify(&target, &proof, &other_root));

        // Tampered sibling.
        let mut bad = proof.clone();
        bad[0].sibling[0] ^= 0xff;
        assert!(!verify(&target, &bad, &root));

        // Flipped side bit.
        let mut flipped = proof.clone();
        flipped[0].sibling_is_right = !flipped[0].sibling_is_right;
        assert!(!verify(&target, &flipped, &root));

        // Proving a different member must not verify against this leaf.
        let (_r, p3) = prove(&hashes, &ch(3)).unwrap();
        assert!(!verify(&target, &p3, &root));
    }

    #[test]
    fn single_member_proof_is_empty_and_root_is_its_leaf() {
        let hashes = set(&[42]);
        let (root, proof) = prove(&hashes, &ch(42)).unwrap();
        assert!(proof.is_empty(), "a 1-leaf tree needs no siblings");
        assert_eq!(root, leaf_hash(&parse_hex32(&ch(42)).unwrap()));
        assert!(verify(&ch(42), &proof, &root));
    }

    #[test]
    fn leaf_and_node_domains_are_separated() {
        let l = leaf_hash(&[0u8; 32]);
        let n = node_hash(&[0u8; 32], &[0u8; 32]);
        assert_ne!(
            l, n,
            "domain prefixes must separate leaf from internal node"
        );
    }

    #[test]
    fn parse_hex32_roundtrip_and_rejects_bad_input() {
        let h = ch(7);
        let raw = parse_hex32(&h).unwrap();
        assert_eq!(to_hex32(&raw), h);
        assert!(parse_hex32("short").is_none());
        assert!(parse_hex32(&"z".repeat(64)).is_none());
    }
}
