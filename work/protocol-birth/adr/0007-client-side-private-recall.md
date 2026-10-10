# 0007. Private recall happens on the client

- Status: Accepted, not yet implemented
- Date: 2026-10-10

## Context

In the live test, a private memory saved from Claude could not be recalled through the
hosted MCP: by design, the server never decrypts private memories, so recall returned
only "1 sealed memory not shown".

## Decision

Private memories are found, decrypted and searched on the client, with the user's key:
in the browser and through the SDK. One SDK implementation serves both; the browser page
reuses it. The server never decrypts.

## Consequences

- The SDK must find the user's memories on Arweave (by owner address, see 0005), fetch,
  decrypt and search them.
- Web agents get a link to open recall in the browser.
- Search in the browser needs a local method (a small embedding model, keywords or both).
- Open: how results reach the agent (copied by the user, or handed back per memory with
  the user's consent); cross-device discovery needs its own specification.

## Alternatives considered

- Make memories public by default: exposes everything on Arweave.
- Server-side recall sessions with the key: retired earlier for privacy reasons.
