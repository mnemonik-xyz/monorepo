# MCP Tool Reference

Every tool the Mnemonic MCP server exposes over JSON-RPC 2.0, with its inputs,
outputs, and auth requirements. Source of truth: `mcp/src/mcp.rs`
(`tool_definitions()` for the schemas, the `call_tool` dispatch for behaviour).

The source refactor adds [binary signed-artifact ingestion](./client-prepared-memory.md)
and [client recovery checkpoints](./recovery-checkpoints.md).
Their migration guides define retired sealed-storage and hosted-decryption routes.
Source support does not establish deployment or published-package availability.

## Source capability boundaries

| Path | Preparation and storage | Limitation |
|---|---|---|
| Plain stdio local memory | Agent-owned store; legacy unsigned row | Not an external signed artifact |
| SDK sealed `store` | Client encryption/signature and session cache | Caller must persist bytes and keys |
| Client-prepared external memory | Signed original bytes externally; hosted metadata | Explicit public consent for plaintext; verify receipt independently |
| HTTP A2A external delivery | Client-signed bytes, shared operation coordinator | Parent verification and supported payment rail required |
| Hosted semantic recall | Available searchable index rows | No sealed decryption or completeness guarantee |
| SDK sealed recall | Local opening and caller-supplied embedder | Only the client's restored/cached corpus |
| General-memory grant creation | Retired hosted writer and SDK method | Existing grant reads/withdrawal retained |

This matrix describes source behavior. Live-provider, package-install and pilot
release drills remain separate evidence gates.

**Endpoints**

| Transport | Address | Auth |
|---|---|---|
| HTTP | `POST https://mcp.mnemonik.xyz/mcp` (self-host: `http://localhost:3000/mcp`) | OAuth 2.1 + PKCE (`Authorization: Bearer <jwt>`) |
| stdio | `npx @mnemonik-xyz/cli mcp` | local keypair at `~/.mnemonic/identity.json` |

New to the protocol? Start with the [Quickstart](./QUICKSTART.md); this page is
the reference you come back to.

---

## Tool index

The default build advertises **12 tools**. Three more appear only when the server
is compiled with the `trajectory-experimental` cargo feature.

