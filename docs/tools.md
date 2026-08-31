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
| [`mnemonic_whoami`](#mnemonic_whoami) | optional | no | Server identity, storage capabilities, pricing |
| [`mnemonic_sign_memory`](#mnemonic_sign_memory) | required for `participate` | `participate` only | Create a signed memory attestation |
| [`mnemonic_check_pending`](#mnemonic_check_pending) | required | no | Resolve a deferred-sign `correlation_id` |
| [`mnemonic_recall`](#mnemonic_recall) | optional (changes scope) | no | Semantic search over stored memories |
| [`mnemonic_verify`](#mnemonic_verify) | optional | no | Verify an attestation against its chain anchors |
| [`mnemonic_prove_identity`](#mnemonic_prove_identity) | optional | no | Sign an arbitrary challenge with the server key |
| [`mnemonic_publish_post`](#mnemonic_publish_post) | required | no | Publish a signed public blog post |
| [`request_public_write_confirmation`](#request_public_write_confirmation) | — | no | Internal ceremony gate (not user-facing) |
| [`mnemonic_attest_step`](#mnemonic_attest_step) ⚗️ | required | no | Append a hash-linked trajectory step |
| [`mnemonic_attest_verdict`](#mnemonic_attest_verdict) ⚗️ | required | no | Record an independent judge's verdict |
| [`mnemonic_verify_trajectory`](#mnemonic_verify_trajectory) ⚗️ | optional | no | Verify a trajectory end-to-end |

⚗️ = experimental, behind `trajectory-experimental`.

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
  "participate_cost": { /* null when the operator does not charge */ }
}
```

`storage_mode` reflects the operator's *capability*, not a global switch — see
[Write modes](#write-modes-local-vs-participate).

---

## `mnemonic_sign_memory`

Embed → compress (TurboQuant) → canonical CBOR → blake3 → COSE_Sign1 → persist.

**Input:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `content` | `string` | yes | The text to attest |
| `tags` | `string[]` | no | Free-form tags, usable as recall filters |
| `mode` | `"local" \| "participate"` | no | Per-request write intent. Omit to use the operator's `default_mode` |

**This tool has two response shapes**, decided by *who signs*:

### Inline (server-signed) — stdio / single-tenant

Taken only when the writer **is** the operator: the stdio path with no JWT, or a
JWT whose subject equals the operator's own pubkey. Returns the finished
attestation:

```jsonc
{
  "attestation_id": "...",
  "content_hash": "<blake3 hex>",
  "hash_algorithm": "blake3",
  "encoding": "cbor+cose",
  "solana_tx": "<sig>",     // "local:..." in local mode
  "arweave_tx": "<tx id>",  // "local:..." in local mode
  "signer": "<base58>",
  "did_sol": "did:sol:...",
  "timestamp": "...",
  "storage_mode": "full",
  "write_mode": "participate",
  "visibility": "private",
  "embedding": { "model": "...", "provider": "..." }
}
```

### Deferred (client-signed) — the non-custodial HTTP path

Taken for **every** JWT write owned by an identity other than the operator —
including an explicit `mode: "local"`. The operator's key never signs content
authored by someone else, so the server returns a bundle for you to sign:

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
| Verifiable by third parties | Signature + hash only | Signature, hash, **and** independent on-chain timestamp |

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

---

## See also

- [Quickstart](./QUICKSTART.md) — install, identity, first signed memory
- [How it works](./how-it-works.md) — module-level walkthrough of the pipeline
- [`packages/cli/README.md`](../packages/cli/README.md) — every CLI command
- [`packages/sdk/README.md`](../packages/sdk/README.md) — TypeScript SDK + OAuth
- [AGENTS.md](../AGENTS.md) — agent-facing service card
- [Whitepaper](./WHITEPAPER.md) — §5 architecture, §6 artifact model, §7 trust
