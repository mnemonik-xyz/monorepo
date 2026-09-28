# Decoupling sequence

One page naming the order of the decoupling work and, more usefully, **why each step blocks the
next**. Written 2026-09-28.

Each item has its own folder. This file exists because the dependencies between them are not
obvious, and getting the order wrong is the only way this work destroys value.

## The order

| # | Work | Blocks the next because | Status |
|---|---|---|---|
| 0 | [`arweave-as-source-of-truth`](arweave-as-source-of-truth/) | Until Arweave is the only copy, the operator's database is a silent fallback and nothing below is testable | **shipped** (#246–#252) |
| 1 | [`chain-agnostic`](chain-agnostic/) stage 1 — enumeration | The Solana memo is today the ONLY working enumeration source. Nothing may remove it before a replacement is **measured** | next |
| 2 | [`chain-agnostic`](chain-agnostic/) stage 2 — drop the memo writer | Frees the per-write Solana fee. Readers stay for ever | after 1 |
| 3 | [`dual-key-identity`](dual-key-identity/) | ERC-8004 needs an EVM address bound to the Mnemonic identity. Cheap, and it removes the only urgent reason for step 6 | parallel with 1–2 |
| 4 | [`erc8004-reputation`](erc8004-reputation/) | Consumes step 3's binding. Otherwise the `mnemonic` namespace proves nothing about the rater | after 3 |
| 5 | [`pluggable-storage`](pluggable-storage/) | Generalises the enumeration problem solved in step 1. Doing it first would mean solving that problem twice | after 1 |
| 6 | [`multi-suite-signing`](multi-suite-signing/) | Unrecoverable failure mode. Last, and only if step 3 proves insufficient | last |
| — | [`sealed-memories`](sealed-memories/) | Independent of all of the above. Its waves 1–5 are pure `core/` work with no conflicts | any time |

## The three constraints that set this order

**1. Never remove an enumeration source before its replacement is measured.**

Arweave-schema gateways return **zero** items for our `App-Name` tag, because Irys bundles them
and those gateways index the containing bundle. The Irys endpoint returns them. Verified
2026-09-27.

So the Solana memo is currently the only thing that can list an owner's anchored memories. Remove
its writer first and every existing anchored memory becomes unenumerable — and so unrestorable —
while its bytes sit perfectly intact on Arweave.

A code review cannot clear this. An empty index and a rejected query are indistinguishable to the
caller, which is exactly how a misconfigured endpoint survived unnoticed for weeks. Step 2 is
gated on a measured superset result against real data.

**2. Identity changes are ordered by how bad the failure is, not by effort.**

Step 3 (binding two keys) cannot invalidate an existing attestation, because it never touches the
signature suite. Step 6 (signing with a different suite) can invalidate **every** attestation,
including bytes already permanent on Arweave that cannot be rewritten.

Same subject area, opposite risk. So step 3 is early and step 6 is last — and step 3 removes the
only pressing reason to attempt step 6 at all.

**3. Storage and anchoring are different axes.**

Storage is where the bytes live. Anchoring is where the hash is committed. They are easy to
conflate — one confusion between `chain_stats_gateway_url` (payload) and
`chain_stats_graphql_url` (index) already produced a real bug, fixed in #256.

`pluggable-storage` comes after `chain-agnostic` stage 1 because both need "list everything this
owner stored", and solving it once generically beats solving it twice.

## What is deliberately not sequenced

`sealed-memories` has no dependency on any of this. Its waves 1–5 touch only `core/`, and
`work/sealed-memories/` records that they can run in parallel with anything here. Its open owner
decision D-2 (where the X25519 key comes from) is its own blocker, unrelated to decoupling.

One overlap to coordinate rather than sequence: both `dual-key-identity` and `sealed-memories`
add a key to the AgentCard `x-mnemonic` extension — an EVM address and an X25519 key. Design the
extension once, with both fields, rather than extending it twice.
