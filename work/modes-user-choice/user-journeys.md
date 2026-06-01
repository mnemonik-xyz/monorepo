---
created: 2026-06-01
status: draft
type: user-journeys
related:
  - work/modes-user-choice/user-spec.md
  - work/modes-user-choice/tech-spec.md
  - work/modes-user-choice/decisions.md
---

# User Journeys — Mode-as-a-user-choice, per client surface

How the **local vs participate** intent is expressed, and the full write → recall →
(optionally) participate → verify loop is lived, on each of the four surfaces:
**CLI**, **Node-SDK**, **IDE-hosted agent**, **browser extension**. Grounded in the
finalized decisions (see decisions.md). Journeys here center on *this* feature (the
mode choice + paid delivery), with just enough surrounding flow to make sense.

## Cross-cutting invariants (true on every surface)

- **One identity everywhere.** The same Ed25519 keypair signs on every surface
  (invisible-identity). A memory written on the CLI verifies as "yours" in the
  browser. No per-surface accounts.
- **One canonical DB on native surfaces.** CLI, Node-SDK, and IDE agents share the
  server-owned `~/.mnemonic/attestations.db`. A write on one is instantly recallable
  on the others. The browser shares it too **when bridged**.
- **Mode is a per-write intent, default `local`.** Nobody is ever *in* a paid mode;
  each write individually is either `local` (free, private, offline) or
  `participate` (anchored, public, paid, delivery-confirmed).
- **`local` is free and silent; `participate` is paid and requires consent.** The
  paid path always has an explicit authorization moment — the shape of that moment
  is the main thing that differs across surfaces.
- **`participate` only returns success after delivery is confirmed** (anchored AND
  verified by recall). On failure the write stays `local` and is never charged — so
  no surface can ever leave the user believing a still-local artifact was published.

## Journey legend

Each surface is described by five journeys:

- **J0 — Onboarding** (first run: identity + reachability)
- **J1 — Remember (local)**: the free default loop, write → recall
- **J2 — Participate (paid)**: publish + the consent moment + delivery confirmation
- **J3 — Verify**: prove an artifact (yours or a third party's) is real & delivered
- **J4 — Continuity**: how this surface sees memories written on the others

---

## Surface 1 — CLI (`mnemonic …`)

**Actor:** a human developer at a terminal. Most explicit, most deliberate surface.

**J0 · Onboarding.** First `mnemonic` invocation generates the Ed25519 identity into
the OS keychain and creates `~/.mnemonic/attestations.db`. `mnemonic whoami` prints
the pubkey + the server's mode envelope (`supported_modes`, `default_mode`,
`participate_cost`). No funded keypair needed for local use.

**J1 · Remember (local).** `mnemonic sign "User prefers async Python"` → defaults to
`local`: embed → compress → canonical CBOR → blake3 → COSE-sign → SQLite, synthetic
`local:` tx id, **zero cost, works offline**. `mnemonic recall "python style"`
returns it via cosine search. This is the everyday loop; the user never thinks about
"mode."

**J2 · Participate (paid).** The user *opts in explicitly*:
`mnemonic sign --mode participate "Audit X passed: <hash>"`. The CLI:
1. checks the envelope — if the configured server can't anchor, it **errors loudly**
   ("this server only supports local; participate needs an anchoring operator"),
   nothing written, nothing charged.
