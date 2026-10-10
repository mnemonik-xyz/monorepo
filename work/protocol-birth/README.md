# Protocol birth

Mnemonic changes from a hosted service into a protocol that anyone can implement:
a client, an operator or an indexer, using only public rules.

- [Architecture](architecture.md): the decided architecture.
- [Architecture decision records](adr/): one record per decision, numbered in the order they were made.
- Brainstorm and open questions: [Mnemonic Protocol Architecture (brainstorm)](https://claude.ai/code/artifact/69372256-17a0-4026-b68a-606a0d94bad1).

## Decision records

| # | Decision | Status |
| --- | --- | --- |
| [0001](adr/0001-record-architecture-decisions.md) | Record architecture decisions as ADRs | Accepted |
| [0002](adr/0002-arweave-via-turbo.md) | Arweave is the single record; uploads through ArDrive Turbo | Accepted, implemented |
| [0003](adr/0003-no-solana-memos.md) | No Solana memo duplication | Accepted, in review |
| [0004](adr/0004-gateway-read-failover.md) | Read failover across Arweave gateways | Accepted, implemented |
| [0005](adr/0005-author-signed-client-uploads.md) | Author-signed uploads, sent by the client directly to Turbo | Accepted |
| [0006](adr/0006-private-never-visible-to-operators.md) | Private memories are never visible to operators | Accepted |
| [0007](adr/0007-client-side-private-recall.md) | Private recall happens on the client | Accepted |
| [0008](adr/0008-payments-and-quota.md) | Payments: free tier, x402 to the operator, Turbo credit sharing | Accepted |
| [0009](adr/0009-open-indexer.md) | A separate open indexer replaces chain statistics | Accepted |
| [0010](adr/0010-registries-and-staking.md) | Operator and agent registries on ERC-8004, starting on Arc | Accepted, details open |
| [0011](adr/0011-identity-v1-one-key.md) | Identity version 1: one key per user | Accepted |
| [0012](adr/0012-scope-limits.md) | Agents do not run code; escrow belongs to Warrant | Accepted |

## How to add a decision

1. Copy the newest record and give it the next number.
2. Set the status to Proposed.
3. Change the status to Accepted after the owner agrees.
4. Do not rewrite an accepted record. Write a new record that supersedes it, and link both ways.
