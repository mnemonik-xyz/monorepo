---
created: 2026-07-13
status: draft
type: feature
size: L
priority: P1
related:
  - "GitHub issue #203 — end-to-end x402 paid anchoring journey"
  - "https://github.com/mnemonik-dev/universal-paywall"
  - "docs/ROADMAP.md — Make anchoring x402-first"
---

# User Spec: Frictionless payments for anchored memories

## Goal

A person should be able to anchor client-signed memories throughout a real work
session without understanding x402, vaults, facilitators, token approvals, or
settlement. The primary paid journey is a capped, expiring **seamless anchoring
session** approved once in the wallet. One-time payment remains available as an
optional trial and fallback.

Payment is part of the `participate` journey, not a separate product. The
client remains the artifact signer. Mnemonic uses the payment only to fund the
storage and anchoring operation.

## Product invariants

- `local` memory remains free, private, and the default.
- Paid anchoring starts only after the user explicitly starts a paid session or
  chooses **Pay once** for one operation.
- A paid session is restricted by total cap, per-anchor price ceiling, expiry,
  recipient, workspace, visibility, and allowed checkpoint types.
- One-time x402 remains available as an optional alternative and never creates
  future spending authority.
- Connecting a wallet or installing a plugin never starts or renews a session.
- Automatic capture outside an active paid session remains local and free.
  Polling and retry never create a new charge.
- Every price is shown before approval. The price cannot increase after the
  wallet has approved it.
- One user action creates at most one charge. Retrying or reopening the browser
  must not charge again.
- Payment never authorizes Mnemonic to sign an artifact for the user.
- A payment or allowance can be inspected and independently reconciled with an
  operation receipt.

## Payment choices

### Start a seamless anchoring session — primary

The user may authorize future Mnemonic anchors up to a maximum USDC amount and
until a visible expiry. The session may also authorize selected automatic
checkpoint types, such as `pre_compaction` or `session_end`, for one workspace.
This is described in ordinary language, for example: **Seamless anchoring for
this workspace — up to 5 USDC for 7 days, never more than 0.05 USDC per
checkpoint**.

The user sees:

- maximum total spend;
- price of the current anchor;
- maximum price per anchor;
- estimated number of anchors covered at the current price;
- expiry;
- recipient, displayed as **Mnemonic anchoring service**;
- workspace, visibility, and allowed checkpoint types;
- remaining allowance; and
- a revoke action.

Creating or increasing an allowance always requires explicit wallet approval.
An active paid session removes per-anchor wallet prompts. It may perform an
automatic paid checkpoint only for a checkpoint type and workspace that the
user explicitly authorized when starting the session.

### Pay once — optional

The user pays the displayed amount for exactly one identified anchoring
operation. No stake vault, deposit, recurring permission, or future spending
authority is required. This remains available for trial use, unsupported
wallets, insufficient session allowance, or users who do not want a session.

## Canonical journeys

### Journey A — start seamless anchoring

1. The user chooses **Start seamless anchoring** in the webapp, extension, CLI,
   or IDE handoff.
2. The UI recommends a small preset expressed as money, approximate anchors,
   expiry, per-anchor ceiling, workspace, visibility, and allowed checkpoint
   types. The user can change each value.
3. One confirmation screen summarizes the complete authority.
4. The wallet approves the allowance setup once.
5. The UI confirms the active session, remaining allowance, and stop/revoke
   action.

Any unavoidable create-vault, token-approval, deposit, and policy transactions
must be composed or guided as one setup flow. The UI must never present those
protocol operations as unexplained, unrelated requests.

### Journey B — automatic checkpoint in an active session

1. An allowed hook, such as pre-compaction, fires in the authorized workspace.
2. The plugin applies exclusions and secret redaction, then the client signs the
   canonical checkpoint artifact locally.
3. If the price is within the per-anchor ceiling and remaining cap, Universal
   Paywall synchronously settles that charge and returns a durable receipt.
4. MCP anchors and verifies the artifact, then associates delivery with the same
   receipt.
