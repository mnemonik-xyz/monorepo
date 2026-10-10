# 0009. A separate open indexer replaces chain statistics

- Status: Accepted, not yet implemented
- Date: 2026-10-10

## Context

Each MCP server counted traction in its own `chain_stats`, which described one operator
only. With several operators and author-owned records (0005), protocol-wide numbers and
feeds need one neutral source that anyone can run and check.

## Decision

A separate, open, read-only indexer service:

- finds all protocol records on Arweave by the `App-Name: mnemonic-protocol` tag;
- attributes each record to its author (the envelope signer, 0005) and verifies signatures;
- serves feed and traction data to the webapp and others;
- replaces the MCP server's `chain_stats`.

## Consequences

- Anyone can run an indexer and get the same numbers.
- The indexer must resist spam and floods of tagged items: count only items whose
  envelope signer matches the signer inside the memory, rate-limit per author, rank
  unknown authors last.
- It never reads private content.
- The earlier draft specifications (`protocol-indexer`, `signed-header`) need rework for
  Arweave and author signing; `irys-fixes` is superseded by 0002.

## Alternatives considered

- Extend `chain_stats` in each operator: numbers stay per operator and depend on that operator.
