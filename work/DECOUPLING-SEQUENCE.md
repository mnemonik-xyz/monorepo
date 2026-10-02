# Decoupling sequence

## Current evidence — 2026-10-02

The original table below records intended ordering, not current implementation status.
Enumeration code shipped, and `1edebe7` removed memo writes from ordinary memory helpers and the callback.
The direct sealed anchor route still submits a memo.
The live [enumeration parity gate](chain-agnostic/tasks/T2-stage1-gate.md) remains unclosed in the reviewed records.
Mocked stage 2 tests do not satisfy this gate.
See the [baseline audit](protocol-product/baseline-audit.md) for current paths and remaining acceptance.
This update neither waives the gate nor changes runtime behavior.

## Original sequence — 2026-09-28

One page naming the order of the decoupling work and, more usefully, **why each step blocks the
next**. Written 2026-09-28.

Each item has its own folder. This file exists because the dependencies between them are not
obvious, and getting the order wrong is the only way this work destroys value.

## The order

| # | Work | Blocks the next because | Status |
|---|---|---|---|
| 0 | [`arweave-as-source-of-truth`](arweave-as-source-of-truth/) | Until Arweave is the only copy, the operator's database is a silent fallback and nothing below is testable | **shipped** (#246–#252) |
| 1 | [`chain-agnostic`](chain-agnostic/) stage 1 — enumeration | The Solana memo is today the ONLY working enumeration source. Nothing may remove it before a replacement is **measured** | next |
| 2 | [`pluggable-anchoring`](pluggable-anchoring/) — Solana as a **choice** | Makes step 3 a config change rather than a code deletion, so it is reversible on a deploy | after 1 |
| 3 | [`chain-agnostic`](chain-agnostic/) stage 2 — select `Anchor::None` | Frees the per-write Solana fee. Readers stay for ever | after 2, and only past the gate |
| 4 | [`dual-key-identity`](dual-key-identity/) | ERC-8004 needs an EVM address bound to the Mnemonic identity. Cheap, and it removes the only urgent reason for step 7 | parallel with 1–3 |
| 5 | [`erc8004-reputation`](erc8004-reputation/) | Consumes the dual-key binding. Otherwise the `mnemonic` namespace proves nothing about the rater | after 4 |
| 6 | [`pluggable-storage`](pluggable-storage/) | Generalises the enumeration problem solved in step 1. Doing it first would mean solving that problem twice | after 1 |
| 7 | [`multi-suite-signing`](multi-suite-signing/) | Unrecoverable failure mode. Last, and only if dual-key identity proves insufficient | last |
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

**4. Prefer making a thing optional over deleting it.**

`pluggable-anchoring` sits before the memo removal for a reason discovered while writing these
specs: an `AnchorWriter` selection expresses "no Solana anchor" as a configuration value rather
than as deleted code. Choosing `Anchor::None` is reversible on a deploy; deleting the write path
is reversible only on a revert.

That also answers `chain-agnostic` Q-1, which asked whether the removal should hide behind an
environment flag. It should not need one — the anchor type *is* the flag.

It does not weaken constraint 1. Pluggability makes the switch reversible; only a measured
enumeration result makes it safe.

## What is deliberately not sequenced

`sealed-memories` has no dependency on any of this. Its waves 1–5 touch only `core/`, and
`work/sealed-memories/` records that they can run in parallel with anything here. Its open owner
decision D-2 (where the X25519 key comes from) is its own blocker, unrelated to decoupling.

One overlap to coordinate rather than sequence: both `dual-key-identity` and `sealed-memories`
add a key to the AgentCard `x-mnemonic` extension — an EVM address and an X25519 key. Design the
extension once, with both fields, rather than extending it twice.