5. If any session constraint fails, the checkpoint stays local and the user is
   notified; no payment occurs.

### Journey C — manual anchor in an active session

1. The user explicitly selects **Anchor on-chain**.
2. The UI shows the exact current charge and remaining allowance.
3. The user confirms in the Mnemonic surface; no wallet prompt is required. If
   manual anchors were included in the session scope, payment settles
   synchronously from the allowance.
4. The operation anchors and produces the same receipt as a one-time payment.

If the allowance is insufficient or expired, the user is offered **Update
session** first and optional **Pay once** second. The memory remains saved
locally while the decision is pending.

### Journey D — optional pay once

1. The user chooses **Pay once** from payment choices.
2. The page shows the artifact preview, visibility, total price, and recipient.
3. The client signs locally and the wallet approves payment for this operation.
4. MCP anchors, verifies, and returns the same combined receipt shape.

### Journey E — IDE and coding-agent handoff

1. The IDE command opens one secure Mnemonic approval URL.
2. The page performs preview, client signing, payment choice, wallet approval,
   progress, and receipt without copy/pasting tokens or transaction hashes.
3. The IDE polls the same correlation ID and updates automatically when the page
   completes.
4. Closing either surface does not lose the operation. Reopening resumes it.

### Journey F — failure and recovery

- Wallet rejection leaves the signed artifact local and unpaid.
- Insufficient funds offers a clear recovery action without rebuilding the
  artifact.
- An expired quote refreshes before requesting approval.
- Failure after a one-time payment shows **Payment received — retrying anchor**,
  never another payment request.
- Failure after a session charge shows **Payment received — retrying anchor**
  and reuses the same operation receipt; it never settles another charge.
- A permanently failed delivery keeps its settled receipt and operation ID for
  support reconciliation. Automated refunds/service credits are deferred until
  the product has enough failure data to define that policy safely.

## Information hierarchy

The primary approval screen shows only:

1. what is being anchored;
2. visibility;
3. total price;
4. the recommended paid session, active session, or optional **Pay once**; and
5. the primary action.

Network identifiers, token addresses, vault addresses, facilitator addresses,
quote hashes, and raw payment proofs belong under **Payment details**. They must
remain available for verification without dominating the journey.

## Surface requirements

### Webapp

- Owns the canonical approval and recovery page used by browser and IDE flows.
- Supports wallet connect, paid-session setup/revoke, optional one-time payment,
  progress, remaining allowance, and receipt history.
- Works on mobile-sized screens and supports keyboard navigation.

### CLI and SDK

- Expose typed quote, payment-choice, approval, status, and receipt objects.
- Interactive CLI recommends starting or using a bounded paid session and keeps
  **Pay once** as an explicit alternative.
- Non-interactive automatic anchoring requires an already active session whose
  scope covers that checkpoint. Absence of a session never falls back to a
  one-time charge.

### Browser extension and IDE plugins

- Automatic capture stays local and free unless an explicitly started paid
  session authorizes that checkpoint type and workspace.
- Outside a paid session, anchoring remains a separate command.
- The extension/plugin delegates wallet interaction to the canonical webapp
  rather than implementing divergent payment behavior.

## Success metrics

- A connected-wallet user can start a bounded paid session through one coherent
  wallet setup flow.
- A user with an active session completes authorized manual and automatic
  checkpoints without repeated wallet prompts.
- No flow asks the user to copy a transaction signature, payment proof, or
  correlation ID.
- Closing and reopening the approval page or IDE resumes the same operation.
- A retry never produces a duplicate charge.
- Usability testing confirms that users understand the session cap, expiry,
  per-anchor ceiling, allowed checkpoint types, and optional **Pay once**.

## Out of scope

- Charging for local storage, recall, or verification.
- Subscriptions or unlimited allowances.
- Custodial balances or developer API-key credits.
- Direct client submission of the Irys upload or Solana Memo transaction.
- Paid background activity outside an explicitly started, scoped session.
