# Mnemonic protocol architecture

Status: decided on 2026-10-10, except where a section says "open".
Open questions and options are in the
[brainstorm document](https://claude.ai/code/artifact/69372256-17a0-4026-b68a-606a0d94bad1).

## Summary

Mnemonic is a decentralized memory layer for AI agents. A user's memories live on
Arweave under the user's own address. Only the user's client holds the key, signs
and reads private content. Operators, indexers and registries provide services, but
they never sign for users and never see private text.

## Layers

| Layer | Who builds or runs it | Holds user keys | Sees private text |
| --- | --- | --- | --- |
| Protocol | Open specification | No | No |
| Client: SDK, local MCP, browser webapp, extension | Users and their tools | Yes, only here | Yes, only here |
| Operator: hosted MCP gateway | Anyone with a stake in the registry | No | No |
| Indexer | Anyone | No | No |
| Registry | Smart contracts (ERC-8004) | No | No |

## Roles

| Part | Does | Never does |
| --- | --- | --- |
| Client | Holds the key. Encrypts private memories. Signs the memory and the upload envelope. Uploads to ArDrive Turbo. Recalls and decrypts private memories. Verifies what it reads. | Sends private text or keys to anyone. |
| Local MCP | Gives AI tools on the user's machine save, recall and share, with no approval page. | Exposes the key to the agent. |
| Hosted operator | Bridges web agents to the user's client. Handles public memories. Sells writes (x402) and grants Turbo credits. Runs public semantic search. Relays signed or encrypted agent messages. Handles accounts, rate limits and abuse. | Holds user keys, signs, decrypts, or keeps the record of truth. |
| Indexer | Lists every protocol record on Arweave, attributes it to its author, verifies signatures, serves feed and traction data. | Reads private content. |
| Registry | Lists operators and agents with keys, endpoints, capabilities and stake. | Decides whom a user trusts. |

## Data flow

1. **Write, local MCP or SDK.** The client builds the memory, encrypts it when it is
   private, signs the memory (COSE) and the upload envelope (ANS-104) with the user's
   key, and uploads it to ArDrive Turbo. Turbo bundles it into Arweave.
2. **Write, web agent (for example claude.ai).**
   - Public memory: the agent calls the operator; the user approves and signs in the
     browser; the browser uploads.
   - Private memory: the agent gives the user a link such as
     `https://www.mnemonik.xyz/save#<text>`. The URL fragment never reaches a server.
     The browser encrypts, signs and uploads. The operator never receives the text.
3. **Delivery status.** After signing, the page shows: uploaded, in a bundle,
   permanent on Arweave (block height).
4. **Private recall.** The client finds the user's memories on Arweave by owner,
   reads them through gateways with failover, decrypts them locally and searches.
5. **Public recall.** An operator runs semantic search over public memories.
6. **Sharing.** A sealed grant signed by the owner's client lets one recipient key
   open one memory.

## Storage

- Arweave is the single record. Uploads go through ArDrive Turbo today
  (`https://upload.ardrive.io/v1/tx/solana`).
- The protocol must not depend on Turbo alone. A fallback (another ANS-104 bundler or
  direct Arweave writes) is open.
- A memory is permanent when an Arweave gateway reports it in a block.
- Reads try `ARWEAVE_GATEWAY_URL`, then the fallbacks (default `arweave.net`,
  `ar-io.dev`, `turbo-gateway.com`).

## Payments and quota

| Tier | Who pays | Mechanism |
| --- | --- | --- |
| Free | Nobody | Turbo free tier per signing wallet: items up to 105 KiB, 10 MiB lifetime per wallet and per IP |
| Operator-funded | User pays the operator per write | x402 (USDC); the operator grants a Turbo credit share approval; the upload sends `x-paid-by: <operator address>` |
| Self-funded | User | The user buys Turbo credits for their own address |

The quota formula and the price per write are open. The protocol fee needs an economic study.

## Discovery

- **Operator registry:** endpoint, operator key, capabilities, prices, stake. The SDK
  ranks operators by availability first.
- **Agent registry:** agent key, agent card (A2A), operators, owner. Registration is
  opt-in for agents that want to be found; entries are public; a small stake applies;
  one user can own any number of agents.
- Both use ERC-8004 registries, starting on Arc.
- Users pin trusted operators. The registry suggests; it never replaces a pin.
- Operators find and talk to each other through the registry.

## Staking and judgment

- Operators stake a protocol token to be listed. Listing also needs proven uptime and
  declared capabilities.
- A separate judges role receives proofs of faults that signatures can prove, and
  decides slashing. Other problems go into reputation.
- Minimum stake (about 1000 USD equivalent proposed), unbonding, slashed funds and
  token design are open.

## Agent to agent

- Messages follow the A2A standard. Mnemonic adds signatures and storage only.
- Each agent signs its own messages; private messages are encrypted to the recipient.
- Operators relay messages but cannot read or change them.
- The agent or user pays for relay.
- Escrow and payment guarantees belong to the separate Warrant protocol.

## Identity, version 1

- One key per user.
- Optional recovery phrase, encouraged with reminders, never blocking.
- One-step transfer of the key to another browser or device by QR code or link.
- Private memories are encrypted to the user's key, so every device with the key reads them.
- Device keys, revocation and passkey backup come later, only when needed.
- A lost device loses nothing that was uploaded: memories live on Arweave.

## Current state (2026-10-10)

- mcp2.mnemonik.xyz runs the Turbo image (sha-faf7f17). A live memory reached Arweave
  block 2,018,533 about 9 minutes after signing.
- The envelope is still signed by the operator; author signing is the next step.
- mcp.mnemonik.xyz (production) still runs an older image; its deploy depends on Tailscale,
  which is being removed.
