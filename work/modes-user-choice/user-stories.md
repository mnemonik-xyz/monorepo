---
created: 2026-06-01
status: draft  # browser "how" has 2 open forks — see end of file
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
  infrastructure** (see Fork B: a remote operator does the anchoring, I just sign +
  pay per artifact).
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

## Open forks the browser "no-infrastructure" want creates

The wants above are firm. Two *mechanism* questions remain — both materially shape
the browser build and one of them touches a prior decision:

### Fork A — How does "reusable across chats/providers" work?

- **A1 · Context injection (PAM-style).** Store structured memory blocks locally;
  inject the relevant/pinned ones into each new chat across providers. **No embedder,
  no server — fully infra-free.** Selection is heuristic (recency / pinned / keyword),
  not semantic. Fits the existing "no in-browser embedder" decision.
- **A2 · Local semantic recall.** Bundle a small ONNX embedder in the extension →
  embed + cosine-search locally. True semantic recall, still infra-free, but ~22 MB
  model + WASM compute, and it **reverses the "no in-browser embedder" decision**.
- **A3 · Remote semantic recall.** Send the query to a hosted embed/recall service.
  Light client, but a **network dependency** (and query/memory leaves the device) —
  in tension with "no infrastructure."

### Fork B — How does a zero-infra browser user `participate` (publish)?

Anchoring inherently needs network (Arweave + Solana + payment), so it can't be
*purely* local. "No infrastructure" = no server *I run*, not "no network ever."

- **B1 · Remote operator over HTTPS + x402.** I sign locally; a hosted operator
  anchors for a per-artifact fee (reuses our `payment_mode = x402`). **No install,
  no daemon** — just network + pay-per-publish. Most consistent with the persona.
- **B2 · Direct-to-chain from the browser.** Extension submits to Arweave/Solana via
  public gateways using my own funded wallet. Maximally decentralized, but I must
  fund/manage a wallet — arguably its own infrastructure + friction.
- **B3 · Bridge-only participate.** Pure-browser users can't publish; participate
  needs the optional local bridge. Keeps the browser standalone for local-only use.
