# Decisions — modes-user-choice

Append-only log of decisions and audit findings.

## Interview outcomes (2026-06-01, user)

**DECIDED:**
- **Payment UX** — *pay per shared artifact* (per-`participate`-write cost; local
  writes always free). Aligns with issue #28's per-sign model.
- **Retraction** — *permanent / immutable*. Once participated, the anchor is
  immutable; no un-share / tombstone in V1. Matches append-only design.

**RESEARCH-BACKED RECOMMENDATIONS (deep-research 2026-06-01, see research.md —
✅ SIGNED OFF, now items #2/#3 in FINALIZED below):**
- **What "participate" means → broadcast-publish a verifiable public record**, not
  recipient-ACK handoff. Evidence: cross-operator exchange is dominated by
  *directed* message-passing that explicitly shares no memory (A2A "Opaque
  Execution"); the only *broadcast* pattern shipping is ERC-8004's public
  attestation/reputation registry — which is Mnemonic-shaped. Verifiable
  cross-operator *shared memory* is still aspirational, so V1 doesn't bet on it.
  Directed exchange deferred to the A2A bridge.
- **Delivery-guarantee → D1: durable write + read-back + re-verify.** Cheapest
  guarantee meaningfully stronger than a receipt; needs no online counterparty;
  fills the exact gap ERC-8004/IPFS leave open (hash committed, availability NOT
  guaranteed). Recipient-ACK (D2) is the gold standard but needs an online
  counterparty → deferred. Receipt schema forward-shaped for D2.
- **Mnemonic wedge identified:** "ERC-8004 proves a hash; Mnemonic proves the
  bytes are actually retrievable."

## Open decisions — ✅ ALL RESOLVED (see FINALIZED below)

— all resolved, see FINALIZED below.

## FINALIZED (2026-06-01, user sign-off)

1. **Storage invariant → S1** (tag rows with `write_mode` in one DB; local +
   shared coexist for one user; recall spans both). Retires CLAUDE.md's "Never
   mix in one DB" as a *conscious* change (update in same PR).
2. **Delivery definition → "anchored AND verified by recall = delivered"** (user's
   words). Supersedes the abstract "D1 read-back": the delivery proof is a
   **recall + verify round-trip against the anchored artifact** — anchor on
   Arweave/Solana, then confirm recall can retrieve it and `verify` re-checks the
   anchored bytes (hash + COSE signature). Reuses existing recall/verify
   machinery; no bespoke read-back primitive. Until that round-trip passes, the
   write is NOT "participated" (still local; no charge on failure).
3. **Participate semantics → broadcast / public publish.** Anchor signed COSE
   **plaintext** (today's shape). Anyone with the tx id can read+verify — the
   ERC-8004-style public attestation model. **No encryption / no key-based access
   in V1** (user: "Public publish"). Encrypted-share + capability-token sharing
   is a future arc, not this iteration. Directed/recipient-ACK exchange deferred
   to the A2A bridge.
4. **Mode granularity → per-request `mode` field on `sign_memory`** (default
   `local`).
5. **Payment → per shared artifact**; local always free (from interview).
6. **Retraction → permanent / immutable** (from interview).

## Local storage substrate & cross-surface topology (2026-06-01, user)

The `local` mode must fit **all four surfaces** — browser extension, CLI, SDK,
and IDE-hosted agents. Decided:

- **Substrate = SQLite everywhere, one schema, one `.db` semantics.** Native
  `rusqlite` for the surfaces that have a filesystem (CLI, IDE agents via the
  local `mnemonic-mcp` server, Node-SDK); the server owns the canonical
  `~/.mnemonic/attestations.db` and CLI + IDE agents + Node-SDK **share it** —
  the storage analogue of invisible-identity's one-keypair-everywhere.
- **Browser reach = "bridge, else local"** (user pick). The extension connects to
  a running local `mnemonic-mcp` when reachable and shares the **same canonical
  `~/.mnemonic/attestations.db`** — fully unified, real-time, no copies. When the
  bridge is unreachable (no host installed / locked-down browser), it falls back to
  its own browser-local store. ⚠️ *The offline-store specifics first written here
  (OPFS-backed SQLite-WASM) were **SUPERSEDED** — see §"Browser store — revised
  after PAM reference" below: the offline store is a `chrome.storage.local`
  signed-artifact buffer, no SQLite-WASM.*
- **Accepted cost = a transient split-brain window.** A browser write made while
  the bridge is down lives only in the browser-local store until it converges — and
  convergence is the **protocol's** job (shared Ed25519 identity +
  `participate`/anchor + `recall`, i.e. the `local → participate` path this
  feature builds), **not** an automatic local file merge. No new machinery — just
  the divergence window. Chosen over "bridge-only" (which avoids split-brain by
  refusing to work offline) to never strand the user.
- **Backend shape = SQLite everywhere on native, no abstraction** (user pick
  "SQLite everywhere"). Keep the concrete `SqliteStore` — no `AttestationStore`
  trait. `rusqlite` on every native/server surface, one schema. ⚠️ *The browser
  half ("OPFS-WASM SQLite in the browser") was **SUPERSEDED** by the PAM revision
  below — the browser is **not** SQLite, it is a `chrome.storage.local` artifact
  buffer.* The browser store is net-new TS outside `core/` (native-only by rule)
  and is a **separate build effort**, not a task in this server-side feature.

**Clarification that reframed the guarantee (user, 2026-06-01):** anchored-on-
Arweave = "mission done" — the risk surface is *local-only* artifacts. `participate`'s
job is to move an artifact out of the local-only risk surface into the anchored
state; the only silent failure is reporting "participated" when the anchor didn't
actually land. That is exactly what "verified by recall" closes. Correction logged:
today's Arweave write is **signed, not encrypted** (`core/src/arweave/mod.rs:56`,
`core/src/wasm/mod.rs:208`) — so "public publish" is consistent with current code;
encryption would have been net-new.

## Locked context (from user, 2026-06-01)

- Local storage (filesystem / self-hosted) ⇒ **free, no payment**. Structural,
  per whitepaper §5.7.1.
- "Participate" (share/exchange artifacts with the network) ⇒ protocol must
  **guarantee delivery**; purely-local artifacts are risky to share because they
  may be unreachable when a peer asks. This is the paid service-layer path.
- Mode must be the **user's** choice driven by intent, not an operator env var.

## Browser store — revised after PAM reference (2026-06-01, user)

**Supersedes the OPFS-SQLite browser node** in §"Local storage substrate" above
(commit `9fc81d2`). Trigger: the user shared how Portable Agent Memory (PAM)
stores data — no DB at all; a memory is a single signed JSON `.pam` artifact
(five components: episodic/semantic/procedural/working/identity + integrity), and
the MV3 extension persists the whole artifact under one `chrome.storage.local`
key (`pam_artifact`), service-worker single-owner. Two things PAM forced:

- **Browser offline = write-only buffer, NOT a local query node** (user pick,
  reasoned: *"is it ok to go with write-only buffer?"* — yes). Decisive reason:
  recall must **embed the query**, which needs an embedder running offline. Without
  bundling the ~22 MB fastembed ONNX model into the extension (OpenAI is
  unreachable offline), **there is no semantic recall offline regardless of the
  store** — so OPFS-SQLite-WASM only ever paid off if we *also* shipped an
  in-browser embedder. We drop both. Browser store becomes a durable
  `chrome.storage.local` **artifact buffer** (PAM proves it's persisted
  per-write, atomic, zero-WASM) holding signed artifacts; on bridge-return the
  local server ingests them and serves recall. Offline you can list/read buffered
  artifacts but not semantically search them. **No OPFS-SQLite-WASM, no
  in-browser embedder, no `sql.js`.** This shrinks the "separate browser build
  effort" considerably.
- **Browser artifacts stay first-class verifiable — diverge from PAM here** (user
  pick: "Match the protocol"). PAM's browser edition degrades to **SHA-256 +
  empty `signature`**, so its browser artifacts are unsigned and their hashes
  don't match the SDK's `blake3:` — a non-verifiable tier. For Mnemonic that
  breaks the whole "verifiable memory" thesis, so the extension runs
  **blake3-wasm + Ed25519-wasm**: every buffered artifact is content-addressed
  and COSE-signed **identically to native/SDK artifacts** (interoperable hashes,
  real signatures). Both primitives run in-browser via WASM — PAM simply chose
  not to.

**Net effect on the earlier topology:** "bridge, else local" stands; "SQLite
everywhere" now means **SQLite on all native/server surfaces** (CLI / IDE agents /
Node-SDK share the canonical `~/.mnemonic/attestations.db`). The browser is **not**
SQLite — it is a `chrome.storage.local` signed-artifact buffer. Convergence is
still the protocol's job (shared Ed25519 identity + `participate`/anchor + the
server ingesting the buffer on bridge-return); the transient split-brain window is
unchanged.

