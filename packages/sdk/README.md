# @mnemonik-xyz/sdk

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
