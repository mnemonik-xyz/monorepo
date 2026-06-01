---
created: 2026-06-01
status: approved  # browser "how" forks resolved 2026-06-01 (A1 + B2)
type: user-stories
related:
  - work/modes-user-choice/user-journeys.md
  - work/modes-user-choice/user-spec.md
---

# User stories — per client surface

The "I want / so that" wants that the journeys (`user-journeys.md`) realize.
Surfaces: CLI, Node-SDK, IDE-hosted agent, browser extension.

## Browser extension — the standalone-first persona (lead)

> Reframes the earlier "bridge, else local" framing: for the browser user the
> **standalone, infrastructure-free path is PRIMARY**, not a fallback. The local
> bridge is demoted to an optional power-user enhancement (adopted decision).

As a **browser-extension user**, I want:

- **…not to depend on any infrastructure** — no local server to install, no daemon
  to keep running, no native-messaging host, no account — *so that* the extension
  works the moment I add it to Chrome, on any machine, including locked-down ones.
- **…my memories and context to be reusable across different chats and providers**
  (ChatGPT, Claude, Gemini, Copilot) — *so that* what I taught one assistant carries
  into the next, regardless of vendor; my memory is mine, not siloed per provider.
- **…everything stored locally on my device by default** — `chrome.storage.local`,
  signed with my own key — *so that* nothing leaves my machine unless I explicitly
  choose to publish it.
- **…the same identity as my other surfaces** — *so that* if I *do* later install
  the CLI/IDE, my browser memories are recognizably mine (one keypair everywhere).
- **…(optionally) to publish a memory as a public, verifiable record** — *so that*
  other agents/people can verify it — **without that requiring me to run any
  infrastructure**: I sign locally and the extension anchors **directly to
  Arweave/Solana from my own wallet** (Fork B2), with no hosted operator in the path.
- **…to carry my memory between machines/browsers** via export/import of the signed
  artifact — *so that* "no infrastructure" never means "trapped on one device."

**Non-negotiables for this persona:** zero install for the core value; cross-provider
reuse; local-by-default privacy; no hard dependency on a local server *or* a remote
service for everyday (local) use.

## CLI (`mnemonic …`)

As a **developer at a terminal**, I want:
- explicit, scriptable control of each write's mode (`--mode participate`) — *so
  that* publishing is always a deliberate, auditable act;
- the canonical local DB on my filesystem — *so that* my CLI, SDK apps, and IDE
  agents all share one memory with no sync;
- free, offline local memory by default — *so that* personal notes never cost money.

## Node-SDK (`@mnemonic/sdk`)

As a **developer embedding memory in my app/agent**, I want:
- a programmatic `sign/recall/verify` API with mode at the call site — *so that* my
  app decides local vs participate from its own logic;
- capability discovery (`m.capabilities()`) before I expose a "publish" button — *so
  that* I never offer participate against an operator that can't anchor;
- the payment handshake (x402 / balance) handled for me — *so that* I don't
  reimplement the paywall.

## IDE-hosted agent (Cursor / Claude Desktop via MCP)

As a **user of an AI coding agent**, I want:
- the agent to persist my working context as free `local` memory automatically — *so
  that* it remembers across sessions without spending my money;
- the agent to **never** publish or spend autonomously — *so that* a paid, public,
  irreversible `participate` only happens after I explicitly approve it;
- cross-session, cross-tool recall — *so that* what the agent learned yesterday (or
  what I wrote on the CLI) is available today.

---

## Resolved forks (browser "no-infrastructure", 2026-06-01, user)

### Fork A — cross-chat/provider reuse → **A1 · Context injection (PAM-style)** ✅

Store structured memory blocks locally; inject the relevant/pinned ones into each
new chat across providers. **No embedder, no server, no semantic recall in-browser
— fully infra-free.** Selection is heuristic (recency / pinned / keyword). **Keeps
the "no in-browser embedder" decision** (which the standalone-first reframing had
reopened). *Rejected:* A2 local-embedder (reverses that decision, +~22 MB), A3
remote recall (network dependency, data leaves device — breaks "no infra").

### Fork B — zero-infra publish → **B2 · Direct-to-chain from the browser** ✅

The extension anchors **directly** to Arweave + Solana via public gateways/RPC using
the **user's own funded wallet** — no hosted operator, no daemon, maximally
decentralized. "No infrastructure" taken to its strongest reading: not even a
third-party operator in the path. *Rejected:* B1 remote-operator-x402 (introduces a
hosted-operator dependency), B3 bridge-only (denies pure-browser users publishing).

**Implications for the (separate) browser build:**
- **Payment ≠ the server-side x402/balance model.** The browser user pays **chain +
  storage fees directly** from their wallet; there is no per-artifact fee to a
  Mnemonic operator. `payment_mode` does not govern browser participate.
- **Wallet management is net-new browser scope:** the Ed25519 *identity* key (signs
  the COSE artifact) is distinct from the **funding wallet(s)** — an Arweave wallet
  (or a bundler like Irys/Turbo that accepts SOL) + a SOL-funded Solana fee-payer.
  Likely "connect your wallet" (e.g. Phantom) rather than a hot key in the extension.
- **Delivery guarantee still holds** via read-back + verify (fetch the anchored COSE
  bytes, re-check blake3 + Ed25519 against the Solana anchor) — this does **not**
  need semantic recall, so it is compatible with A1.
- **Bridge stays optional.** A power user who also runs the CLI/IDE may still bridge
  to the canonical DB; it is never required for local use or for publishing.
