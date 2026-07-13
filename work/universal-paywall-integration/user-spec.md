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

A person should be able to anchor a client-signed memory without understanding
x402, vaults, facilitators, token approvals, or settlement. The default paid
journey is a one-time payment for one anchor. People who anchor repeatedly may
explicitly enable a capped, expiring allowance to avoid repeated wallet prompts.

Payment is part of the `participate` journey, not a separate product. The
client remains the artifact signer. Mnemonic uses the payment only to fund the
storage and anchoring operation.

## Product invariants

- `local` memory remains free, private, and the default.
- A paid operation starts only after the user explicitly chooses **Anchor
  on-chain**.
- One-time x402 is always available and is the default on first use.
- A recurring allowance is optional. It is never created merely because the
  user connected a wallet or installed a plugin.
- Automatic capture, context compaction, background synchronization, polling,
  and retry cannot create a new paid operation.
- Every price is shown before approval. The price cannot increase after the
  wallet has approved it.
- One user action creates at most one charge. Retrying or reopening the browser
  must not charge again.
- Payment never authorizes Mnemonic to sign an artifact for the user.
- A payment or allowance can be inspected and independently reconciled with an
  operation receipt.

## Payment choices

### Pay once — default

The user pays the displayed amount for exactly one identified anchoring
operation. No stake vault, deposit, recurring permission, or future spending
authority is required.

After the wallet is connected, the target experience is one wallet approval.
The payment surface may perform network selection automatically when safe, but
must explain any required network switch before opening the wallet.

### Set a spending limit — optional

The user may authorize future Mnemonic anchors up to a maximum USDC amount and
until a visible expiry. This option is described in ordinary language, for
example: **Avoid wallet prompts — allow up to 10 anchors / 10 USDC until 20
July**.

The user sees:

- maximum total spend;
- price of the current anchor;
- estimated number of anchors covered at the current price;
- expiry;
- recipient, displayed as **Mnemonic anchoring service**;
- remaining allowance; and
- a revoke action.

Creating or increasing an allowance always requires explicit wallet approval.
An active allowance removes the per-anchor wallet prompt, but not the explicit
**Anchor on-chain** action.

## Canonical journeys

### Journey A — first one-time anchor

1. The user selects **Anchor on-chain** in the webapp, CLI, extension, or IDE.
2. The approval page shows the memory preview, public/private visibility, total
   price, and a short explanation of what will be stored.
3. The client signs the canonical artifact locally.
4. **Pay once** is selected by default. The user selects **Pay and anchor**.
5. The wallet requests one payment authorization after connection.
6. The page shows a single continuous progress flow: **Payment confirmed →
   Storing → Anchoring → Verifying**.
7. The page returns a receipt with content hash, client signer, amount, network,
   payment reference, Arweave/Irys ID, Solana transaction, and verification time.
8. After success, the UI may unobtrusively offer **Avoid wallet prompts next
   time**. It must not create an allowance automatically.

### Journey B — enable an allowance

1. The user chooses **Avoid wallet prompts** from a successful receipt or
   payment settings.
2. The UI recommends a small preset expressed as both money and approximate
   anchors. The user can choose another cap or expiry.
3. One confirmation screen summarizes cap, expiry, network, and recipient.
4. The wallet approves the allowance setup.
5. The UI confirms the remaining allowance and provides a revoke action.

Any unavoidable create-vault, token-approval, deposit, and policy transactions
must be composed or guided as one setup flow. The UI must never present those
protocol operations as unexplained, unrelated requests.

### Journey C — anchor with an active allowance

1. The user explicitly selects **Anchor on-chain**.
2. The UI shows the exact current charge and remaining allowance.
3. The user confirms in the Mnemonic surface; no wallet prompt is required.
4. The operation anchors and produces the same receipt as a one-time payment.

If the allowance is insufficient or expired, the user is offered **Pay once**
first and **Update spending limit** second. The memory remains saved locally
while the decision is pending.

### Journey D — IDE and coding-agent handoff

1. The IDE command opens one secure Mnemonic approval URL.
2. The page performs preview, client signing, payment choice, wallet approval,
   progress, and receipt without copy/pasting tokens or transaction hashes.
3. The IDE polls the same correlation ID and updates automatically when the page
   completes.
4. Closing either surface does not lose the operation. Reopening resumes it.

### Journey E — failure and recovery

- Wallet rejection leaves the signed artifact local and unpaid.
- Insufficient funds offers a clear recovery action without rebuilding the
  artifact.
- An expired quote refreshes before requesting approval.
- Failure after a one-time payment shows **Payment received — retrying anchor**,
  never another payment request.
- Failure after an allowance reservation keeps or releases the same reservation
  according to its state; it never creates a second reservation silently.
- A permanently abandoned paid operation exposes its refund or service-credit
  status in the same receipt.

## Information hierarchy

The primary approval screen shows only:

1. what is being anchored;
2. visibility;
3. total price;
4. **Pay once** or the active allowance; and
5. the primary action.

Network identifiers, token addresses, vault addresses, facilitator addresses,
quote hashes, and raw payment proofs belong under **Payment details**. They must
remain available for verification without dominating the journey.

## Surface requirements

### Webapp

- Owns the canonical approval and recovery page used by browser and IDE flows.
- Supports wallet connect, one-time payment, allowance setup/revoke, progress,
  and receipt history.
- Works on mobile-sized screens and supports keyboard navigation.

### CLI and SDK

- Expose typed quote, payment-choice, approval, status, and receipt objects.
- Interactive CLI defaults to one-time payment and asks before opening a wallet
  handoff.
- Non-interactive payment or use of an allowance requires an explicit option;
  absence of that option never falls back to an automatic charge.

### Browser extension and IDE plugins

- Automatic capture stays local and free.
- Anchoring is a separate command.
- The extension/plugin delegates wallet interaction to the canonical webapp
  rather than implementing divergent payment behavior.

## Success metrics

- A connected-wallet user can complete a first one-time anchor with one wallet
  payment approval.
- A user with an active allowance completes an anchor without a wallet prompt,
  after one explicit Mnemonic confirmation.
- No flow asks the user to copy a transaction signature, payment proof, or
  correlation ID.
- Closing and reopening the approval page or IDE resumes the same operation.
- A retry never produces a duplicate charge.
- Usability testing confirms that users can explain the difference between
  **Pay once** and **Set a spending limit** without protocol terminology.

## Out of scope

- Charging for local storage, recall, or verification.
- Subscriptions or unlimited allowances.
- Custodial balances or developer API-key credits.
- Direct client submission of the Irys upload or Solana Memo transaction.
- Silent payment by context-compaction or background agents.

