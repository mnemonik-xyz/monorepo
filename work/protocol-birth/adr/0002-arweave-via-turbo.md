# 0002. Arweave is the single record; uploads go through ArDrive Turbo

- Status: Accepted, implemented (monorepo #282, #280, #281; coding-fabric #19, #23)
- Date: 2026-10-10

## Context

A live check showed that production uploads through the previous bundler never reached
Arweave: the bundler reported the items as confirmed but not seeded, and arweave.net
answered HTTP 404. A test upload of an unchanged ANS-104 item through ArDrive Turbo
reached an Arweave block in about 10 minutes and read back byte-exact from
arweave.net and ar-io.dev.

## Decision

- Arweave is the single record of every memory.
- Uploads go through ArDrive Turbo (`https://upload.ardrive.io/v1/tx/solana`).
- All code, configuration and documentation for the previous bundler are removed.
- Items stored only by the previous bundler are not migrated.
- A memory counts as permanent only when an Arweave gateway reports it in a block.

## Consequences

- Turbo has no test network: every upload is permanent on Arweave mainnet.
- Turbo's free tier covers items up to 105 KiB (10 MiB lifetime per wallet and per IP).
  Larger uploads need Turbo credits; without them Turbo answers HTTP 402.
- Old memories stored only by the previous bundler can no longer be read or restored.
- The protocol must not depend on Turbo alone; a fallback is still to design.

## Alternatives considered

- Keep the previous bundler: data did not reach Arweave.
- Write directly to Arweave: needs an Arweave wallet funded with AR on every operator,
  and confirmations take minutes per item.
- Re-upload the old items: rejected by the owner.