## Bridge mechanism — native messaging (2026-06-01, user)

Locks *how* the "bridge, else local" extension reaches the local server. Chrome/Edge
allow only two extension→local-process channels; user picked **native messaging**.

- **Channel = native messaging** (`chrome.runtime.connectNative`). A one-time
  install drops a *native-messaging host manifest* (JSON naming the `mnemonic`
  binary + allowed extension IDs); thereafter **Chrome auto-spawns the binary on
  demand** and kills it when the port/tab closes. The user never launches a daemon
  by hand. The spawned host opens the canonical `~/.mnemonic/attestations.db`
  directly (SQLite multi-process file locking / WAL arbitrates concurrent access —
  no "single long-lived owner" assumption needed).
- **No admin / root required.** Manifest + binary install entirely in **user
  space** — per-user manifest dirs (`~/.config/google-chrome/NativeMessagingHosts/`
  on Linux, `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/` on
  macOS, **`HKCU`** registry on Windows, never `HKLM`), binary in `~/.local/bin` /
  `~/.cargo/bin`. A `mnemonic install-bridge` command does it with no `sudo`/UAC.
- **`localhost` HTTP = power-user alternative, not default.** `fetch` against a
  running `mnemonic-mcp --transport http`. Chrome will NOT start it; the user must
  keep the daemon alive (manually or via a login/systemd/launchd service). Offered
  for users who already run the daemon; not the recommended path.
- **Policy-locked machines degrade gracefully.** MDM-managed/corporate browsers can
  disable native messaging or block writes to the manifest dir. In that case the
  bridge never comes up and the extension falls back to the offline
  `chrome.storage.local` signed-artifact buffer — the exact reason "bridge, else
  local" never assumes the user *can* install anything.

All of this remains part of the **separate browser build effort**, not a task in
this server-side feature.
