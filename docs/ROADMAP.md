# Roadmap

> Current direction as of July 2026. This file is the source of truth for the
> public [/roadmap](https://mnemonik.xyz/roadmap) page and is bundled into the
> webapp when it is built.

Mnemonic's immediate goal is not to add more protocol surface. It is to make
the existing non-custodial memory flow reliable and understandable in every
client: the client owns the signing key, local memory is free, and durable
anchoring is an explicit paid operation.

## Shipped

### Protocol and server foundation

- Canonical CBOR artifacts, BLAKE3 content hashes, Ed25519 identities, and
  COSE_Sign1 signatures.
- Semantic recall backed by full-precision embeddings in SQLite, with portable
  TurboQuant-compressed embeddings stored in the artifact.
- Per-request write intent: `local` keeps a memory on the node; `participate`
  uploads the signed data item through Irys and writes its content commitment
  to a Solana SPL Memo.
- Public and private memories, recall, verification, recovery of externally
  stored public artifacts, OAuth 2.1 + PKCE, and the hosted HTTP MCP endpoint.
- Non-custodial x402 payment verification for paid `participate` writes.

### Client foundation

- Webapp identity management and browser-mediated approval/signing.
- TypeScript SDK and CLI with local key ownership and deferred COSE signing.
- Install flows for Cursor, VS Code, Claude Desktop, and Windsurf.
- A functional Manifest V3 browser extension codebase with local capture,
  recall, identity, cloud-sync, and release assets. It is not yet published in
  the Chrome Web Store.

## In progress

The following four workstreams are the current development vector. They are
ordered: signing ownership is the invariant; client reliability and payment
build on it; documentation must describe the resulting product exactly.

### 1. Make client-side signing the only user-artifact path

**Decision:** the user or agent client signs every user-owned artifact. The MCP
operator may prepare canonical bytes, store them, relay them, pay network fees,
and verify the result, but it must not impersonate the artifact author. The
operator key remains valid only for operator-owned service artifacts such as
seed content or service-authored posts.

Remaining work:

- Consolidate the browser approval flow and programmatic SDK/CLI flow around
  one canonical client-signing contract.
- Expose `mode`, visibility, and payment intent consistently in the SDK, CLI,
  browser extension, and IDE instructions.
- Remove server-signing language and ambiguous `arweave_tx` terminology from
  public documentation and responses. Current external data receipts are Irys
  ANS-104 data-item IDs; their gateway availability must not be presented as
  proven Arweave L1 permanence.
- Add cross-client conformance tests proving that identical canonical bytes are
  signed by the client identity and rejected when the JWT subject, signer, or
  callback payload does not match.

This workstream is complete when no hosted user-memory path can produce a valid
artifact signature without the user's client key.

### 2. Make every client surface dependable

**IDE integrations:** Cursor, VS Code, Claude Desktop, and Windsurf installation,
OAuth, tool discovery, browser handoff, signing approval, recall, and verification
must pass a versioned release matrix. CI covers deterministic protocol and
deeplink behavior; a pre-release smoke run covers the real desktop applications
and production OAuth callbacks.

**CLI and SDK:** add explicit local-versus-anchored write selection, x402
challenge handling, actionable identity-mismatch errors, and end-to-end examples
that match the current API. A CLI command must never claim a memory is anchored
unless external storage and the Solana memo are both confirmed.

**Browser extension:** the core implementation exists, but publication requires
the release gates below:

- Pass the complete Playwright suite against the packaged extension and a live
  staging server, including Chrome profile restore and cloud signing.
- Publish the extension privacy page at `/extension/privacy` and ensure the
  manifest, store listing, and actual data handling agree.
- Finish Google OAuth verification and configure the production extension ID
  and redirect URI.
- Run adapter smoke tests against the current ChatGPT, Claude, and Gemini DOMs.
- Produce the signed release zip/source archive, complete the Chrome Web Store
  publisher/listing forms, upload final screenshots, and submit for review.
- Replace the stale extension README with an accurate operator and contributor
  guide before release.

### 3. Build native plugins for major IDEs and coding agents

Create first-class Mnemonic plugins for the environments where developers and
agents already work, beginning with Claude Code, Codex, Cursor, VS Code/Copilot,
and Windsurf. MCP connectivity remains the common protocol surface; each plugin
adds host-specific lifecycle integration and commands.

The primary plugin capability is durable conversation checkpointing:

- When a host exposes a context-compaction or context-reset hook, capture the
  conversation chunk that is about to leave the active context and save it as a
  client-signed memory.
- Provide an explicit agent/user command for hosts without a reliable hook and
  for moments when the user wants to checkpoint context manually.
- Preserve chunk order and continuity with session identity, sequence numbers,
  timestamps, and a hash link to the previous chunk so a conversation can be
  reconstructed and verified.
- Default automatic checkpoints to private/local storage. On-chain anchoring,
  public visibility, and any paid operation require an explicit user choice;
  background compaction must never trigger an unexpected payment.
- Redact secrets and respect per-workspace exclusions before signing. Show what
  was captured, make repeated hooks idempotent, and provide clear pause/delete
  controls.
- Add conformance fixtures so every plugin produces the same canonical artifact
  for the same conversation chunk, independent of host-specific event formats.

The first release is complete when at least one hook-capable coding agent saves
automatically before compaction, every supported host offers the explicit save
command, and a fresh session can recall the ordered checkpoints under the same
client identity.

### 4. Make anchoring x402-first

Move the hosted service from operator-subsidized anchoring to an explicit paid
path without making ordinary memory capture fragile or surprising. The server
continues to relay the Irys upload and Solana Memo from its funded operator
wallet; the client pays the quoted anchoring service cost in USDC through x402.

[Universal Paywall](https://mnemonik-dev.github.io/universal-paywall-site/) is
the candidate payment rail. The integration follows the
[frictionless payment user spec](../work/universal-paywall-integration/user-spec.md)
and [technical spec](../work/universal-paywall-integration/tech-spec.md): a
standard one-time x402 payment is the default, while a capped and expiring
allowance is an optional convenience for people who anchor repeatedly.

Product rules:

- `local` writes are free and never invoke a payment gate.
- `participate` is always an explicit user choice. The client shows the exact
  price, settlement network, public/private visibility, and operation being
  purchased before requesting wallet approval.
- Payment authorizes storage and anchoring work only. It never authorizes the
  MCP server to sign on the client's behalf.
- One-time x402 must remain available without creating a vault, deposit, or
  recurring spending permission. After wallet connection, the target is one
  payment approval for one anchor.
- A recurring allowance is opt-in, restricted to the Mnemonic payee, capped,
  expiring, visible, and revocable. It reduces future wallet prompts but never
  removes the explicit **Anchor on-chain** action.
- Automatic capture, context-compaction checkpoints, background sync, and
  retries must never create a new payment without an explicit user action.

Canonical paid journey:

1. The user selects **Anchor on-chain**; free local saving remains the default.
2. The client fetches a fresh quote and prepares the canonical artifact.
3. The user previews the content, visibility, price, and network, then signs the
   artifact locally.
4. MCP returns a quote bound to the user identity, content hash, correlation ID,
   amount, network, recipient, and expiry. The quote offers one-time x402 first
   and, when available, the user's existing allowance.
5. For one-time x402, the wallet approves exactly this operation. With an active
   allowance, the payment rail atomically reserves the quoted amount after the
   user confirms in Mnemonic; no wallet prompt is required.
6. MCP uploads the client-signed bytes to Irys, writes the Solana Memo, refetches
   and verifies delivery, then commits an allowance reservation if applicable
   and returns a complete receipt.
7. If delivery fails after payment or reservation, the same payment state
   resumes the same operation;
   the client must not ask for a second payment. Recovery status remains visible
   until the write succeeds or is explicitly abandoned.

Every surface must expose the same resumable states:
`awaiting_signature` → `awaiting_payment` → `payment_confirming` → `anchoring`
→ `verifying_delivery` → `anchored`. The webapp provides the approval/payment
page used by IDE handoffs; CLI and SDK expose explicit `participate` selection
and x402 retry handling; the browser extension keeps automatic capture local
and offers anchoring as a separate action.

Rollout order:

- Bind quotes and payment proofs to one idempotent anchoring operation and make
  failure/retry semantics honest: a transferred payment is reusable for that
  operation, not refunded merely because its nonce was not consumed.
- Productize Universal Paywall's standard one-time x402 path first. It is the
  minimum-friction and compatibility path and must not depend on StakeVault.
- Add the recurring allowance only after Universal Paywall has durable atomic
  reservations, funded-headroom accounting, operation-bound typed proofs,
  payee-restricted policies, signed receipts, and restart reconciliation.
- Finish SDK, CLI, webapp, IDE handoff, and extension support before changing
  production payment configuration.
- Run real-wallet staging tests for approval rejection, insufficient funds,
  expired quotes, duplicate callbacks, RPC failure, delivery failure, and retry.
- Enable x402 in production only after old clients receive a clear upgrade path
  instead of an unexplained HTTP 402 failure.

### 5. Make onboarding and how-to documentation executable

- Keep a tested CLI installation, login, local sign, recall, and verification
  walkthrough in the repository README.
- Put the same CLI workflow on the webapp Install page next to the IDE-specific
  instructions.
- Add separate, copyable examples for a free local write and an anchored
  `participate` write. Document x402 behavior when the corresponding client
  support is released.
- Test commands, URLs, tool names, expected response fields, and gateway links
  in CI or release smoke checks.
- Treat `docs/ROADMAP.md` as the roadmap source of truth. Publishing a roadmap
  change requires rebuilding and deploying the webapp because the Markdown is
  bundled at build time.

## Planned

These items follow the stabilization work above; they must not displace it:

- Move toward **verifiable agent computations**: client-signed, content-addressed
  evidence for agent steps, tool inputs and outputs, execution results,
  evaluator verdicts, and complete trajectories. The goal is to let another
  party verify what an agent executed and how a result was derived, not merely
  verify that a memory was stored. This begins only after the personal-use
  client surfaces—webapp, CLI, IDE integrations, and browser extension—are
  polished and dependable. It will build on the existing experimental step,
  verdict, and trajectory schemas and requires explicit privacy, redaction,
  replay, and cross-runtime conformance rules before promotion.
- Publish the Chrome extension and establish a regular adapter-update cadence.
- Add A2A and Hermes bridges using the same client-signed artifact contract.
- Register Mnemonic verification with ERC-8004 once the verification contract
  and identity mapping are specified.
- Expand portable memory composition, capability-scoped sharing, and safe
  rehydration only after their protocol specifications have executable
  conformance tests.
- Evaluate additional anchoring and x402 settlement networks without weakening
  the storage-independent artifact format.

Feature-level specifications and acceptance evidence live under
[`work/`](https://github.com/mnemonik-xyz/monorepo/tree/main/work). A roadmap item
counts as shipped only when its user-facing path is deployed, documented, and
covered by its release checks.
