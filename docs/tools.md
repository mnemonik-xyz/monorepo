# MCP Tool Reference

Every tool the Mnemonic MCP server exposes over JSON-RPC 2.0, with its inputs,
outputs, and auth requirements. Source of truth: `mcp/src/mcp.rs`
(`tool_definitions()` for the schemas, the `call_tool` dispatch for behaviour).

**Endpoints**

| Transport | Address | Auth |
|---|---|---|
| HTTP | `POST https://mcp.mnemonik.xyz/mcp` (self-host: `http://localhost:3000/mcp`) | OAuth 2.1 + PKCE (`Authorization: Bearer <jwt>`) |
| stdio | `npx @mnemonik-xyz/cli mcp` | local keypair at `~/.mnemonic/identity.json` |

New to the protocol? Start with the [Quickstart](./QUICKSTART.md); this page is
the reference you come back to.

---

## Tool index

The default build advertises **8 tools**. Three more appear only when the server
is compiled with the `trajectory-experimental` cargo feature.

| Tool | Auth | Paid | Purpose |
|---|---|---|---|
| [`mnemonic_whoami`](#mnemonic_whoami) | required on HTTP | no | Server identity, storage capabilities, pricing |
| [`mnemonic_sign_memory`](#mnemonic_sign_memory) | required for `participate` | `participate` only | Create a signed memory attestation |
| [`mnemonic_check_pending`](#mnemonic_check_pending) | required | no | Resolve a deferred-sign `correlation_id` |
| [`mnemonic_recall`](#mnemonic_recall) | optional (changes scope) | no | Semantic search over stored memories |
| [`mnemonic_verify`](#mnemonic_verify) | required on HTTP | no | Verify an attestation against its chain anchors |
| [`mnemonic_prove_identity`](#mnemonic_prove_identity) | required on HTTP | no | Sign an arbitrary challenge with the server key |
| [`mnemonic_publish_post`](#mnemonic_publish_post) | required | no | Publish a signed public blog post |
| [`request_public_write_confirmation`](#request_public_write_confirmation) | — | no | Internal ceremony gate (not user-facing) |
| [`mnemonic_attest_step`](#mnemonic_attest_step) ⚗️ | required | no | Append a hash-linked trajectory step |
| [`mnemonic_attest_verdict`](#mnemonic_attest_verdict) ⚗️ | required | no | Record an independent judge's verdict |
| [`mnemonic_verify_trajectory`](#mnemonic_verify_trajectory) ⚗️ | required on HTTP | no | Verify a trajectory end-to-end |

⚗️ = experimental, behind `trajectory-experimental`.

The Auth column applies to the HTTP transport. The stdio transport uses the
local keypair and needs no token. Refer to
[Authentication over HTTP](#authentication-over-http).

Only `mnemonic_sign_memory` is ever charged, and only for `participate` writes on
an operator that has a payment mode enabled. Everything else is free.

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
- `tools/call` for `mnemonic_recall` (it searches the public pool only)

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
modes the operator supports and what a `participate` write costs, so a client can
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
  "supported_modes": ["local", "participate"],
  "default_mode": "local",
  "participate_cost": {            // null when the server cannot anchor (local only)
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
[Write modes](#write-modes-local-vs-participate).

**`participate_cost` fields:**

| Field | Type | Meaning |
|---|---|---|
| `currency` | `string` | Always `"USD"`. |
| `amount_micro_usdc` | `integer` | Price of one `participate` write, in micro-USDC (1 USDC = 1,000,000 micro-USDC). This is the exact price. |
| `amount_cents` | `integer` | The same price in US cents. The server rounds up, so a price above zero never shows as `0`. |
| `pricing_status` | `"live" \| "fallback" \| "disabled"` | The source of the price. Refer to the list below. |
| `payment_methods` | `string[]` | The payment methods that the server accepts: `["x402"]`, or `[]` when the server does not charge. |

**`pricing_status` values:**

- `live`: The last price refresh was successful. The price comes from current Irys and SOL/USDC quotes.
- `fallback`: The server has no current quote, or the last refresh failed. The price is the operator floor or the last good quote. The server still charges this price.
- `disabled`: The operator does not charge (`PAYMENT_MODE=none`). Both amounts are `0`.

The server calculates `participate_cost` again for each `mnemonic_whoami` call.
Do not show a `participate` write as free unless `pricing_status` is `disabled`.

`free_anchors` shows the free daily quota of the caller (available now). See
[Free daily quota](#free-daily-quota). The server adds the field only over HTTP
on a `PAYMENT_MODE=x402` deploy that supports `participate`:

| Field | Meaning |
|---|---|
| `eligible` | `true` when the caller's key is linked to a Google account and the quota is on |
| `reason` | Why the caller (or this write) gets no free write. Absent when a free write is available. Refer to the list below |
| `link_hint` | How to link a Google account. Present only with `reason: "google_account_required"` |
| `per_day` | Free `participate` writes per Google account per UTC (Coordinated Universal Time) day |
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

Embed → compress (TurboQuant) → canonical CBOR → blake3 → COSE_Sign1
(`participate` only) → persist.

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `content` | `string` | yes | The text to attest |
| `tags` | `string[]` | no | Free-form tags, usable as recall filters |
| `mode` | `"local" \| "participate"` | no | Per-request write intent. Omit to use the operator's `default_mode` |

**This tool has two response shapes.** The write mode and the transport select
the shape. Only a `participate` write gets a signature. A local write stores a
hash and signs nothing, so it never opens an operating system (OS) keychain
prompt.

### Inline — stdio, or an explicit local write over HTTP

The server returns the finished attestation in these cases (available now):

- **Stdio, no JSON Web Token (JWT).** The operator key is the identity of the
  local agent. A `participate` write gets a COSE_Sign1 signature from this key.
- **HTTP with a JWT and `mode: "local"`.** The server stores a hash-only row
  that the JWT subject owns. The client does not sign it. The operator key does
  not sign it.

Over HTTP, the operator key never signs a memory. A request without a JWT
cannot start an inline `participate` write. To write a local memory over HTTP
with no signing step, set `mode: "local"`. A request without `mode` uses the
deferred path.

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
  "write_mode": "participate",
  "visibility": "private",
  "embedding": { "model": "...", "provider": "..." }
}
```

### Deferred (client-signed) — the non-custodial HTTP path

The server uses this path for each JWT write in `participate` mode (available
now). It also uses this path for a JWT write without a `mode` field. The SDK and
the browser extension send no `mode` field and expect this shape. The operator
key never signs content from a different identity. Thus the server returns a
bundle for you to sign:

```jsonc
{
  "status": "awaiting_signature",
  "correlation_id": "<uuid>",
  "approve_url": "https://mnemonik.xyz/approve?...",
  "content_hash": "<blake3 hex>",
  "expires_in": 300
}
```

Complete it one of two ways:

1. **Browser** — open `approve_url`, approve, then poll
   [`mnemonic_check_pending`](#mnemonic_check_pending) with the `correlation_id`.
2. **Headless** — sign the canonical-CBOR bundle locally with your Ed25519 key
   and `POST {correlation_id, signed}` to `/api/sign-callback`, then call
   `mnemonic_check_pending`. The SDK's `MnemonicClient.signMemory()` does this
   for you.

No SQLite row, Arweave upload, or Solana memo exists until the callback lands.
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

---

## `mnemonic_recall`

Semantic search: the query is embedded with the same provider used at sign time,
then cosine-scored against the uncompressed f32 embeddings in SQLite. No chain
calls — recall is a local read.

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `query` | `string` | yes | Natural-language query, matched by meaning not keyword |
| `limit` | `integer` | no | Max results, default `5` |

**Auth changes what you can see** — this is the important part:

| Caller | Scope |
|---|---|
| Authenticated (JWT) | Your own corpus, across both visibilities and **both** `local` and `participate` writes |
| Anonymous | The cross-owner **public** pool only (`visibility = 'public'`) |

Private rows never surface to an anonymous caller, whoever owns them.

Returns the top-k rows ordered by cosine score, joined to their attestation
metadata.

---

## `mnemonic_verify`

Recompute the hash and check it against what was signed and anchored.

**Input:** `solana_tx` and/or `arweave_tx` (both optional strings — supply at
least one). Verification is version-aware; legacy SHA-256/JSON artifacts still
verify through a fallback path.

**Returns** a `status` of `verified`, `tampered`, `not_found`,
`anchor_not_found`, or `arweave_not_found`, alongside `content_hash`,
`solana_tx`, and `arweave_tx`.

The chain-anchored path additionally fetches the SPL Memo, parses its
`{h, a, v}` payload, and confirms the on-chain hash and Arweave tx match the
stored row — that is what makes the claim third-party checkable.

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

## `request_public_write_confirmation`

**Not user-facing.** A ceremony gate that surfaces the `content_hash` about to be
anchored so a user can confirm or refuse before any chain write fires. Agent
skills invoke it inline before issuing a `mode: "participate"` write with
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

## Write modes: `local` vs `participate`

`STORAGE_MODE` sets the operator's **capability and default**, not a global
switch. The write mode is a per-request user choice on `mnemonic_sign_memory`.

| | `local` | `participate` |
|---|---|---|
| Storage | Operator's SQLite only | Arweave bytes + Solana SPL Memo, plus SQLite |
| Tx ids | Synthetic `local:...` | Real `arweave_tx` / `solana_tx` |
| Cost | Free | Priced by the operator (`participate_cost` from `whoami`) |
| Signed by | Nobody (hash-only row, no keychain prompt) | The author: the client over HTTP, the local agent key over stdio |
| Verifiable by third parties | Hash only | Signature, hash, **and** independent on-chain timestamp |

Choosing one:

```jsonc
// explicit local — free, stays on the node
{ "name": "mnemonic_sign_memory",
  "arguments": { "content": "a private note", "mode": "local" } }

// explicit participate — anchored and independently verifiable
{ "name": "mnemonic_sign_memory",
  "arguments": { "content": "a claim I want provable", "mode": "participate" } }
```

Rules worth knowing:

- **Omitting `mode`** falls back to the operator's default (legacy clients keep
  working). Call `whoami` to read `default_mode`.
- **Asking for `participate` on a local-only operator** returns a typed
  `UnsupportedMode` error listing `supported_modes` — it does not silently
  downgrade.
- **A `participate` write only succeeds after the anchored bytes pass a
  recall+verify round-trip.** On failure the row is demoted to `local` and **no
  payment is charged**. "Delivered" means anchored *and* verified, never a
  silent receipt.
- **Both modes coexist in one database** for a single owner, tagged by the
  `write_mode` column, and `recall` spans both. Mixing them is by design.

Rationale and the full decision log: `work/modes-user-choice/user-spec.md` and
`work/modes-user-choice/decisions.md`; whitepaper §5.7.

---

## Payment

Payment applies only on HTTP, only in `full` mode, and only to
`mnemonic_sign_memory` `participate` writes. `PAYMENT_MODE` ∈ `none` |
`balance` | `x402` | `both`.

- `balance` — send `Authorization: Bearer mnm_<key>`. The balance is checked
  against the live pricing quote and reserved before execution.
- `x402` — the first request returns HTTP 402; retry with
  `X-Payment: {"tx_sig":"...","network":"solana-mainnet"}`.

`whoami`, `recall`, `verify`, `prove_identity`, `check_pending`, and
`publish_post` are always free.

### Free daily quota

Available now, on `PAYMENT_MODE=x402` over HTTP. An agent key that is linked to
a Google account gets free `participate` writes every UTC day before payment is
required. The agent needs no wallet and gets no payment prompt for these
writes. Paid writes (x402) do not need a Google account.

A free write must pass all of these checks:

1. **Google account.** The signer's Ed25519 key is linked to a Google account.
   The server reads the link from its `google_identity_links` table. The browser
   extension creates the link at Google sign-in (`POST /oauth/google/link`,
   with a possession proof for the key). All keys that link to one Google
   account share one quota. A key without a link, or a call without a JWT,
   gets no free write.
2. **Per-account quota.** The Google account has free writes left today.
3. **Per-IP share.** The client IP address of the agent that made the
   `mnemonic_sign_memory` call has free writes left today. The server groups
   IPv6 addresses by their /64 prefix.
4. **Global cap.** Free writes of all accounts together are below the daily
   cap. This cap limits the chain fees that the operator pays.
5. **Size.** The signed envelope (COSE_Sign1 bytes, which the server uploads to
   Arweave) is not larger than `MNEMONIC_FREE_ANCHOR_MAX_BYTES`. A typical
   memory of 1 KiB text makes an envelope of about 1.7 KiB.

The operator sets these limits:

| Variable | Default | Meaning |
|---|---|---|
| `MNEMONIC_FREE_ANCHORS_PER_DAY` | `10` | Free writes per Google account per UTC day. `0` disables the quota. |
| `MNEMONIC_FREE_ANCHORS_PER_IP_PER_DAY` | `20` | Free writes per client IP address per UTC day. `0` disables the quota. |
| `MNEMONIC_FREE_ANCHORS_GLOBAL_PER_DAY` | `1000` | Free writes per UTC day for all accounts together. `0` means no free writes. |
| `MNEMONIC_FREE_ANCHOR_MAX_BYTES` | `16384` | Largest COSE_Sign1 envelope, in bytes, for a free write. A larger write uses the paid path. |
| `TRUSTED_PROXIES` | loopback + private ranges | Reverse proxies whose `X-Forwarded-For` header gives the client IP address. |

The quota follows these rules:

- A free write uses the quota only when the anchor is confirmed. The quota
  counts the write when the client posts the signed bundle to
  `/api/sign-callback`. A bundle that expires unsigned uses nothing.
- If the write fails before the server starts a chain write, the server gives
  back all three counters.
- If the write fails after the server started the Arweave upload (for example,
  the delivery check fails and the row is demoted to `local`), the server gives
  back the account and IP counters. The global counter stays used, because
  the operator paid the chain fees.
- The server counts the IP address of the agent that parked the bundle. The IP
  address of the browser that posts the signature does not count.
- The server takes the client IP address from `X-Forwarded-For` only when the
  TCP (Transmission Control Protocol) peer is a trusted proxy. It uses the
  rightmost address that is not a trusted proxy. From any other peer, the
  server ignores `X-Forwarded-For` and `X-Real-IP`.
- A request with an `X-Payment` header uses the paid path. It does not use the
  quota.
- `PAYMENT_MODE=none` is free already and does not count writes. Stdio is
  never charged.
- All counters start again at UTC midnight.
- The server stores only hashes of Google account IDs and IP addresses in the
  counter table. The IP hash uses a secret salt.

When no free write is available, the payment-required responses add a
`free_anchors` block (the same shape as in `mnemonic_whoami`) with a
`reason`. This applies to the HTTP 402 of `mnemonic_sign_memory` and to the
wallet-link (HTTP 428) and payment (HTTP 402) steps of `/api/sign-callback`. The
block tells the agent why it must pay. A write that is too large gets HTTP 402
with `reason: "too_large"`, and the server keeps no parked bundle. If a parked
bundle loses its free write before the callback (for example, you parked more
bundles than you have free writes), `/api/sign-callback` returns HTTP 402 with
`status: "payment_required"`. The bundle stays parked. To pay, call
`mnemonic_sign_memory` again with `X-Payment`.

### Content size limit

Available now. `mnemonic_sign_memory` refuses `content` larger than
`MNEMONIC_MAX_CONTENT_BYTES` (default and maximum 32768 bytes) on every
transport and in both modes. The error is JSON-RPC (JSON Remote Procedure Call)
`-32602`. The HTTP server also limits the request body: 1 MiB on `/mcp` and
2 MB on `/api/sign-callback`.

### Replay protection

Available now. The server gives these guarantees:

- `/api/sign-callback` anchors one bundle at most once. A second post of the
  same `correlation_id` (also a concurrent one) gets HTTP 410 and uses no free
  write. A bundle expires after 300 seconds.
- If an artifact with the same `content_hash` is already anchored, the callback
  returns the existing anchor with `already_anchored: true`. It writes nothing
  new, charges nothing and uses no free write.
- One x402 payment (`X-Payment` transaction) pays for one call. The server
  reserves the payment before the call. Two concurrent calls with one payment
  cannot both succeed. A failed call releases the payment for a retry. EVM
  (Ethereum Virtual Machine) transaction hashes are compared in lowercase.

---|---|---|
| `MNEMONIC_FREE_ANCHORS_PER_DAY` | `10` | Free writes per key per UTC day. `0` disables the quota. |
| `MNEMONIC_FREE_ANCHORS_GLOBAL_PER_DAY` | `1000` | Free writes per UTC day for all keys together. `0` means no free writes. |

The global cap exists because a new key costs nothing to make. The quota
follows these rules:

- A free write uses the quota only when the anchor is confirmed. The quota
  counts the write when the client posts the signed bundle to
  `/api/sign-callback`. A bundle that expires unsigned uses nothing.
- If the delivery check fails, the server demotes the row to `local` and gives
  the free write back.
- A request with an `X-Payment` header uses the paid path. It does not use the
  quota.
- `PAYMENT_MODE=none` is free already and does not count writes. Stdio is
  never charged.
- Both counters start again at UTC midnight.

When no free write is left, the payment-required responses add a
`free_anchors` block (the same shape as in `mnemonic_whoami`). This applies to
the HTTP 402 of `mnemonic_sign_memory` and to the wallet-link (HTTP 428) and
payment (HTTP 402) steps of `/api/sign-callback`. The block tells the agent why
it must pay. If a parked bundle loses its free write before the callback (for example,
you parked more bundles than you have free writes), `/api/sign-callback`
returns HTTP 402 with `status: "payment_required"`. The bundle stays parked.
To pay, call `mnemonic_sign_memory` again with `X-Payment`.

---

## See also

- [Quickstart](./QUICKSTART.md) — install, identity, first signed memory
- [How it works](./how-it-works.md) — module-level walkthrough of the pipeline
- [`packages/cli/README.md`](../packages/cli/README.md) — every CLI command
- [`packages/sdk/README.md`](../packages/sdk/README.md) — TypeScript SDK + OAuth
- [AGENTS.md](../AGENTS.md) — agent-facing service card
- [Yellow paper](./YELLOWPAPER.md) — §5 architecture, §6 artifact model, §7 trust
