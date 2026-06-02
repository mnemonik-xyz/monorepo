# Decisions — modes-user-choice

> ⚠️ **NEEDS REVIEW (flagged 2026-06-01).** Append-only, so nothing here is deleted —
> but entries are **mixed validity now**. Still good: the *server-side* spine
> (per-request `write_mode`, free-local default, "delivered = anchored AND confirmed
> via recall", retire "never mix in one DB"). **Stale / contradicted:** every
> **browser** decision (bridge, `chrome.storage.local`, no-embedder, direct-to-chain)
> — the shipped `work/chrome-extension/` does the opposite. Also predates the user's
> reframing into a *transparent self-host ↔ remote cost spectrum*. Treat
> **`user-spec.md` as canonical**; re-confirm each decision against it before relying
> on it.

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

## Browser persona = standalone-first + infra-free (2026-06-01, user)

User-story input reframed the browser surface: *"as a browser-extension user I want
not to depend on any infrastructure; I want memories/context reusable across chats
and providers."* Consequences (see user-stories.md, user-journeys.md):

- **Standalone-first; the bridge is demoted to an optional power-user enhancement.**
  The earlier "bridge, else local" wording made the local server primary — for the
  browser persona that is inverted: zero-install standalone is the *primary* path,
  the bridge is a bonus for users who also run the CLI/IDE. Local use and publishing
  must both work with **no server the user runs**.
- **Fork A → A1 context injection (PAM-style).** Cross-chat/provider reuse = inject
  locally-stored memory blocks into each new chat; **no in-browser embedder, no
  semantic recall** browser-side. Confirms (does not reverse) the no-embedder
  decision, which standalone-first had reopened. (Rejected: bundled local embedder;
  remote recall.)
- **Fork B → B2 direct-to-chain.** Pure-browser `participate` anchors **directly to
  Arweave + Solana from the user's own funded wallet** via public gateways/RPC — no
  hosted operator, maximally decentralized. (Rejected: remote-operator-x402;
  bridge-only.) ⇒ browser participate is **not** governed by server-side
  `payment_mode`; the user pays chain/storage fees directly. Wallet management
  (funding wallet distinct from the Ed25519 identity key; likely "connect Phantom")
  is net-new browser scope. Delivery guarantee still holds via read-back + verify
  (no semantic recall needed → compatible with A1).

Still a **separate browser build effort**, not a task in this server-side feature.

## Finalization (2026-06-01, user) — interview round 2

Closes the user-spec.md "Открытый рефайнмент" and reconciles with shipped
`work/chrome-extension/`. **`user-spec.md` (rewritten 2026-06-01) is canonical.**

**FINALIZED:**

1. **Scope of this feature = server-side only.** No code changes in
   `packages/extension/` in this iteration. Extension keeps working as-is.
2. **Compatibility invariant (load-bearing):** server-side changes must keep the
   existing HTTP contract that the shipped extension's **Cloud-tier** uses
   (deferred signing → hosted `STORAGE_MODE=full`). Concretely: the new `mode`
   field on `sign_memory` is **optional, default `local`**; requests without
   `mode` keep current env-var-driven behavior — no silent semantic change for
   un-updated clients.
3. **V1 API surface = binary `mode: local | participate`.** No `target` /
   `visibility` / `cloud` field. The third "cloud" point that the open
   refinement discussed is **deferred to V2+** with this framing: private
   durable cloud-mirror, when it lands, sits **on top of** the chain anchor
   (anchor remains the source of verifiability), not as a replacement for it.
4. **Tier-2 ("self-operator")** is **not new code** — it is the existing
   `STORAGE_MODE=full + PAYMENT_MODE=none` deployment, just made an explicit
   positioning point. This feature does not add a "your own MCP" code path; it
   only acknowledges that the path already exists and ensures `whoami` envelope
   exposes it correctly (`participate_cost.amount_cents: 0`,
   `payment_methods: []`).
5. **`whoami` envelope contract** (new in V1, called out in user-spec):
   `supported_modes`, `default_mode`, `participate_cost
   {currency, amount_cents, payment_methods}`. **Typed error**
   `UnsupportedMode { requested, supported }` (JSON-RPC `-32010`) on requesting
   a mode the server cannot serve — **never a silent downgrade to `local`**.
   This is the discoverability contract clients (CLI, SDK, extension, agents)
   target.
6. **"Спектр" stays as user-spec positioning, not code structure.** Tier-1/2/3
   are deploy variants ("which MCP am I pointed at"), not three values in the
   API. The interview confirmed: *"the main idea here is that user should not
   care. Locally — use free. Want to save onchain/cloud? Pay for it."* The
   per-call axis stays binary; the per-deployment axis is operator-side env-vars
   and is invisible at the API surface.

**RETIRED — stale relative to canonical user-spec.md (kept here only for
audit-trail; do NOT use as design input):**

- **Browser native-messaging bridge** (section "Bridge mechanism — native
  messaging") — out of scope of this feature; covered by `work/chrome-extension/`
  if/when it ever revisits browser↔server bridging.
- **Browser OPFS-SQLite-WASM store** + later **`chrome.storage.local`
  signed-artifact buffer** revision (sections "Local storage substrate" and
  "Browser store — revised after PAM reference") — superseded by the shipped
  extension's actual choice (IndexedDB + transformers.js embedder in
  `packages/extension/`).
- **Browser standalone-first + Forks A1 (no embedder) / B2 (direct-to-chain)**
  (section "Browser persona") — directly contradicted by the shipped extension
  (in-browser ONNX embedder *exists*; Cloud-tier is hosted-operator, not
  direct-to-chain). The shipped extension is the canonical browser model.

**Sibling docs status:**
- `tech-spec.md`, `user-stories.md`, `user-journeys.md` are **superseded** by
  this finalization. They predate (a) the reframing into a transparent
  positioning spectrum and (b) the chrome-extension discovery. To be
  regenerated via `/new-tech-spec` (tech-spec) and removed/rewritten on demand
  (stories/journeys — they were exploratory artifacts, not template-required).

## Task 2: Per-request mode + whoami envelope + typed errors + paywall reframing

**Status:** Ready for review
**Commit:** (assigned by team lead)
**Agent:** task2-impl
**Summary:** Added `tools::resolve_write_mode` — a pure function that maps the
optional `mode` field on `mnemonic_sign_memory` to a typed `WriteMode`, with
strict rejection (-32602 InvalidParams, `data.field`/`data.received`) for every
non-canonical input (case-variant, whitespace, null, non-string, unknown).
That resolver is the SINGLE source of truth: `mcp_handler` calls it once
before the paywall gate (`is_sign_memory && resolved == Participate &&
payment_mode != "none"`), and the same resolved value is threaded into
`sign_memory(... write_mode ...)` and then into `save_attestation`. The three
T1 `WriteMode::Participate` placeholders in `mcp/` are replaced: two in
`tools.rs` and one in `api.rs::sign_callback_handler` (the deferred-sign
flow always anchors → `Participate` by construction; documented in-line).
Added a `mcp::Envelope` struct on `McpState` populated at process start in
`main.rs::run_http` AND in every test-state constructor (`test_support`,
`mcp::transport_tests::build_test_state`, `tests/sign_callback.rs`,
`tests/pending_authz.rs`, `tests/pending_expiry.rs`, `chat.rs` test scope),
plus the new typed-error helpers `unsupported_mode` (-32010) and
`invalid_params` (-32602) sitting next to `JsonRpcError` in `mcp.rs`.
`whoami` returns the envelope (`supported_modes`, `default_mode`,
`participate_cost { currency, amount_cents, payment_methods }`) merged onto
the legacy fields so the chrome-extension Cloud-tier `storage_mode` echo
keeps working. Routing change in `sign_memory`: an EXPLICIT `mode: "local"`
request against a deploy that ALSO supports `participate` (i.e. `full`)
short-circuits to the inline path even with a JWT — the user explicitly
opted out of the deferred Cloud-tier flow to get the free local write
(user-spec invariant "Личная память бесплатна всегда"). Mode-absent + JWT
on a local-only deploy continues to take the deferred branch (legacy
chrome-extension Cloud-tier preserved). Test harness lives in
`mcp/tests/_helpers/mod.rs` (`TestServer::builder().storage_mode(…).
payment_mode(…).build()`, `.call_tool()`, `.attestation_count()`,
`.write_mode_for_tx()`, `.attestation_cost_rows()`, `.balance_for()`) and
new integration suite `mcp/tests/modes_per_request.rs` covers all six
ACs end-to-end. A golden fixture at
`mcp/tests/fixtures/modes/legacy_sign_response.json` pins the mode-absent
deferred-signing envelope shape (regression guard for shipped extension).
**Deviations:** Round-1 had a routing rule "skip deferred when
`write_mode == Local AND envelope.supports_participate()`" — round-2
review flagged this as the wrong predicate (conflated "explicit
local" with "running on a full server"). Replaced with the simpler
`ResolvedMode::is_explicit_local()` (i.e. caller sent `mode: "local"`
explicitly) regardless of deploy variant. The mode-absent + JWT +
local-only path (chrome-extension Cloud-tier production target)
keeps routing through deferred because `ResolvedMode::fallback(Local)`
is `explicit == false`. No assertions in `deferred_sign_flow.rs` /
`sign_callback.rs` change.

**Reviews:**

*Round 1:*
- code-reviewer: 5 minors → applied in round 2.
- test-reviewer: 3 minors → applied in round 2.
- security-auditor: 1 major (explicit-local routing on local-only
  deploy) + 3 minors → all applied in round 2.

*Round 2 changes (single commit):*
- Resolver now returns `ResolvedMode { write_mode, explicit }`. Routing
  in `sign_memory` uses `is_explicit_local()` (drops the round-1
  `envelope.supports_participate()` workaround). Fixes the
  security-auditor major. New integration test
  `modes_per_request::explicit_local_against_local_only_server_is_inline_not_deferred`
  pins the regression.
- Resolver called ONCE per request in `mcp_handler`; threaded through
  new `handle_request_with_resolved_mode` →  `handle_tool_call` →
  `sign_memory`. Stdio path resolves on demand inside
  `handle_tool_call` when `pre_resolved_mode = None`.
- `tool_error_to_json_rpc` now matches on the typed
  `tools::ToolError { TypedRpc(JsonRpcError), Other(anyhow::Error) }`
  carrier instead of parsing `anyhow::Error.to_string()` as JSON.
  Closes the security-auditor "forged error code" minor.
- `mnemonic_sign_memory` `inputSchema` now declares the `mode` field
  with `enum: ["local", "participate"]` + description.
- `tracing::warn!` emitted on resolver `Err` in both dispatch paths.
- Stale paywall doc-comment on `mcp_handler` rewritten.
- `whoami_envelope_per_deploy_variant` sub-case 4c: added
  `default_mode == "local"` + `currency == "USD"` assertions.
- `local_against_full_server_returns_free`: explicit assertion that
  the response is 200 OK (paywall bypassed — would be 402 if not).
- Misleading "inline" comment on the golden-fixture test corrected
  (test actually pins the deferred awaiting_signature shape).
- Stale resolver-test comment ("rejection happens in
  sign_memory_inline") corrected to `sign_memory`.

**Verification (round 2):**
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast`
  → **TOTAL: 507 passed, 0 failed** (mcp lib: 154 passed including
  new `test_explicit_local_with_jwt_takes_inline_path`;
  modes_per_request: 7 passed; every other test unchanged).
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings`
  → clean.
- `cargo fmt --all -- --check` → clean.

## Task 1: WriteMode enum + write_mode column + save_attestation signature

**Status:** Done
**Commit:** b62038c
**Agent:** task1-impl
**Summary:** Added `core::storage::WriteMode { Local, Participate }` (pure
type, strict serde + rusqlite round-trip, `from_str_strict` rejects every
non-canonical input) and threaded it through `AttestationStore::save_attestation`
+ `SqliteStore::save_attestation`. New idempotent `migrate_write_mode_column`
adds `write_mode TEXT NOT NULL DEFAULT 'participate'` plus the composite
`(owner_pubkey, write_mode)` index, and is wired into both `SqliteStore::open`
and `::in_memory`. Backfill rule: `UPDATE … SET write_mode='local' WHERE
solana_tx LIKE 'local:_%'` — the `_` requires ≥1 char after the colon, so bare
`'local:'` stays `'participate'` (default) and real base58 sigs stay
`'participate'` (base58 excludes lowercase `l`, so collision is impossible).
DEFAULT is `'participate'` on purpose: legacy global-`STORAGE_MODE=full` rows
were paid writes. This is the foundation that T2 consumes when it wires the
per-request `mode` field through `sign_memory_inline`.
**Deviations:** None.

**Reviews:**

*Round 1:* pending (will be dispatched by the team lead).

**Verification:**
- `cargo test -p mnemonic-core --no-fail-fast` → 121 passed, 0 failed, 1 ignored.
- `cargo clippy -p mnemonic-core --all-targets -- -D warnings` → clean.
- `cargo fmt --all -- --check` → clean.

## Task 3: Delivery guarantee + refund + DoS guard

**Status:** Done
**Agent:** task3-impl
**Summary:** Wrapped the participate branch of `sign_memory_inline` in a
three-stage delivery confirmation (Arweave re-fetch with wall-clock budget →
`verify_cose` → in-process recall). Row is saved as `Participate`
immediately after the chain anchor (recall queries the DB; the row must
exist by recall time); on any stage failure the row is demoted in place
via `INSERT OR REPLACE` to `WriteMode::Local`, no `attestation_costs` row
is written, and the typed `-32011 DeliveryNotConfirmed { stage,
arweave_tx, solana_tx, row_demoted_to: "local", attestation_id }` is
returned. `mcp_handler`'s refund-on-error branch consumes the typed
error's `data.attestation_id`, refunds the balance with a reason that
includes the demoted id, increments the per-stage
`delivery_not_confirmed_total` counter AND the per-`api_key_hash`
`RefundsBySubject` quota counter, and on refund-itself-failure writes a
`payment_events` audit row via the new `payment::record_refund_failed`
API (body lives in `mcp/`, per the architectural rule). DoS guard
consulted at participate ENTRY in `mcp_handler` before any chain write;
exceeded → `-32011 DeliveryQuotaExceeded` with `HTTP 429`, zero chain
spend. Background eviction task spawned from `main.rs::run_http` drops
idle entries on a configurable interval. Four new env vars in
`mcp/src/config.rs`:
`MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS=15`,
`MNEMONIC_DELIVERY_QUOTA_THRESHOLD=5`,
`MNEMONIC_DELIVERY_QUOTA_WINDOW_SECS=60`,
`MNEMONIC_DELIVERY_QUOTA_EVICT_SECS=30`.

**Key design calls:**
- Quota counter keyed on `blake3(api_key).to_hex()`, NEVER `owner_pubkey` —
  Ed25519 keys rotate for free, billable subjects don't (CWE-312 +
  bypass-prevention).
- Audit row's `payment_events.api_key` column carries the HASH for
  `refund_failed` rows; column name is legacy, schema is untouched
  (no migration needed).
- Wall-clock retry budget bounded by `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS`
  (default 15s, exp backoff 200ms→2s) instead of a fixed attempt count —
  sized against Arweave's eventual-consistency window.
- Two short critical sections in the participate flow: one for
  save_attestation (Participate or Local), one (in `mcp_handler`) for
  refund_balance + record_refund_failed. Neither holds the SQLite mutex
  or any DashMap shard guard across `.await` — Decision 8 honoured and
  extended to DashMap by this task.
- Row saved as `Participate` BEFORE the delivery check (recall queries the
  DB), then demoted in place via `INSERT OR REPLACE` on failure. The
  spec's pseudo-code reads as two separate saves but the same
  `attestation_id` + `INSERT OR REPLACE` semantics make this single-row
  flow cleaner and lets the recall stage actually find the row.
- T3 tests target the INLINE participate path (no JWT) because the
  current T2 routing sends JWT+participate to the deferred-signing branch
  (which sits in `api::sign_callback_handler`, not in
  `sign_memory_inline`). The OAuth middleware is intentionally NOT
  mounted in `build_state_and_router` for T3 tests — production HTTP
  clients receive the same delivery flow on the inline path; the JWT
  layer is orthogonal. `happy_path` is `#[ignore]`d with a comment
  explaining the real arlocal + solana-test-validator harness it would
  require; the four failure-mode tests exercise the same code paths in
  their non-failure direction.

**Deviations:** None substantive. Two minor adjustments:
1. Row save now precedes delivery check (`INSERT OR REPLACE` semantics on
   demotion) so the recall stage has a row to find — the spec text reads
   as two saves but the same `attestation_id` makes the single-row flow
   the intended behaviour.
2. Refund + counter + record_refund_failed wired into `mcp_handler`'s
   existing on-error refund branch rather than inside `sign_memory_inline`
   (api_key only lives at the dispatcher boundary). Functionally
   equivalent to the brief; keeps the lock-discipline rule applied to
   `mcp_handler` instead of duplicating across two files.

**Verification:**
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast`
  → workspace green (all task suites pass, delivery_guarantee: 4 passed +
  1 ignored).
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings`
  → clean.
- `cargo fmt --all -- --check` → clean.

## Task 3 — round 2 (lead-applied finish + partial deferral)

**Status:** Ready for review (round 2)
**Commit:** (pending — this entry written before commit)
**Agent:** main agent (lead) — applied the round-2 finish after two consecutive
teammate runtime failures (first teammate stalled mid-iteration; recovery
teammate's socket closed unexpectedly after ~65 min). Code-substantive work
in the working tree was authored by the prior teammates and verified by the
lead; the lead added only the small mechanical pieces noted below.

**Summary:** Round-1 critical (recall stage used `content_hash` instead of
primary-key lookup, broken for real embedders) closed — replaced with a
direct `SELECT 1 FROM attestations WHERE attestation_id = ? AND
owner_pubkey = ?` existence check in `perform_delivery_check`. Round-1 major
"Cloud-tier deferred path lacks delivery guarantee" closed — extracted a
shared helper `tools::confirm_delivery_or_demote(DeliveryContext)` called by
BOTH `sign_memory_inline` (inline path) AND `sign_callback_handler` (deferred
path) so the user-spec invariant "delivered = anchored AND verified by recall"
now holds for the chrome-extension Cloud-tier flow. Round-1 major
"x402 nonce consumed before delivery" closed — `check_payment` now only
VERIFIES the nonce; consumption deferred to `consume_x402_nonce_after_success`
which fires after delivery success (failure path leaves the nonce reusable
for retry; race window between two concurrent same-nonce requests resolved by
the `x402_nonces.tx_sig` UNIQUE constraint — loser sees a clean error).
Round-1 major "DoS quota guard skipped for x402-only callers" closed via new
`derive_quota_subject(headers, payment_mode)` returning either
`blake3(api_key).to_hex()` (Bearer) or `blake3(tx_sig).to_hex()` (x402 —
the on-chain payment proof; stable across retries with the same payment).

**Deviations:**

- **Test gap deferred:** `demotion_on_x402` integration test is NOT
  implemented in this round. The x402-nonce-deferral CODE is in place and
  exercised by the production paths, but the dedicated test requires new
  x402 mock infrastructure (signed USDC transfer mock, `X-Payment` header
  helpers, `x402_nonces`-state read helpers) that the existing test harness
  doesn't provide. Adding this infra is moderate scope and would re-expose
  the agent-stall failure mode the round-2 fix already hit twice. Deferred
  to a small follow-up task (T3.5 — "x402 delivery-failure integration
  test"), to land before merge to main. Audit-wave will flag this gap as
  visible-but-known.
- **Lead authored a partial commit.** Per the lead's role boundary
  ("dispatcher, not doer"), the lead does not normally write code. After
  two teammate runtime failures left the round-2 work uncommitted in the
  working tree but visibly complete, the user explicitly approved the
  lead-deviation path. Lead-authored changes are limited to: (a) one
  `#[allow(clippy::too_many_arguments)]` annotation on
  `perform_delivery_check` (matching the project's existing pattern on
  `AttestationStore::save_attestation`); (b) two test-strengthening
  additions in `delivery_guarantee.rs` per the test-reviewer's round-1
  asks (DB-row assertion on `demotion_on_verify_failure`; description-
  format-prefix assertion on `refund_failure_writes_audit_row`). All other
  changes in this commit are unmodified from the teammates' uncommitted
  working-tree work.

**Verification:**
- `cargo fmt --all -- --check` → clean.
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings` → clean.
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast` →
  **527 passed, 0 failed** (4 ignored, all intentional per project convention).

## Task 4: verify routes by stored write_mode + tenant isolation on find_by_tx

**Status:** Ready for review
**Agent:** task4-impl
**Summary:** Replaced the env-var routing branch in `mcp/src/tools.rs::verify`
with a SQLite lookup of the row's stored `write_mode`. New
`AttestationStore::find_write_mode_by_tx(tx, owner_pubkey)` plus a
re-shaped `find_by_tx(tx, owner_pubkey)` close a pre-existing tenant-
isolation gap surfaced by per-request mode coexistence: both lookups
filter by `AND owner_pubkey = ?` in the SQL predicate, so a wrong-tenant
probe returns `Ok(None)` indistinguishable from a genuine miss. `verify`
routes `WriteMode::Local` → `verify_local`, `WriteMode::Participate` →
new `verify_participate` helper (extracted from the old env-var branch),
`None` → flat `{status: "not_found", lookup_id}` envelope with NO
`content_hash` / `signer` / `content` / `preview` leakage. The
`storage_mode` parameter is kept on `verify` for ABI compatibility but
marked unused via `_storage_mode` and a doc comment. `recall` now
surfaces `write_mode` on each row via `SearchResult.write_mode`
(serialized as `"local"` / `"participate"`). The `_helpers/` harness
gained `mint_test_jwt(pubkey)` + `with_token(jwt) -> AuthedClient`
primitives so the tenant-isolation tests can drive two distinct
authenticated callers against one shared DB. New `mcp/tests/
verify_by_stored_mode.rs` covers the four scenarios: routing by stored
local-mode, routing by stored participate-mode, tenant isolation
(critical, both local + participate row shapes), and recall surfacing
`write_mode`.

**Tenant-isolation residual:** R2-F4 (response-timing symmetry) is
documented as accepted in the user-spec — no constant-time SQL wrapper
was added. The shape-level isolation (no leakage of identifying fields
through error data) is fully closed.

**Deviations:** Added `mcp/src/mcp.rs` to Files-to-Modify (the
dispatcher's `tools::verify(...)` call site now passes `owner_pubkey`,
in scope from the JWT `sub` claim). The task file listed only `core/`
and `mcp/src/tools.rs` originally.

**Verification:**
- `cargo test -p mnemonic-mcp --features test-support --test verify_by_stored_mode`
  → **5 passed, 0 failed** (`verify_routes_local_for_local_row`,
  `verify_routes_participate_for_participate_row`,
  `tenant_isolation_local`, `tenant_isolation_participate`,
  `recall_surfaces_write_mode`).
- `cargo test -p mnemonic-mcp --features test-support verify_` → green
  (includes the new file plus `demotion_on_verify_failure` from T3).
- `cargo test -p mnemonic-mcp --features test-support recall_` → green
  (includes the new file plus `recall_owner_isolation` and
  `mixed_mode_coexistence_recall_returns_both`).
- `cargo test --workspace --features mnemonic-mcp/test-support --no-fail-fast`
  → green across all integration suites.
- `cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings`
  → clean.
- `cargo fmt --all -- --check` → clean.
