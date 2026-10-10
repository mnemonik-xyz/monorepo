# 0003. No Solana memo duplication

- Status: Accepted, in review (monorepo #283)
- Date: 2026-10-10

## Context

Each anchored memory used to write a Solana memo transaction that pointed at its storage
item. New writes no longer did so, and the old memos point only at items that are not
migrated (0002). The empty memo fields produced empty explorer links in live responses.

## Decision

Arweave is the only record. Remove the Solana memo layer: memo writing and reading,
memo-based recovery, `solana_tx`, `solana_explorer_url` and `anchoring_network` in
responses, and the `ANCHORING_NETWORK` setting.

Solana stays only for the key format (Ed25519, `did:sol`), x402 USDC payments and
buying Turbo credits with SOL.

## Consequences

- Breaking change for clients that read the removed fields.
- Recovery and statistics use Arweave only.
- The `solana_tx` database columns stay empty and are marked as legacy until a later migration.

## Alternatives considered

- Keep memo readers for old records: they point at data that is no longer readable.
