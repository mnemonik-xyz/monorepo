# @mnemonik-xyz/sdk

> Project site: [mnemonik.xyz](https://mnemonik.xyz) · Hosted MCP: `https://mcp.mnemonik.xyz/mcp`

Runtime-agnostic JavaScript/TypeScript SDK for the Mnemonic Protocol. Wraps
the hosted MCP HTTP surface, the OAuth 2.1 + PKCE handshake, and COSE_Sign1
canonical-CBOR signing through a WASM-compiled `mnemonic-core`. Pure ESM,
no bundler required, no `node:*` imports — the same artifact loads under
Node 20+, Bun, Deno, and modern browsers.

## Recovery refactor in source

The current source adds [client-prepared memory](../../docs/client-prepared-memory.md),
[authenticated checkpoints and encrypted backups](../../docs/recovery-checkpoints.md), and
[replaceable A2A discovery](../../docs/sealed-a2a.md).
These interfaces require compatible client and operator builds; published-package and live-provider drills remain release gates.
Private sealing, opening and ranking run locally. Persist returned signed bytes and back up keys before discarding the client instance.
Hosted sealed storage, recall-key sessions and new general-memory grant writes are retired by this migration.
Existing A2A recipient grants retain their separate signed-artifact path.

## Quick start

```typescript
import { MnemonicClient, LocalSigner, Keypair } from '@mnemonik-xyz/sdk';
const kp = await Keypair.fromJSON(JSON.parse(localStorage.getItem('mnemonic.identity')!));
const client = new MnemonicClient({ baseUrl: 'https://mcp.mnemonik.xyz', signer: new LocalSigner(kp), jwt });
client.setKeypair(kp);
const saved = await client.sealMemory('hello', { mode: 'store', tags: ['demo'] });
// Persist saved.signedBytes and saved.outerCbor in your own durable store.
const opened = await client.openMemory(saved.outerCbor!);
```

`signMemory` is the legacy server-prepared path: it sends plaintext to the
operator before signing. Its write modes are:

- `mode: "local"`: HTTP operators reject this mode. Use agent-local storage;
  `sealMemory` with `mode: "store"` retains encrypted bytes in the client session cache.
- `mode: "anchored"`: the SDK uses the deferred pending-bundle /
  sign-callback flow. The server returns a `correlation_id`. The SDK gets
  the canonical-CBOR bundle, signs it locally (COSE_Sign1) and sends the
  envelope back. The server then anchors the memory on-chain.
- No `mode`: the server applies its default (legacy behavior). Bind a
  keypair before the call.

The SDK never re-encodes the CBOR in JS, so the byte-level content_hash
matches what the server stores.

### Load the private key only when a signature is necessary

Use `setKeypairProvider` in place of `setKeypair`. The SDK calls the
provider when an operation needs the identity key. Plain hosted `recall` and
`verify` do not call it; sealed local writes and opening/recalling sealed data do.
An explicit HTTP local write is rejected rather than stored on the operator.

```typescript
const client = new MnemonicClient({ baseUrl, signer: pubkeyOnlySigner, jwt });
client.setKeypairProvider(() => loadKeypairFromKeychain()); // called lazily, once
await client.recall('public research');                    // no private-key access
await client.signMemory('public claim', { mode: 'anchored' }); // loads the key
```

### Renew the session without a new login (issue #33)

The server issues an OAuth refresh token with each login (one-year rolling
lifetime). `loginWithIdentity` and `exchangeCodeForToken` return it as
`refreshToken`. `refreshAccessToken` exchanges it for a new JWT. This
needs no private key. The server rotates the refresh token on each use,
so always keep the new value.

```typescript
let refreshToken = saved.refreshToken;
client.setTokenRefresher(async () => {
  const r = await refreshAccessToken({ baseUrl, refreshToken });
  refreshToken = r.refreshToken; // persist it: the old one is now spent
  return r.jwt;
});
```

With a refresher, the client renews the JWT when it expires within 60
seconds. It also retries a call one time after a 401 or 403 response.

## Examples

### Sign + recall + verify

```typescript
import { MnemonicClient, LocalSigner, Keypair } from '@mnemonik-xyz/sdk';

const kp = Keypair.fromJSON(JSON.parse(identityJson));        // your stored keypair
const client = new MnemonicClient({
  baseUrl: 'https://mcp.mnemonik.xyz',
  signer: new LocalSigner(kp),
  jwt: storedJwt,
});
client.setKeypair(kp);

const { attestationId } = await client.signMemory(
  'Findings from market research: TAM ~$2B, top-3 competitors are X, Y, Z.',
  { tags: ['research', 'q2-2026'] },
);

const hits = await client.recall('what did I find about market research', { topK: 5 });
hits.forEach(h => console.log(`[${h.score.toFixed(2)}] ${h.attestationId}: ${h.content}`));

const status = await client.verify(attestationId);
//   { status: 'verified', signer: '6ZsT...3kQp' }      — happy path
//   { status: 'tampered', signer: '...', reason: ... } — content_hash mismatch
//   { status: 'not_found' }                            — unknown id or wrong tenant
```

### Headless OAuth (CI / serverless / agent)

For environments where you cannot open a browser, mint a JWT through the
webapp once and pass it to the SDK directly. The SDK does not validate the
signature client-side — the server rejects an invalid token on first use.

```typescript
import { MnemonicClient, LocalSigner, Keypair, parseJwtPayload } from '@mnemonik-xyz/sdk';

const claims = parseJwtPayload(process.env.MNEMONIC_JWT!);
//   { sub, exp, iat }   — throws AuthError on alg=none, expired, malformed.

const kp = Keypair.fromJSON(JSON.parse(process.env.MNEMONIC_IDENTITY!));
const client = new MnemonicClient({
  baseUrl: 'https://mcp.mnemonik.xyz',
  signer: new LocalSigner(kp),
  jwt: process.env.MNEMONIC_JWT,
});
client.setKeypair(kp);

await client.signMemory('CI run #1234 succeeded', { tags: ['ci', 'release'] });
```

### Interactive OAuth (browser, Chrome extension, custom flow)

The SDK exposes the OAuth primitives without binding to any host. Wrap them
with whatever redirect mechanism your runtime offers — `chrome.identity.launchWebAuthFlow`
in an extension, a popup window in a webapp, the loopback server `mnemonic
login` uses in `node:http`, etc.

```typescript
import { buildAuthorizeUrl, exchangeCodeForToken } from '@mnemonik-xyz/sdk';

const { url, state, sessionId } = buildAuthorizeUrl({
  baseUrl: 'https://mcp.mnemonik.xyz',
  clientId: 'my-app',
  redirectUri: 'https://my-app.example/oauth/callback',
});

window.location = url;                     // OR open a popup, OR launchWebAuthFlow

// ... after the redirect with ?code=&state= ...

const { jwt, expiresAt } = await exchangeCodeForToken({
  baseUrl: 'https://mcp.mnemonik.xyz',
  code: callbackParams.code,
  state: callbackParams.state,
  redirectUri: 'https://my-app.example/oauth/callback',
  sessionId,                               // returned by buildAuthorizeUrl, validated against stored state
});
```

State + verifier + redirectUri are bound at `buildAuthorizeUrl` time and
re-validated at `exchangeCodeForToken` time. A mismatch throws `AuthError`
before any HTTP request.

### LangChain / agent framework

```typescript
import { DynamicTool } from '@langchain/core/tools';
import { MnemonicClient, LocalSigner, Keypair } from '@mnemonik-xyz/sdk';

const kp = Keypair.fromJSON(JSON.parse(process.env.MNEMONIC_IDENTITY!));
const mnemonic = new MnemonicClient({
  baseUrl: 'https://mcp.mnemonik.xyz',
  signer: new LocalSigner(kp),
  jwt: process.env.MNEMONIC_JWT,
});
mnemonic.setKeypair(kp);

export const signMemoryTool = new DynamicTool({
  name: 'mnemonic_sign_memory',
  description: 'Persist a verifiable memory. Use for research findings, decisions, and any fact you want to recall later.',
  func: async (content: string) => {
    const { attestationId } = await mnemonic.signMemory(content);
    return `signed: ${attestationId}`;
  },
});

export const recallTool = new DynamicTool({
  name: 'mnemonic_recall',
  description: 'Search prior signed memories by semantic similarity.',
  func: async (query: string) => {
    const hits = await mnemonic.recall(query, { topK: 5 });
    return JSON.stringify(hits, null, 2);
  },
});
```

## Runtime targets

The SDK consumes the `mnemonic-core` Rust crate compiled to WebAssembly via
`wasm-pack`. Three targets were investigated in Task 1; `--target web` is
the only one that loads under all four Phase 1 runtimes without a bundler.

| `wasm-pack --target` | Output shape | Node 20 / 22 | Bun 1.3 | Deno 2.7 |
| --- | --- | --- | --- | --- |
| `web` | ESM, lazy `init()` via `import.meta.url` + `fetch`/`fs.readFile` | works | works | works |
| `nodejs` | CommonJS, `require('fs')` + `require('util')` | works | works | fails (no `default` export under Deno's CJS interop) |
| `bundler` | ESM with sync `import * as wasm from "./*.wasm"` | fails (`ERR_UNKNOWN_FILE_EXTENSION`) | fails (`__wbindgen_start` undefined) | works |

No `package.json` conditional exports are needed — the single `web`
artifact covers Node 20, Node 22, Bun, and Deno. Cloudflare Workers smoke
is deferred to pre-release; the same `--target web` artifact already
ships in the webapp. See `decisions.md` Task 1 for the full smoke matrix.

### Building the WASM artifact

```bash
bash packages/sdk/scripts/build-wasm.sh
```

Wraps `wasm-pack build core --target web --features wasm`, output goes to
`core/pkg-web/`. Output is gitignored.

**Prereqs:** `wasm-pack` ≥ 0.14 (`cargo install wasm-pack`). `binaryen`
(`wasm-opt`) is optional but recommended — when on PATH the build pipeline
runs an extra `wasm-opt -Oz --strip-debug --strip-producers` pass and the
shipped artifact is ~3.5 KB smaller. Install via `brew install binaryen`
(macOS) or `apt-get install binaryen` (Linux). If absent, the script logs a
notice and falls back to the wasm-pack default.

## API reference

All names below are re-exported from the package root.

### Client

- **`MnemonicClient`** — HTTP client with session-only default local caches.

  **Current source interfaces** (compatible builds required):
  - `whoami()` — server identity/capability check.
  - `signMemory(content, opts?)` — legacy server-prepared signing. The operator
    receives plaintext. The client verifies a sealed pending bundle before signing;
    that check does not remove the earlier plaintext disclosure.
  - `sealMemory(content, {mode, tags?})` — locally encrypt and sign. `store`
    retains a session cache without HTTP; `anchor` sends complete signed bytes
    to `/api/ingest-artifact` and independently fetch-compares delivery.
    Persist returned `signedBytes` and `outerCbor` for restart recovery.
  - `preparePublicMemory(content, opts?)` — prepare signed public MEMORY_V1 locally.
  - `ingestPreparedMemory(signedBytes, opts?)` — resubmit identical bytes with
    optional operation/payment headers; public publication requires `publicConsent`.
  - `openMemory(hashOrBytes)` — decrypt session-cached or restored outer bytes.
    This is not an independent signature verification API.
  - `share(...)` — explicit migration error before HTTP; new hosted general-memory
    grant creation is retired. Sealed A2A recipient grants remain supported.
  - `importLink(url)` and `listGrants()` — compatibility reads of existing material;
    these do not establish a new grant delivery path.
  - `recallSealed(query, {embedder, topK?})` — open and rank the local cache with
    an explicitly supplied local embedder. No fallback to hosted query embedding.
  - `retryA2ADelivery(id, opts?)` — resume a retained signed A2A artifact without
    generating a new signature or encryption randomness.
  - `recall(query, opts?)` — hosted searchable rows, preserving server evidence.
    It does not decrypt sealed data or turn receipt-only rows into memory content.
  - `verify(attestationId)` and `proveIdentity(challenge)`.

  **Setters:** `setJwt`, `setKeypair` or `setKeypairProvider` (necessary
  for signed writes and sealed operations), `setTokenRefresher`.

- **`SignMemoryOptions`** — `{tags?, mode?}`. **`WriteMode`** —
  `"local" | "anchored"`. **`SignMemoryResult.status`** — `stored`,
  `signed`, `pending` or `anchored`.
- **`SealMemoryOptions`** — `{mode: SealMode, tags?, embedder?}`.
  **`SealMode`** — `"anchor" | "store"`. **`SealMemoryResult`** — memory hash, original signed/outer bytes, and optional locator/receipt.
- **`OpenMemoryResult`** — `{content: string, innerJson: Uint8Array}`.
- **`ShareTarget`** — `{kid: string, x25519Pub: Uint8Array} | "link"`.
- **`ShareResult`** — `{type: "grant", grantCbor: Uint8Array} | {type: "link", url: string}`.
- **`GrantEntry`** — `{grantId, memoryHash, reader?, createdAt}`.
- **`SealedHit`** — `{memoryHash, similarity, content?}`.
- **`Embedder`** — `{embed(text: string): Promise<Float32Array>}`.
- **`RecallSealedOptions`** — `{topK?, embedder?}`.

### Signer

- **`Signer`** — interface for raw Ed25519 byte signers. Has a `pubkey`
  string and a `sign(bytes): Promise<Uint8Array>` method.
- **`LocalSigner`** — Phase 1 in-memory implementation. Wraps a `Keypair`
  and signs through WASM `sign_challenge`.

### Keypair

- **`Keypair`** — Ed25519 keypair wrapper. Static factories: `Keypair.generate()`,
  `Keypair.fromJSON(json)`, `Keypair.fromBackupString(json)`. Instance:
  `pubkey` getter, `toJSON()`, `toBackupString()`.
- **`KeypairJson`** — TypeScript type matching the webapp's
  `localStorage["mnemonic.identity"]` shape: `{secret: number[64], pubkey_base58: string}`.

### COSE

- **`coseSignPayload(canonicalCbor, keypairJson)`** — wraps server-built
  canonical-CBOR bytes in a COSE_Sign1 envelope. Used internally by
  `MnemonicClient.signMemory`; exported for advanced consumers.

### OAuth 2.1 + PKCE

- **`buildAuthorizeUrl({baseUrl, clientId, redirectUri, scope?})`** —
  builds a `/oauth/authorize` URL with PKCE S256 and stores the
  `{verifier, state, redirectUri, sessionId}` tuple for later validation.
- **`exchangeCodeForToken({baseUrl, code, state, redirectUri, sessionId})`**
  — validates `state` and `redirectUri` against the stored session and
  posts to `/oauth/token`. Returns `{jwt, expiresAt, refreshToken?}`.
- **`loginWithIdentity({baseUrl, clientId, keypair})`** — browserless
  login. Signs the server challenge with the local keypair. Returns
  `{jwt, expiresAt, sub, refreshToken?}`.
- **`refreshAccessToken({baseUrl, refreshToken, clientId?})`** — gets a
  new JWT with `grant_type=refresh_token`. Returns
  `{jwt, expiresAt, sub, refreshToken}`. Keep the rotated `refreshToken`.
  Throws `AuthError` when the server rejects the token.
- **`parseJwtPayload(jwt)`** — decodes a JWT payload without signature
  verification. Asserts `alg=HS256` (Decision 6), required claims, and
  fresh `exp`. Throws `AuthError` otherwise.
- **`generatePkceVerifier()`**, **`pkceChallenge(verifier)`**,
  **`randomState()`** — low-level PKCE primitives.
- **`pendingAuthSessions`** — module-level session store (TTL 10 min, FIFO
  cap 100). Use the helpers above; direct mutation is reserved for tests.

### Errors

All SDK errors extend `MnemonicError`. The CLI maps each subclass to a
documented exit code (Decision 10).

- **`UserError`** — caller-side bad input (CLI exit 1).
- **`ServerError`** — 5xx, network failure, malformed JSON (CLI exit 2).
  Carries an optional `status` field.
- **`IntegrityError`** — content_hash mismatch / verify=tampered (CLI exit 3).
- **`AuthError`** — 401 / 403, missing or expired JWT, OAuth state
  mismatch (CLI exit 4).
- **`MnemonicError`** — base class. Constructor runs every message
  through `redactJWT` so JWT-shaped substrings and 128-hex secrets never
  leak into stderr.
- **`redactJWT(input)`** — exported helper for downstream consumers.

### ERC-8004 reputation feedback (offline, no server required)

Four free functions for building and verifying `MNEMONIC_FEEDBACK_V1` documents
that commit Mnemonic reputation evidence to the ERC-8004 Reputation Registry on
Ethereum. No EVM signer, no gas handling, no network: the functions are pure and
run offline.

- **`prepareFeedback(input, opts)`** — builds the `MNEMONIC_FEEDBACK_V1` document,
  signs the payload with the caller's Ed25519 identity, computes
  `feedbackHash = keccak256(JCS(document))` and returns ABI-encoded `giveFeedback`
  calldata ready to send. See `docs/spec/erc8004-feedback-v1.md` for the full
  document format.
  - `input`: `{agentId, value, valueDecimals, clientAddress, feedbackUri, mnemonic, tags?, createdAt?, chainId?, registryAddress?, note?}`
  - `opts.signer`: any `Signer` — the Ed25519 identity that endorses the document.
  - Returns `PreparedFeedback`: `{document, documentJson, feedbackHash, payloadHash, onchain, preflight, warnings}`.

- **`verifyFeedbackDocument(input)`** — offline verifier. Re-derives both hashes,
  verifies the Ed25519 proof via `@noble/ed25519`, and optionally checks the
  `msg.sender == feedback.clientAddress` binding.
  - `input`: `{documentJson, onchainFeedbackHash?, onchainSender?}`
  - Returns `VerifyFeedbackResult`: `{valid, checks: {feedbackHash, payloadHash, ed25519, senderBinding}, error?}`.

- **`checkSelfPromotion(opts)`** — pre-flight guard. Offline: rejects zero address
  and bad EIP-55 checksum. Online (requires `rpcUrl`): calls `ownerOf`,
  `isApprovedForAll`, `getApproved` to detect controller ratings.
  - Returns `SelfPromotionCheck`: `{allowed, reason?, warnings}`.

- **`encodeGiveFeedback(args)`** — low-level ABI encoder for `giveFeedback(uint256,
  int128,uint8,string,string,string,string,bytes32)`. Used internally by
  `prepareFeedback`. Exported for custom calldata construction.

All four functions and their TypeScript interfaces are re-exported from
`@mnemonik-xyz/sdk`. The canonical document format and verification procedure are
specified in `docs/spec/erc8004-feedback-v1.md`. The reproducible fixture at
`test/fixtures/erc8004-feedback-v1.json` lets any third party verify
`feedbackHash` with `cast keccak` and no Mnemonic code.

```typescript
import { prepareFeedback, verifyFeedbackDocument } from '@mnemonik-xyz/sdk';

// Build the feedback document and calldata — offline, no server call.
const prepared = await prepareFeedback({
  agentId: '42',
  value: 9800,
  valueDecimals: 2,
  clientAddress: '0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B',
  feedbackUri: 'https://example.com/feedback/1.json',
  mnemonic: {
    schema: 'MEMORY_V1',
    attestation_id: 'mn_01j...',
    blake3: '<64 hex chars>',
    ed25519_pubkey: '<base58>',
  },
  tags: { tag1: 'quality', tag2: 'research' },
  createdAt: '2026-09-27T12:00:00Z',
  chainId: 1,
}, { signer });

// Upload prepared.documentJson to prepared.onchain.to (feedbackUri) BEFORE sending the tx.
// Then send the transaction:
// await wallet.sendTransaction({ to: prepared.onchain.to, data: prepared.onchain.data });

// Verify any MNEMONIC_FEEDBACK_V1 document — offline.
const result = await verifyFeedbackDocument({
  documentJson: prepared.documentJson,
  onchainFeedbackHash: prepared.feedbackHash,
  onchainSender: '0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B',
});
// result.valid === true, result.checks.senderBinding === 'verified'
```

## Golden COSE fixture

`test/fixtures/golden-cose.json` (and its checksum `golden-cose.sha256`)
is the byte-for-byte parity contract between Rust core's canonical CBOR +
COSE_Sign1 encoder and the SDK's WASM-driven `coseSignPayload`. The
fixture is generated by `core/tests/golden_fixtures.rs::emit_fixtures`
behind the `golden-fixtures` cargo feature.

Regenerate:

```bash
bash packages/sdk/scripts/regen-golden-fixtures.sh
```

CI re-runs the regenerator on every PR and fails if the checksum drifts.

## Backlog & roadmap

See [`work/mnemonic-cli/backlog.md`](../../work/mnemonic-cli/backlog.md)
for Phase 1.5 items (on-chain anchoring, billing, additional signer
backends, Cloudflare Workers smoke).

## License

Apache-2.0.

## Sealed A2A

All A2A attestations are signed locally. Use `sealed: { recipients }` on
`attestA2AMessage`/`attestA2AArtifact`, then `recallA2AContext(context, {sealed:true})`
and `openA2AAttestation(row, trustedAuthor)` on the recipient client. It returns `{payload, innerSigned}`;
`innerSigned` is the author's signature over the plaintext and recipients. A recipient
is `{card, trustedCardSigner}`; the card must carry a valid detached EdDSA JWS.
`verifyA2AAttestation(hex, trustedAuthor)` needs no private key. Optional
`sealed.chunkSize` seals a completed payload into a verified chunk chain.
See [the contract and threat notes](../../docs/sealed-a2a.md).


### A2A recovery (draft)

A2A artifacts are signed on the client and stored externally on Arweave.
Hosted MCP returns metadata receipts; clients fetch, verify and decrypt locally.
SDK `restoreA2AContext(context, {expectedAuthors, heads})` needs no MCP login.
Configure payload gateway and index URL separately (defaults: `https://arweave.net`
and `https://arweave.net/graphql`). Completeness
means verified ancestry to pinned heads, not exhaustive index enumeration.
Local mode uses an agent-owned index; SDK defaults to session-only memory.
CLI provides `a2a attest --mode local`, `a2a recall --mode local`, and
`a2a restore --context ID --authors KEY --heads HEAD`. Anchored non-root writes
accept `--prev-locator ar://ID`; configured operators also accept migrated
`blob://sha256` parent hints. See [Sealed A2A](../../docs/sealed-a2a.md) for limits and validation.