2. shows the cost and the public-publish warning ("this becomes a public,
   immutable, verifiable record — anyone with the tx id can read it"), and waits for
   confirmation (or `--yes` to skip the prompt in scripts).
3. anchors on Arweave + Solana, then runs the **recall+verify round-trip** against
   the anchored artifact. Only on success: prints `participated` + the
   `delivery_receipt {arweave_tx, solana_tx, recall_verified_at}` and charges.
   On failure: prints `delivery: failed`, leaves the artifact `local`, no charge.

**J3 · Verify.** `mnemonic verify <tx|hash>` re-fetches the anchored COSE bytes,
re-checks blake3 + Ed25519 against the Solana anchor, and surfaces
`recall_verified_at` — so a third party (not just the author) can confirm the bytes
are *retrievable*, not merely committed.

**J4 · Continuity.** Because the CLI opens the canonical DB directly, it recalls
everything written by the SDK and IDE agents on the same machine, and (once
published) anyone's `participate` artifacts by tx id.

---

## Surface 2 — Node-SDK (`@mnemonic/sdk`)

**Actor:** a developer embedding verifiable memory into their own app/agent. The
mode is a *code-level* decision, set at the call site or threaded from app logic.

**J0 · Onboarding.** `const m = new Mnemonic()` loads/creates the shared identity and
opens the canonical DB (or connects to a running local `mnemonic-mcp`). The envelope
is exposed as `m.capabilities()` so the app can branch on `supportsParticipate`
before offering a "publish" feature.

**J1 · Remember (local).** `await m.sign("project · uses · FastAPI")` → `local` by
default; free, offline, returned with a `local:` id. `await m.recall("stack")` for
read-back. The app treats this like a local embedded store that happens to be signed.

**J2 · Participate (paid).** `await m.sign(text, { mode: "participate" })`. The SDK
mirrors the CLI guarantees programmatically:
- throws `UnsupportedModeError` if the operator can't anchor (never silently local);
- the **consent moment is the app's responsibility** — the SDK surfaces
  `m.quoteParticipate()` (cost) so the app shows its *own* confirmation UI before
  calling with `participate`; for `payment_mode = x402` the SDK handles the 402 +
  `X-Payment` retry, for `balance` it uses the bearer token.
- resolves only after the recall+verify round-trip; returns `{ mode:"participate",
  delivery_receipt }`. Rejects (no charge) on delivery failure.

**J3 · Verify.** `await m.verify(txOrHash)` → `{ valid, recall_verified_at,
delivery_receipt }`. Lets the app render a "verified & delivered" badge.

**J4 · Continuity.** Same canonical DB as CLI + IDE agents. An SDK-built app and the
developer's CLI see each other's local memories with no sync step.

---

## Surface 3 — IDE-hosted agent (Cursor / Claude Desktop via MCP stdio)

**Actor:** an **AI agent** calling the 5 MCP tools mid-task. The distinguishing
trait: the *agent*, not a human, picks the mode — by inferring intent. This is the
subtle surface, because a wrong call either leaks a private note publicly or spends
money without consent.

**J0 · Onboarding.** The user adds `mnemonic-mcp --transport stdio` to their MCP
config once. On connect the agent calls `mnemonic_whoami`, learns the envelope, and
learns it shares the same canonical DB as the user's CLI.

**J1 · Remember (local) — the default the agent reaches for.** During a session the
agent persists working context — "user is refactoring the auth module", "prefers
async Python" — via `mnemonic_sign_memory` with **no mode** (→ `local`). Free,
private, instant. The agent should default here for anything that is the user's own
context; it never costs the user money and never leaves the machine.

**J2 · Participate (paid) — gated by explicit user consent.** The agent proposes
`participate` only when intent is clearly *public/shareable* ("publish this audit
result so other agents can verify it"). Because it is **paid + irreversible +
public**, the agent must **not** call `participate` autonomously: it surfaces the
cost + the public-publish/immutability warning and asks the user to confirm, then
calls `mnemonic_sign_memory { mode: "participate" }`. The unsupported-mode error and
the deliver-or-don't-charge guarantee protect the user if the agent misjudges
capability. The returned `delivery_receipt` is what the agent reports back ("published
& verified, tx …").

**J3 · Verify.** The agent calls `mnemonic_verify` to check a hash another agent
handed it — turning "agent B claims X" into "X is signed by B's key and the bytes
are retrievable."

**J4 · Continuity.** The agent recalls memories the user wrote earlier on the CLI or
in another IDE session — cross-session, cross-tool memory is the whole point. Same
identity, same DB.

**Mode-choice guidance (belongs in the agent's tool description):** default `local`;
choose `participate` only for content meant to be a public, permanent, third-party-
verifiable record, and only after explicit user consent because it costs money and
cannot be retracted.

---

## Surface 4 — Browser extension (ChatGPT / Claude / Gemini / Copilot, MV3)

**Actor:** a human clicking in the extension UI over a web chat. **Standalone-first
persona** (see user-stories.md): the core value depends on **no infrastructure** —
no server, no daemon, no install. The bridge is an *optional* enhancement, not a
precondition.

**J0 · Onboarding — zero-install, works immediately.** Add the extension; it
generates/loads the Ed25519 identity (blake3-wasm + Ed25519-wasm) and creates a local
`chrome.storage.local` store of **structured, signed memory blocks** (episodic /
semantic / procedural / working / identity). No account, no server. *Optional:* a
power user who also runs the CLI/IDE can `mnemonic install-bridge` to share the
canonical DB — pure bonus, never required.

**J1 · Remember + reuse across chats/providers (local, free) — the core loop.**
"Save to memory" on a chat selection writes a signed block into the local store.
Reuse works by **context injection (Fork A1)**: when the user starts a new chat on
ChatGPT / Claude / Gemini / Copilot, the extension injects the relevant/pinned blocks
into the prompt — so the assistant carries prior context across vendors. Selection is
heuristic (recency / pinned / keyword), **not** semantic search — there is no
in-browser embedder by design. Everything stays on-device unless explicitly published.

**J2 · Participate (publish a public, verifiable record) — direct-to-chain (Fork
B2).** The user clicks **"Publish & verify"**; the extension signs locally and anchors
**directly to Arweave + Solana from the user's own connected wallet** (e.g. Phantom /
a bundler) via public gateways — **no hosted operator, no bridge required.** Cost =
the chain/storage fee paid from the user's wallet (not a server-side `payment_mode`
charge). After anchoring, the extension does the read-back + verify round-trip (fetch
the COSE bytes, re-check blake3 + Ed25519 against the Solana anchor) and shows the
`delivery_receipt`. If the user has no funded wallet connected, the action prompts
them to connect/fund one — never a silent spend, never falsely "published".

**J3 · Verify.** Paste/scan a tx id or hash → fetch the anchored bytes and re-check
signature + hash against the Solana anchor. Works fully standalone (it is just public
network reads), and for local-only blocks acts as a self-signature check.

**J4 · Continuity + portability.** *On one device:* memory is reused across every
provider's chat via injection. *Across devices:* the user **exports/imports the
signed artifact** to carry memory to another browser/machine — "no infrastructure"
never means "trapped on one device." *Optional bridge:* if installed, the browser
shares the canonical DB with the CLI/IDE (one identity, one DB) — the power-user
upgrade, not the baseline.

---

## The cross-surface demo (why this matters in one story)

1. In **Cursor**, the agent saves `local` "user prefers async Python" — free, private.
2. At the **terminal**, `mnemonic recall "python"` returns it — same DB, no sync.
3. In the **browser** (zero install), the user carries context across ChatGPT,
   Claude, and Gemini via injection, and clicks **Publish & verify** on an audit
   result → `participate`: anchored **directly from their own wallet** to
   Arweave/Solana, read-back-verified, `delivery_receipt` shown. No server in the
   path.
4. A **third party** runs `mnemonic verify <tx>` on another machine and confirms the
   bytes are real *and retrievable* — the ERC-8004 gap Mnemonic closes.
5. The user **exports** their signed memory artifact and **imports** it into a
   browser on a second machine — full portability, still no infrastructure. (A power
   user who *also* runs the CLI/IDE can instead bridge to share the canonical DB.)

## Open / deferred (not designed here)

- **Directed exchange** (hand an artifact to a *specific* recipient with an ACK) —
  deferred to the A2A bridge; `participate` V1 is broadcast/public only.
- **Encrypted / capability-gated share** — future arc; V1 publishes plaintext signed
  bytes.
- **Browser build itself** (buffer + WASM crypto + native-messaging host +
  `install-bridge`) — separate build effort, not this feature's 8 server-side tasks.
