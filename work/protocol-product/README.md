# Mnemonik product and protocol

Status: product contract with implementation evidence. Source implementation and release acceptance are tracked separately.
Reviewed baseline: `89532db4556e53e81b5243c57535a76046c08207`, checked on 2026-10-02.
Task 1 records source and test evidence; implementation and release acceptance remain separate.

Mnemonik gives an agent one service to save and restore signed memory.
It accepts a finished artifact, delivers it through external infrastructure,
checks delivery, and returns evidence that the client can verify.

The owner has no current goal to issue a native token.
Product delivery does not depend on token issuance or a new blockchain.

## Documents

| Document | Purpose |
|---|---|
| [User specification](user-spec.md) | Problem, buyer, value and user journeys |
| [Technical specification](tech-spec.md) | Normative target architecture and contracts |
| [Surface map](surfaces.md) | Data, trust, modules and responsibility |
| [Product plan](product-plan.md) | Packaging, sales tests and funding choices |
| [Gap register](gaps.md) | Code evidence, remaining work and closure tests |
| [Baseline audit](baseline-audit.md) | Baseline paths, existing tests and claim-to-release map |
| [Release evidence](release-evidence.md) | Integrated task 5 drills, live observations and unresolved gates |
| [Implementation evidence](implementation-evidence.md) | Executed checks, supported scope and remaining release gates |
| [Repository inventory](repository-inventory.md) | Package boundaries, migration work and proposed extraction gates |
| [Decisions](decisions.md) | Owner inputs, recommendations and open choices |
| [Proof of concept](proof-of-concept.md) | Research handoff roles, flow and failure demonstration |
| [Acceptance plan](acceptance.md) | Evidence required before product claims |
| [Indexer contract](indexer-spec.md) | Discovery role, interface and replacement |
| [Storage portability](storage-portability-spec.md) | Recovery conditions and exact-byte migration |
| [Repository plan](repository-plan.md) | Proposed extraction boundaries and versioning |
| [Flows](flows.md) | Information, recovery and payment diagrams |
| [Shipping plan](shipping-plan.md) | Implementation order, release evidence and customer pilots |
| [Source capabilities](../../docs/source-capabilities.md) | Tested source support and explicit release limits |
| [Research handoff demo](../../docs/research-handoff-demo.md) | Repeatable synthetic recovery and continuation evidence |
| [Pilot runbook](pilot-runbook.md) | Candidate provenance, backups, rollback and release gates |
| [Tasks](tasks/1.md) | Ordered implementation work |

## Existing contracts

[A2A recovery](../sealed-memories/a2a-recovery-spec.md) owns the Agent-to-Agent (A2A)
wire bindings, parent checks, discovery and restoration details.
Tasks 18–21 there already cover that implementation. Do not create competing versions.

[Storage source of truth](../arweave-as-source-of-truth/tech-spec.md),
[decoupling sequence](../DECOUPLING-SEQUENCE.md),
[pluggable storage](../pluggable-storage/tech-spec.md), and
[payment integration](../universal-paywall-integration/tech-spec.md) remain related contracts.
Resolve conflicts through their append-only decision logs before implementation.

This package defines the product scope across those efforts.
It does not change existing signed formats, release gates, or payment rules by itself.

## Meaning of closure

A written contract closes a specification gap only.
An implementation gap closes after code, tests and recorded evidence meet its acceptance criteria.
A product hypothesis closes after customer evidence supports it.
No issue closes automatically when this documentation merges.
