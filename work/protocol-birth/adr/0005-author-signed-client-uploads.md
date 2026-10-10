# 0005. Author-signed uploads, sent by the client directly to Turbo

- Status: Accepted, not yet implemented
- Date: 2026-10-10

## Context

Each memory has two signatures. The inner signature (COSE) over the memory is made by
the user's key. The outer upload envelope (ANS-104) was signed by the operator, so
Arweave listed every memory under the operator's address. This hid authorship from
the Arweave index, made it hard to find a user's own memories, and put all users on the
operator's single Turbo free tier.

## Decision

- The author's key signs both the memory and the upload envelope.
- No server or operator ever signs an upload.
- The client (SDK, local MCP, browser) uploads directly to Turbo. The operator is not in
  the data path.

## Consequences

- Arweave lists each memory under its author's address.
- A client finds a user's memories by owner address.
- The indexer proves authorship from Arweave metadata; the planned signed-header tags are
  probably not needed.
- Each user has their own Turbo free tier, per wallet and per IP.
- The client needs the ANS-104 builder (Rust core, exposed to WebAssembly).
- Signing is not paying: payment is decided in 0008.

## Alternatives considered

- Operator signs and relays: the operator owns the user's data on Arweave and is in the data path.
- Operator relays a user-signed item: all users share the operator's IP free-tier limit.