| Tool | Auth | Paid | Purpose |
|---|---|---|---|
| [`mnemonic_whoami`](#mnemonic_whoami) | required on HTTP | no | Server identity, storage capabilities, pricing |
| [`mnemonic_sign_memory`](#mnemonic_sign_memory) | required for `anchored` | `anchored` only | Create a signed memory attestation |
| [`mnemonic_check_pending`](#mnemonic_check_pending) | required | no | Resolve a deferred-sign `correlation_id` |
| [`mnemonic_recall`](#mnemonic_recall) | optional (changes scope) | no | Semantic search over stored memories |
| [`mnemonic_verify`](#mnemonic_verify) | required on HTTP | no | Verify an attestation against its chain anchors |
| [`mnemonic_prove_identity`](#mnemonic_prove_identity) | required on HTTP | no | Sign an arbitrary challenge with the server key |
| [`mnemonic_operator_proof`](#mnemonic_operator_proof) | none | no | Prove the operator key before a client sends credentials |
| [`mnemonic_publish_post`](#mnemonic_publish_post) | required | no | Publish a signed public blog post |
| [`mnemonic_share`](#mnemonic_share) | required | no | Legacy approval-URL handoff; not grant delivery |
| [`request_public_write_confirmation`](#request_public_write_confirmation) | — | no | Internal ceremony gate (not user-facing) |
| [`mnemonic_attest_a2a`](#client-signed-a2a-tools) | required | external writes on paid operators | Deliver client-signed A2A bytes |
| [`mnemonic_recall_a2a`](#client-signed-a2a-tools) | required | no | Read authorized A2A receipt metadata |
| [`mnemonic_attest_step`](#mnemonic_attest_step) ⚗️ | required | no | Append a hash-linked trajectory step |
| [`mnemonic_attest_verdict`](#mnemonic_attest_verdict) ⚗️ | required | no | Record an independent judge's verdict |
| [`mnemonic_verify_trajectory`](#mnemonic_verify_trajectory) ⚗️ | required on HTTP | no | Verify a trajectory end-to-end |

⚗️ = experimental, behind `trajectory-experimental`.

The Auth column applies to the HTTP transport. The stdio transport uses the
local keypair and needs no token. Refer to
[Authentication over HTTP](#authentication-over-http).

External memory and A2A delivery can require payment on paid operators.
Read tools do not charge. Rail support and quota are operator capabilities;
see [Payment](#payment).

---

## Calling a tool

Tools are invoked with the standard MCP `tools/call` method:

```bash
curl -s https://mcp.mnemonik.xyz/mcp \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $MNEMONIC_JWT" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "mnemonic_recall",
      "arguments": { "query": "what did we decide about pricing", "limit": 5 }
    }
  }'
```

To enumerate the live surface of any server — the reliable way to tell whether
the experimental tools are compiled in:

```bash
curl -s https://mcp.mnemonik.xyz/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq '.result.tools[].name'
```

---

## Authentication over HTTP

The HTTP endpoint uses OAuth 2.1 with PKCE (Proof Key for Code Exchange).
Send the access token in each request as `Authorization: Bearer <token>`.
The token is a JWT (JSON Web Token). These rules are available now.

**Requests that work without a token:**

- `initialize`, `ping` and `tools/list`
- `prompts/list`, `prompts/get`, `resources/list` and `resources/read`
- JSON-RPC notifications, for example `notifications/initialized`
- `tools/call` for `mnemonic_recall` (it searches public rows only; private
  rows go only to their owner)
- `tools/call` for `mnemonic_operator_proof`

All other requests need a valid token.

**Error responses.** The server sends HTTP 401 (Unauthorized) in two cases.
Each 401 has a `WWW-Authenticate: Bearer` header with a `resource_metadata`
parameter. For `/mcp`, this parameter points to
`/.well-known/oauth-protected-resource/mcp`.

| Case | HTTP status | `WWW-Authenticate` parameters |
|---|---|---|
| The request needs a token and has no token | 401 | `realm`, `resource_metadata` (no `error`) |
| The request has a token that is expired or not valid | 401 | `realm`, `error="invalid_token"`, `error_description`, `resource_metadata` |

The second case applies to all methods, also to the methods in the list above.
An expired token on `initialize` gets a 401, so the client can refresh the token
before it calls a tool. To use a method from the list without a token, send no
`Authorization` header.

The body of each 401 is a JSON-RPC error with code `-32001`.

---

## `mnemonic_whoami`

Identity and capability discovery. Call this **first** — it tells you which write
modes the operator supports and what an `anchored` write costs, so a client can
choose before attempting a write that might be rejected or charged.

**Input:** none.

**Returns:**

```jsonc
{
  "public_key": "<base58 Ed25519>",
  "did_sol": "did:sol:<base58>",
  "did_key": "did:key:z6Mk...",
  "attestation_count": 42,
  "storage_mode": "full",          // legacy field, kept for pre-envelope clients
  "supported_modes": ["local", "anchored"],
  "default_mode": "local",
  "anchored_cost": {            // null when the server cannot anchor (local only)
    "currency": "USD",
    "amount_micro_usdc": 1000,
    "amount_cents": 1,
    "pricing_status": "fallback",
    "payment_methods": ["x402"]
  },
  "free_anchors": {                // HTTP + PAYMENT_MODE=x402 only
    "eligible": true,
    "per_day": 10,
    "per_ip_per_day": 20,
    "max_bytes": 16384,
    "remaining": 7,
    "global_remaining": 812,
    "resets_at": "2026-09-28T00:00:00Z"
  }
}
```

`storage_mode` reflects the operator's *capability*, not a global switch — see
[Write modes](#write-modes-local-vs-anchored).

**`anchored_cost` fields:**

| Field | Type | Meaning |
|---|---|---|
| `currency` | `string` | Always `"USD"`. |
| `amount_micro_usdc` | `integer` | Price of one `anchored` write, in micro-USDC (1 USDC = 1,000,000 micro-USDC). This is the exact price. |
| `amount_cents` | `integer` | The same price in US cents. The server rounds up, so a price above zero never shows as `0`. |
| `pricing_status` | `"live" \| "fallback" \| "disabled"` | The source of the price. Refer to the list below. |
| `payment_methods` | `string[]` | The payment methods that the server accepts: `["x402"]`, or `[]` when the server does not charge. |

**`pricing_status` values:**

- `live`: The last price refresh was successful. The price comes from current ArDrive Turbo storage quotes (converted to USD) and SOL/USDC quotes.
- `fallback`: The server has no current quote, or the last refresh failed. The price is the operator floor or the last good quote. The server still charges this price.
- `disabled`: The operator does not charge (`PAYMENT_MODE=none`). Both amounts are `0`.

The server calculates `anchored_cost` again for each `mnemonic_whoami` call.
Do not show an `anchored` write as free unless `pricing_status` is `disabled`.

`free_anchors` shows the free daily quota of the caller (available now). See
[Free daily quota](#free-daily-quota). The server adds the field only over HTTP
on a `PAYMENT_MODE=x402` deploy that supports `anchored`:

| Field | Meaning |
|---|---|
| `eligible` | `true` when the caller's key is linked to a Google account and the quota is on |
| `reason` | Why the caller (or this write) gets no free write. Absent when a free write is available. Refer to the list below |
| `link_hint` | How to link a Google account. Present only with `reason: "google_account_required"` |
| `per_day` | Free `anchored` writes per Google account per UTC (Coordinated Universal Time) day |
| `per_ip_per_day` | Free writes per client IP (Internet Protocol) address per UTC day |
| `max_bytes` | Largest signed envelope (COSE_Sign1 bytes) that a free write can carry |
| `remaining` | Free writes that the caller's Google account can still use today. Absent when `eligible` is `false` |
| `ip_remaining` | Free writes left today for the caller's IP address. Present only in HTTP 402 bodies, where the server knows the IP |
| `global_remaining` | Free writes left today for all accounts together. Absent when `eligible` is `false` |
| `resets_at` | Next UTC midnight, when all counters start again |

`remaining` is never more than `global_remaining` or `ip_remaining`.

**`reason` values:**

- `authentication_required`: the call has no JWT (JSON Web Token).
- `google_account_required`: the caller's key is not linked to a Google account.
- `account_quota_used`: the Google account used all its free writes today.
- `ip_quota_used`: the client IP address used all its free writes today.
- `global_quota_used`: the free writes of all accounts are used for today.
- `too_large`: the write is larger than `max_bytes`.
- `free_quota_disabled`: the operator set a limit to `0`.

---

## `mnemonic_sign_memory`

Legacy flow: operator receives plaintext, embeds and prepares canonical CBOR;
the client signs anchored bundles. Noncustodial signing is not client-private
preparation. Use [client-prepared ingestion](./client-prepared-memory.md) for
local encryption and signing before HTTP.

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `content` | `string` | yes | The text to attest |
| `tags` | `string[]` | no | Free-form tags, usable as recall filters |
| `mode` | `"local" \| "anchored"` | no | Per-request write intent. Omit to use the operator's `default_mode`. `"local"` is refused over HTTP (`-32010`) |

**This tool has two response shapes.** The write mode and the transport select
the shape. Only an `anchored` write gets a signature. A local write stores a
hash and signs nothing, so it never opens an operating system (OS) keychain
prompt.

### Inline stdio and HTTP mode restrictions

The server returns the finished attestation in these cases (available now):

- **Stdio, no JSON Web Token (JWT).** The operator key is the identity of the
  local agent. An `anchored` write gets a COSE_Sign1 signature from this key.
- **HTTP with `mode: "local"`.** Refused with `-32010 UnsupportedMode` and
  `supported: ["anchored"]`. `local` means the memory stays on your own machine,
  and a hosted server cannot do that: the memory would live in the operator's
  database instead. Install the server locally for free local storage, or use
  `anchored`.

Over HTTP, the operator key never signs a memory. A request without a JWT
cannot start an inline `anchored` write. A request without `mode` uses the
operator's `default_mode` and the deferred path.

```jsonc
{
  "attestation_id": "...",
  "content_hash": "<blake3 hex>",
  "hash_algorithm": "blake3",
  "encoding": "cbor+cose",
  "solana_tx": "<sig>",     // "local:..." in local mode
  "arweave_tx": "<tx id>",  // "local:..." in local mode
  "signer": "<base58>",     // the owner of the memory
  "signature": "cose_sign1", // "none" for a local hash-only row
  "did_sol": "did:sol:...", // DID of the owner
  "timestamp": "...",
  "storage_mode": "full",
  "write_mode": "anchored",
  "visibility": "public",       // always "public" for an anchored write
  "plaintext_on_arweave": true, // the content is plain text on Arweave
  "embedding": { "model": "...", "provider": "..." }
}
```

### Deferred (client-signed) — the non-custodial HTTP path

The server uses this path for each JWT write in `anchored` mode (available
now). It also uses this path for a JWT write without a `mode` field. The SDK and
the browser extension send no `mode` field and expect this shape. The operator
key never signs content from a different identity. Thus the server returns a
bundle for you to sign:

```jsonc
{
  "status": "awaiting_signature",
  "correlation_id": "<uuid>",
  "approve_url": "https://www.mnemonik.xyz/sign/<uuid>?mcp_base=<server origin>",
  "content_hash": "<blake3 hex>",
  "expires_in": 300
}
```

Complete it one of two ways:

1. **Browser** — open `approve_url`, approve, then poll
   [`mnemonic_check_pending`](#mnemonic_check_pending) with the `correlation_id`.
   The `mcp_base` parameter names the server that holds the bundle. Each
   hosted server (for example `mcp` and `mcp2`) keeps its own pending
   bundles. The sign page accepts only `mnemonik.xyz` hosts and loopback
   development servers.
2. **Headless** — sign the canonical-CBOR bundle locally with your Ed25519 key
   and `POST {correlation_id, signed}` to `/api/sign-callback`, then call
   `mnemonic_check_pending`. The SDK's `MnemonicClient.signMemory()` does this
   for you.

A pending signature does not establish external delivery. New deliveries do
not require Solana memos. Financial replay metadata may exist before delivery.
Bundles expire after 300 seconds.

---

## `mnemonic_check_pending`

Resolves a deferred-sign `correlation_id`. Poll after `awaiting_signature`.

**Input:** `correlation_id` (string, required).

**Returns** one of:

```jsonc
{ "status": "signed", "attestation_id": "...", "content_hash": "...",
  "solana_tx": "...", "arweave_tx": "...", "anchoring_network": "mainnet",
  "solana_explorer_url": "...", "arweave_url": "..." }

{ "status": "awaiting_signature" }   // user has not approved yet — keep polling
{ "status": "not_found" }            // never existed, or the 300s window expired
```

`solana_explorer_url` links to the Solana explorer. On devnet it adds `?cluster=devnet`.
`arweave_url` is `<gateway>/<id>` on the primary read gateway (`ARWEAVE_GATEWAY_URL`).
`anchoring_network` names the Solana network only. Every upload goes to Arweave mainnet.

---

## `mnemonic_recall`

Semantic search: the query is embedded with the same provider used at sign time,
then cosine-scored against available f32 embeddings in the server index.
HTTP recall does not open sealed content; client sealed recall is a separate
offline path. Metadata-only anchored receipts are not searchable memory rows.

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `query` | `string` | yes | Natural-language query, matched by meaning not keyword |
| `limit` | `integer` | no | Max results, default `5` |

**Auth changes what you can see** — this is the important part:

| Caller | Scope |
|---|---|
| Authenticated (JWT) | Searchable rows in your own corpus; rows without embeddings are excluded |
| Anonymous | The cross-owner **public** pool only (`visibility = 'public'`) |

**Private rows go only to their owner** (available now):

- An anonymous caller gets only rows with `visibility = 'public'`, from all
  owners. The server applies this filter in the SQL query.
- An authenticated caller gets only rows that the caller owns. The result
  includes private and public rows. It does not include rows of other owners.
- A row with no `visibility` value (a legacy row) counts as private. The
  database migration sets these rows to `private`.
- A local write is always private.
- An unencrypted externally stored memory is public plaintext. A sealed
  artifact contains ciphertext; external storage alone does not make it plaintext.

Returns the top-k rows ordered by cosine score, joined to their attestation
metadata.

Each result has a `plaintext_on_arweave` field. It is `true` when the
content went to Arweave as plain text.

**Sealed memories and `sealed_hidden`.** HTTP recall can report that legacy
sealed rows exist, but never decrypts them. Hosted recall-key sessions are retired.
Use client-side recovery and an explicit local embedder to open and rank sealed
memories. Anonymous callers never receive private sealed rows.

The current source supports client-prepared sealed external delivery. The legacy
`signMemory` preparation flow still sends plaintext to the operator. Historical
plaintext artifacts remain public even if their old row label said `private`.
Do not confuse ciphertext delivery with a promise that every client path seals.

---

## `mnemonic_verify`

Recompute the hash and check it against what was signed and anchored.

**Input:** `solana_tx` and/or `arweave_tx` (both optional strings — supply at
least one). Verification is version-aware; legacy SHA-256/JSON artifacts still
verify through a fallback path.

**Returns** a `status` of `verified`, `tampered`, `not_found`,
`anchor_not_found`, or `arweave_not_found`, alongside `content_hash`,
`solana_tx`, and `arweave_tx`. For a row that the caller owns, the result
also has `plaintext_on_arweave` (`true` when the content is plain text on
Arweave). A caller who does not own the row gets `not_found`, with no content.

**Sealed-aware.** For a sealed memory (`sealed: true`), the `content_hash`
is the hash of the `SEALED_V1` outer ciphertext. The verify result confirms
the COSE_Sign1 signature over the ciphertext and the on-chain hash match.
It does not decrypt the content.

Legacy artifacts with a Solana transaction additionally verify its memo.
New delivery verification checks the original signed bytes and expected author
without requiring a Solana memo or operator receipt row.

You do not need this server to verify a memory. See
[Verifying without Mnemonic](./QUICKSTART.md#5-verify-independently) for the
gateway-only recipe.

---

## `mnemonic_prove_identity`

Signs an arbitrary challenge with the server's Ed25519 key — identity proof with
no on-chain transaction and no stored artifact.

**Input:** `challenge` (string, required).

**Returns:** the signature over the challenge bytes plus the signing pubkey.

---

## `mnemonic_operator_proof`

Proves the operator's Ed25519 key without a token. Clients call it before they
send credentials to a selected operator. This tool is available now.

**Input:** `nonce` (string, required). The nonce is 32 random bytes as 64
lowercase hex characters.

**Signed message:** the server composes the message itself. The caller cannot
choose other bytes, so the tool does not sign arbitrary data.

```text
mnemonic.operator-selection.v1
<server public origin>
<nonce>
```

The lines are joined with `\n` and have no trailing newline. The origin is the
value of `MCP_PUBLIC_BASE_URL`.

**Returns:** `public_key` (base58), `origin`, `nonce`, `signature` (128 hex
characters) and `algorithm` (`Ed25519`).

A client must compare `public_key` with an independently obtained pin. It must
also verify the signature over the message with its own configured origin. An
operator with a different configured origin fails this check. See
[operator selection](./operator-selection.md).

---

## `mnemonic_publish_post`

Agent-native publishing: writes a blog post as a signed **public** attestation,
listed at `GET /blog`. Stored as a free `local` public attestation — no payment,
no on-chain anchoring in V1.

**Requires authentication** (OAuth2 Bearer or Ed25519).

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `title` | `string` | yes | Slugified into the URL. **The slug is the primary key** — republishing the same title replaces the post |
| `body_markdown` | `string` | yes | Markdown source; `content_hash` commits to this exact text |
| `tags` | `string[]` | no | |
| `author` | `string` | no | Human-readable display name. Distinct from the cryptographic signer (`producer`), which is always the caller's identity |

**Returns:** `{ slug, title, body_markdown, tags, author, attestation_id, content_hash, published_at }`.

---

## `mnemonic_share`

This legacy tool returns a browser approval URL using `memory_hash` and `reader`.
Its `awaiting_signature` response is only a UI handoff, not evidence that a grant
was signed, persisted or delivered. It does not establish a complete sharing flow.

New hosted `POST /api/grants` writes and SDK `share` are retired in the current
source migration. Existing grant reads and withdrawal remain available. Retain
and distribute signed grants client-side, or use sealed A2A recipient grants.
See [client-prepared memory](./client-prepared-memory.md) for compatibility limits.
A recipient who already has a content key can retain it; withdrawal cannot erase
that key or previously decrypted data.

---

## `request_public_write_confirmation`

**Not user-facing.** A ceremony gate that surfaces the `content_hash` about to be
anchored so a user can confirm or refuse before any chain write fires. Agent
skills invoke it inline before issuing a `mode: "anchored"` write with
`visibility: "public"`. Listed here only because it appears in `tools/list`.

**Input:** `content_hash` (string, required).

---

## Experimental: verifiable trajectories

⚗️ These three tools exist **only** when the server is built with the
`trajectory-experimental` cargo feature — `default = []` in `mcp/Cargo.toml`, so
stock builds and the hosted server do not advertise them. Check with
`tools/list` before depending on them.

```bash
cargo build --release -p mnemonic-mcp --features trajectory-experimental
```

All three are **strictly non-custodial**: the server verifies signatures and
linkage but signs nothing itself. The producer identity is always the COSE
signer.

### `mnemonic_attest_step`

Stores one ordered, hash-linked step in a trajectory.

**Input:** `signed` (string, required) — a hex-encoded COSE_Sign1 STEP envelope
signed by the producing agent's own key. `trajectory_id`, `seq`, and `prev_hash`
live *inside* the signed payload, not in the tool arguments. Build the envelope
with the SDK or `mnemonic_core::trajectory::build_step`.

The server verifies the signature and enforces dense `seq` plus `prev_hash`
linkage against the current trajectory head — a gap or a fork is rejected.

### `mnemonic_attest_verdict`

Records an **independent** judge's verdict over a step.

**Input:** `signed` (string, required) — a hex-encoded COSE_Sign1 VERDICT
envelope signed by the judge's own key. `step_hash`, `status`
(`pass` / `concern` / `reject`), and the optional `score` and `proof_ref` live
inside the signed payload.

The server enforces `judge != producer`; a self-issued verdict is rejected.

### `mnemonic_verify_trajectory`

**Input:** `trajectory_id` (string, required).

Verifies end-to-end and returns: chain integrity (ordered, hash-linked, signed),
verdict coverage by independent judges, the order-preserving batch root,
per-step inclusion proofs, and the `safe_to_settle` gate — which is true only
when the chain is valid **and** coverage is complete **and** no verdict is a
`reject`.

---

## Write modes: `local` vs `anchored`

`STORAGE_MODE` sets the operator's **capability and default**, not a global
switch. The write mode is a per-request user choice on `mnemonic_sign_memory`.

| | `local` | `anchored` |
|---|---|---|
| Storage | Agent-owned store (stdio/client) | External original signed bytes; hosted metadata receipts |
| Tx ids | Synthetic `local:...` for legacy stdio rows | External locator; Solana IDs only on legacy artifacts |
| Cost | Free | Priced by the operator (`anchored_cost` from `whoami`) |
| Signed by | Plain legacy rows unsigned; sealed SDK/A2A local artifacts signed | Author identity |
| Verifiable by third parties | Depends on the local artifact format | Signature and content hash; bounded availability observation |

Choosing one:

```jsonc
// stdio only: explicit local stays on the agent node
{ "name": "mnemonic_sign_memory",
  "arguments": { "content": "a private note", "mode": "local" } }

// explicit anchored — anchored and independently verifiable
{ "name": "mnemonic_sign_memory",
  "arguments": { "content": "a claim I want provable", "mode": "anchored" } }
```

Rules worth knowing:

- **Omitting `mode`** falls back to the operator's default (legacy clients keep
  working). Call `whoami` to read `default_mode`.
- **Asking for `anchored` on a local-only operator** returns a typed
  `UnsupportedMode` error listing `supported_modes` — it does not silently
  downgrade.
- **Delivery and payment are independent states.** Exact-byte external verification
  establishes delivery even if receipt persistence fails. Settlement can precede
  delivery; terminal paid failures require an explicit remedy state.
- **Local and external records may coexist**, but hosted metadata-only receipts
  do not provide plaintext semantic recall. Recover and search locally.
- **Public memory is plaintext; sealed memory is ciphertext.** Choose client
  preparation deliberately. Legacy server-prepared signing exposes input text.

Rationale and the full decision log: `work/modes-user-choice/user-spec.md` and
`work/modes-user-choice/decisions.md`; whitepaper §5.7.

---

## Payment

`PAYMENT_MODE` is `none` or `x402`; removed custodial `balance`/`both` modes
fail closed. Client-prepared memory and HTTP A2A delivery use one immutable
operation coordinator. Supported paid delivery uses the configured Universal
Paywall rail with a linked wallet, quote and exact envelope digest. Unsupported
rails fail before payment/upload. Legacy deferred signing has separate migration
compatibility; it must not be presented as client-private preparation.

Payment can settle before external delivery. Responses report payment and delivery
states separately. A settled retry reuses its operation identity and original
bytes; it must not create a second charge. Terminal delivery failure produces a
visible remedy state, not an automatic claim that the payment was refunded.
Read tools and `publish_post` do not charge.

### Free daily quota

On eligible paid deployments, a key linked to a Google account can use free
external writes within configured per-account, real-client-IP and global limits.
Defaults are 10/account/day, 20/IP/day, 1000/global/day and 16 KiB per envelope.
`whoami` reports applicable quota and denial reasons. The coordinator claims
quota after validation. A confirmed retry of the same original bytes does not
consume another allocation. A failed free delivery refunds the eligible quota;
the global upload budget may remain consumed once an upload was attempted.
Stdio and `PAYMENT_MODE=none` do not charge.

### Bounds and retries

Binary ingestion accepts at most 1 MiB of complete signed COSE bytes; free quota
has its smaller configured bound. Requests must retain original bytes through
wallet approval, payment and retry. Resubmitting different bytes under the same
operation ID is rejected. Hosted financial/replay metadata remains durable;
client-side resubmission replaces autonomous SQL artifact staging.

Use the source [ingestion guide](./client-prepared-memory.md) for SDK retry APIs,
structured HTTP 402/428 responses, and retired routes. Mocked tests do not establish
live payment-provider or published-package readiness.

---

## See also

- [Quickstart](./QUICKSTART.md) — install, identity, first signed memory
- [How it works](./how-it-works.md) — module-level walkthrough of the pipeline
- [`packages/cli/README.md`](../packages/cli/README.md) — every CLI command
- [`packages/sdk/README.md`](../packages/sdk/README.md) — TypeScript SDK + OAuth
- [AGENTS.md](../AGENTS.md) — agent-facing service card
- [Yellow paper](./YELLOWPAPER.md) — §5 architecture, §6 artifact model, §7 trust

## Client-signed A2A tools

`mnemonic_attest_a2a` accepts signed client envelopes and an optional `sealed`
flag. `mnemonic_recall_a2a` returns external delivery receipt metadata to
the author or a named grant recipient. See [Sealed A2A](./sealed-a2a.md) for the
complete input/output contract, SDK/CLI usage and streaming limits.


A2A anchored artifacts live on Arweave; hosted SQL keeps metadata receipts
only. Clients fetch and verify original signatures and open sealed data locally.
Use `prev_locator: "ar://..."` for external parent verification. Explicit hosted
`mode: "local"` is rejected; SDK/CLI local mode uses agent-owned storage.
