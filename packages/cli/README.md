# @mnemonik-xyz/cli

> Project site: [mnemonik.xyz](https://mnemonik.xyz) · Hosted MCP: `https://mcp.mnemonik.xyz/mcp`

`mnemonic` — command-line interface for the Mnemonic Protocol. A thin
wrapper over [`@mnemonik-xyz/sdk`](../sdk/) that adds Node-only persistence
under `~/.mnemonic/`, an interactive PKCE loopback OAuth flow, and
TTY-aware output formatting. Same OAuth + COSE substrate as the
Cursor / VS Code / Claude.ai connectors and the webapp; the CLI just
swaps the renderer.

## Install

```bash
npm install -g @mnemonik-xyz/cli
```

Bun and Deno work too — Bun `bun install -g @mnemonik-xyz/cli`, Deno
`deno install -A -n mnemonic npm:@mnemonik-xyz/cli/bin/mnemonic.js`.
Requires Node ≥ 20 (or equivalent Bun / Deno).

## Quick start

**Recommended (paired with the webapp — keeps CLI + browser keypairs in sync):**

```bash
# 1. Open https://mnemonik.xyz/install in your browser
# 2. Click "Send to CLI" — copy the ticket UUID
# 3. Paste it back into the terminal:
mnemonic init --ticket <uuid>
mnemonic login
mnemonic sign "hello"
```

**Standalone (CLI-only, no webapp pairing):**

```bash
mnemonic init --standalone && mnemonic login && mnemonic sign "hello"
```

> **Why the mode flag?** Pre-0.1.6, `mnemonic init` (no flags) silently generated a fresh keypair that did NOT match the webapp's localStorage keypair. That caused "pending bundle owner mismatch" 403s on every browser-mediated sign. From 0.1.6 forward you must explicitly pick a mode.

## Commands

### `mnemonic init [--ticket <uuid> | --standalone] [--force]`

Set up the CLI identity at `~/.mnemonic/identity.json`. Pick exactly one mode:

- **`--ticket <uuid>`** (recommended) — redeem a one-time ticket from the webapp's "Send to CLI" button. The CLI imports the webapp's localStorage keypair, so future browser-mediated signs Just Work.

  ```bash
  $ mnemonic init --ticket 550e8400-e29b-41d4-a716-446655440000
  identity imported: /Users/you/.mnemonic/identity.json
  pubkey: 6ZsT...3kQp
  did:    did:sol:6ZsT...3kQp
  ```

- **`--standalone`** — generate a fresh, CLI-only keypair. The keypair will NOT match any browser localStorage; signs from Cursor / VS Code / Claude.ai will fail until you align the keys via `mnemonic identity import`. Pre-0.1.6 default behavior, opt-in for advanced use.

  ```bash
  $ mnemonic init --standalone
  identity created: /Users/you/.mnemonic/identity.json
  pubkey: 6ZsT...3kQp
  did:    did:sol:6ZsT...3kQp
  ```

`--force` overwrites an existing `identity.json`. Use with care — losing the previous keypair means losing access to memories signed under it. Run `mnemonic identity export --file backup.json` first.

### `mnemonic login [--browser | --token <jwt>] [--base-url <url>]`

Three modes:

- **Browserless** (default): the CLI signs the server challenge with the
  local keypair and gets a JSON Web Token (JWT). No browser is necessary.
  This mode reads the private key.
- **Browser** (`--browser`): the CLI opens your browser at
  `/oauth/authorize`. The webapp keypair signs the challenge. PKCE state
  and `redirect_uri` are validated before any token request.
- **Headless** (`--token <jwt>`): persist a pre-issued JWT. The token is
  parsed locally (alg=HS256, fresh `exp`, present `sub`) but not
  verified against the server.

The browserless and browser modes also save an OAuth refresh token in
`token.json` (available now). See [Session renewal](#session-renewal).

```bash
$ mnemonic login
login OK
sub: 6ZsT...3kQp
expires: 2026-04-29T18:32:11.000Z
```

### `mnemonic sign <content> [--anchor] [--public] [--tags <list>] [--base-url <url>]`

Save a memory. Content is read from the positional argument or — if
absent and stdin is piped — from stdin. Tags are comma-separated.

- **Default** encrypts and signs locally, then atomically saves ciphertext and
  original signed bytes in `~/.mnemonic/sealed/<author>/<hash>.json` (0600 on Unix).
  It requires a file-backed identity; it does not open the OS keychain or require
  a login token. Back up this folder and your identity separately.
- **`--anchor`** saves the same local record before sending its signed bytes to
  `/api/ingest-artifact`. The client checks the external bytes independently.
  It may read the OS keychain and require payment. Failed delivery retains the
  original bytes; retry them using SDK `ingestPreparedMemory`. Running `sign`
  again creates a new artifact and is not a payment-resume operation.
- **`--public --anchor`** explicitly selects the legacy plaintext preparation
  flow and permanent public disclosure. `--public` alone fails before HTTP:
  hosted local plaintext writes are no longer supported by this CLI.

```bash
mnemonic sign "private note" --tags=demo
mnemonic sign "sealed claim" --anchor
mnemonic sign "public claim" --public --anchor
```

These are source behavior changes, not a statement of hosted rollout.
See [client-prepared memory migration](../../docs/client-prepared-memory.md).

### `mnemonic open <hash|link> [--base-url <url>]`

Decrypt and print a sealed memory. Accepts either a content hash (hex)
returned by `mnemonic sign` and available in your local ciphertext folder,
or an existing legacy share link (URL with `#k=<base64url>`
fragment). Reads the private key. Local hash opening requires no login token
and checks the ciphertext BLAKE3 hash before decrypting. This raw ciphertext
check is not an author-signature verification API. Legacy links still use
the hosted reader while their existing blobs remain available.

```bash
$ mnemonic open 6c7f9b2a...
private note

$ mnemonic open "https://mcp.mnemonik.xyz/open/6c7f9b2a...#k=AAEC..."
private note
```

### `mnemonic share <hash> --link | --to <did|key> [--base-url <url>]`

Hosted grant creation is retired. This command returns a migration error before
network or key access. Distribute signed grants client-side or use the sealed
A2A recipient-grant workflow. Existing grants remain readable.

### `mnemonic grants [--base-url <url>]`

List access grants you have created. Uses only your public key.

```bash
$ mnemonic grants
2 grant(s):
  grant-001     6c7f9b2a...  2026-09-01T00:00:00Z  → (anonymous link)
  grant-002     deadbeef...  2026-09-02T00:00:00Z  → did:sol:6ZsT...
```

### `mnemonic recall <query> [--top-k <n>] [--tag <tag>] [--sealed] [--base-url <url>]`

Semantic recall over your stored memories. Default `--top-k` is 5;
`--tag` filters to a single tag. Uses only your public key.

`--sealed` fails before any network call. This CLI does not yet supply a local
sealed index and embedder. Use SDK `recallSealed` with an explicit caller-local
embedder and locally prepared memories. The private query is never sent by
this unsupported CLI path.

```bash
$ mnemonic recall "hello" --top-k=3
2 hit(s) of 14:
  01HX9F2KQ7  sim=0.987  [demo,test]  hello world
  01HX9F0YBZ  sim=0.812  [demo]       hello again
```

### `mnemonic verify <attestation_id> [--base-url <url>]`

Verify an attestation. Exit codes: `0` verified, `3` tampered, `1` not
found.

```bash
$ mnemonic verify 01HX9F2KQ7...
status: verified
signer: 6ZsT...3kQp
```

### `mnemonic whoami [--with-count] [--base-url <url>]`

Print the user's local identity and JWT — **client-side**, no server
call by default (Decision 14). `--with-count` adds an optional
`recall(' ', topK=0)` round-trip for a memory total.

```bash
$ mnemonic whoami
pubkey:       6ZsT...3kQp
did:          did:sol:6ZsT...3kQp
jwt sub:      6ZsT...3kQp
jwt issued:   2026-04-28T11:00:00.000Z
jwt expires:  2026-04-29T11:00:00.000Z
signer_match: yes
```

### `mnemonic prove [--challenge <hex>]`

Sign a challenge with the local key — entirely offline. Defaults to a
fresh 32-byte challenge if `--challenge` is omitted. Output can be
verified offline with the printed `pubkey` + Ed25519 verify.

```bash
$ mnemonic prove --challenge=00112233
pubkey:    6ZsT...3kQp
did:       did:sol:6ZsT...3kQp
challenge: 00112233
signature: 7f3a...c411
```

### `mnemonic erc8004 feedback` — build a `MNEMONIC_FEEDBACK_V1` document

Build a `MNEMONIC_FEEDBACK_V1` reputation feedback document, sign it with
your local Ed25519 identity, and emit the `giveFeedback` calldata ready to
send. **Entirely offline — no server call, no gas, no transaction.** The
private key is read from the local keystore (same as `mnemonic prove`).

```
mnemonic erc8004 feedback
  --agent-id <id>          ERC-8004 agent token ID (decimal string)
  --value <n>              Fixed-point score numerator (int128)
  --value-decimals <n>     Decimal places (0–18; default 2)
  --client-address <addr>  Your EVM address — MUST equal msg.sender of the tx
  --uri <url>              URI where you will host the document (feedbackUri)
  --attestation-id <id>    Mnemonic attestation_id of the cited memory
  --blake3 <hex>           blake3 hash of the cited attestation
  --tag1 <str>             Optional rating tag 1
  --tag2 <str>             Optional rating tag 2
  --note <text>            Optional note (max 280 chars)
  --rpc-url <url>          Optional RPC URL for the self-promotion pre-flight check
  --registry <addr>        Override the default registry address
  --chain-id <n>           Override chain ID (default 1)
  --out <path>             Write the document JSON to a file instead of stdout
  --json                   Machine-readable JSON output
```

The document must be uploaded to `--uri` **before** the transaction lands.
Send the tx with:

```bash
# viem
# const { hash } = await wallet.sendTransaction({ to, data })

# cast
$ cast send <to> <data> --private-key $KEY --rpc-url $RPC
```

Example:

```bash
$ mnemonic erc8004 feedback \
    --agent-id 42 \
    --value 9800 --value-decimals 2 \
    --client-address 0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B \
    --uri https://example.com/feedback/1.json \
    --attestation-id mn_01j... \
    --blake3 aaaa...aaaa \
    --tag1 quality --tag2 research

feedbackHash:    0xe16ec856...
calldata:        0x3c036a7e...
document:        { "schema": "MNEMONIC_FEEDBACK_V1", ... }

WARNING  self-promotion check skipped (no --rpc-url)
PREFLIGHT  send from: 0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B
PREFLIGHT  chain: 1
PREFLIGHT  registry: 0x8004BAa17C55a88189AE136b182e5fdA19dE9b63
```

Use `--json` for machine-readable output. Exit codes follow the standard table.

### `mnemonic erc8004 feedback-verify` — verify a `MNEMONIC_FEEDBACK_V1` document

Verify the hashes and Ed25519 proof of a hosted feedback document — entirely
offline. Optionally supply the on-chain `feedbackHash` and `msg.sender` to
confirm the on-chain binding.

```
mnemonic erc8004 feedback-verify
  --document <path|url>          Path to a local file or HTTPS URL of the document
  --feedback-hash <0x...>        On-chain feedbackHash to verify against (optional)
  --sender <0xaddr>              On-chain msg.sender to check against clientAddress (optional)
  --json                         Machine-readable JSON output
```

Example:

```bash
$ mnemonic erc8004 feedback-verify \
    --document /tmp/feedback-1.json \
    --feedback-hash 0xe16ec856f50f9686ab1cfb50eb4e547737f25ed75ee0e7709e5cff0314af61aa \
    --sender 0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B

valid:          yes
feedbackHash:   ok
payloadHash:    ok
ed25519:        ok (kid: 3F5qRPtK...)
senderBinding:  verified
```

Exit codes: `0` valid, `1` invalid (bad hash or proof), `2` network/parse error.

### `mnemonic identity import [--ticket <uuid> | --file <path>] [--force] [--base-url <url>]`

Import a keypair from either a webapp "Send to CLI" ticket (via
`/api/cli-bootstrap/redeem`) or a local JSON file in the same shape as
`~/.mnemonic/identity.json`. `--ticket` and `--file` are mutually
exclusive. Refuses to overwrite an existing identity unless `--force`.

```bash
$ mnemonic identity import --ticket 7c3f9b2a-...-...
identity imported: /Users/you/.mnemonic/identity.json
pubkey: 6ZsT...3kQp
did:    did:sol:6ZsT...3kQp
```

### `mnemonic identity export --file <path>`

Write the current `~/.mnemonic/identity.json` to `<path>` with file
mode 0600 (Windows: ACL restricted to the current user via `icacls`).
There is no clipboard option — clipboard leakage is a documented
security concern.

```bash
$ mnemonic identity export --file /tmp/k.json
identity exported: /tmp/k.json
pubkey: 6ZsT...3kQp
mode:   0600 (file permissions restricted to current user)
```

## Private key access

The CLI reads your private key for signing, encryption, or decryption.
When the key is in the OS keychain, a read can show a system prompt. The
public key in `~/.mnemonic/identity.json` is sufficient for all other
commands.

| Command | Reads the private key |
|---|---|
| `recall`, `verify`, `whoami` (also `--with-count`), `grants` | Never |
| `sign` (default sealed local write) | From file only — never from keychain |
| `sign --anchor` | Yes, to encrypt and anchor |
| `sign --public` without `--anchor` | Rejected before key access |
| `sign --public --anchor` | Yes, to sign the memory |
| `open` | Yes, to decrypt |
| `share` | Retired; rejected before key access |
| `recall --sealed` | Unsupported; rejected before network or key access |
| `login` (browserless), `prove`, `identity export` | Yes, to sign or export |

These rules also apply (available now, CLI 0.3.0):

- A command that does not read the key does not create an identity.
  With no identity, it tells you to run `mnemonic init`.
- The CLI does not move a file-stored key into the OS keychain. It also
  does not check the keychain entry before each command.
- The default `sign` saves encrypted ciphertext on your device.
  Use `--public --anchor` for the legacy plaintext path.

## Session renewal

The access JWT expires after one hour. You do not have to run
`mnemonic login` again when it expires (available now, CLI 0.3.0). Before
each server call, the CLI renews an expired token in this sequence:

1. It sends the saved refresh token to the server. This needs no private
   key and shows no prompt. The server rotates the refresh token on each
   use; one refresh token stays valid for one year.
2. It uses a newer `token.json` that another CLI process saved.
3. It does a browserless login with your identity key. The CLI does this
   automatically only when reading the key cannot show a prompt. This is
   true when the key is in a file, or when the command reads the key
   anyway (`sign --anchor`).

If all three fail, the command stops with one message: run
`mnemonic login` once. This occurs when a token has no refresh token,
because an older CLI or `login --token` saved it. It also occurs when the
server rejects the refresh token.

`recall` does not continue with an expired token. The server would answer
anonymously and return only the public memories, not your own memories.

If the server rejects a token that is not expired, the CLI renews the
token and retries the call one time.

## Output flags

Top-level flags are accepted before the subcommand.

| Flag | Effect |
| --- | --- |
| `--json` | Emit machine-readable JSON to stdout; hints / progress to stderr. |
| `--quiet` | Suppress non-essential stdout. Combine with `--json` for a single payload. |
| `--no-color` | Force plain text. Implicit when stdout is not a TTY. |

```bash
mnemonic --json sign "hello"
mnemonic --quiet --json recall "demo"
mnemonic --no-color whoami
```

## Exit codes

Per Decision 10. Tested in `packages/cli/test/`.

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | User error — bad input, missing identity, `verify` not_found. |
| `2` | Server / network error — 5xx, connection refused, malformed response. |
| `3` | Integrity failure — `verify <id>` returned `tampered`. |
| `4` | Auth error — 401 / 403, expired JWT, OAuth state mismatch. |

## Manual smoke checklist

Pre-release smoke flow lives in [`SMOKE.md`](./SMOKE.md). Run before
publishing a new version.

## License

Apache-2.0.

## Sealed A2A

```sh
mnemonic a2a attest --kind message --file message.json --context ctx --recipients-file recipients.json --chunk-size 16384
mnemonic a2a recall --context ctx --sealed --open-author TRUSTED_AUTHOR_PUBKEY --json
```

`recipients.json` is an array of `{card, trustedCardSigner}`. Signing and opening
use the local identity/keychain. The trusted author and card-signing keys must
come from your trust configuration. See [Sealed A2A](../../docs/sealed-a2a.md).


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
accept `--prev-locator ar://ID`. See `docs/sealed-a2a.md` for limits and validation.
