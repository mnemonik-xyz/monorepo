# @mnemonik-xyz/sdk

> Project site: [mnemonik.xyz](https://mnemonik.xyz) · Hosted MCP: `https://mcp.mnemonik.xyz/mcp`

Runtime-agnostic JavaScript/TypeScript SDK for the Mnemonic Protocol. Wraps
the hosted MCP HTTP surface, the OAuth 2.1 + PKCE handshake, and COSE_Sign1
canonical-CBOR signing through a WASM-compiled `mnemonic-core`. Pure ESM,
no bundler required, no `node:*` imports — the same artifact loads under
Node 20+, Bun, Deno, and modern browsers.

## Quick start

```typescript
import { MnemonicClient, LocalSigner, Keypair } from '@mnemonik-xyz/sdk';
const kp = await Keypair.fromJSON(JSON.parse(localStorage.getItem('mnemonic.identity')!));
const client = new MnemonicClient({ baseUrl: 'https://mcp.mnemonik.xyz', signer: new LocalSigner(kp), jwt });
client.setKeypair(kp);
const { attestationId } = await client.signMemory('hello', { tags: ['demo'] });
```

`signMemory` always uses the deferred pending-bundle / sign-callback flow:
the server returns a `correlation_id`, the SDK fetches the canonical-CBOR
bundle, COSE-signs it locally, and POSTs the envelope back. The SDK never
re-encodes the CBOR in JS, so the byte-level content_hash matches what the
server stores.

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

- **`MnemonicClient`** — stateless HTTP client for the hosted MCP server.
  Methods: `whoami()`, `signMemory(content, opts?)`, `recall(query, opts?)`,
  `verify(attestationId)`, `proveIdentity(challenge)`. Setters: `setJwt`,
  `setKeypair` (required before `signMemory`).

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
  posts to `/oauth/token`. Returns `{jwt, expiresAt}`.
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
