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

### `mnemonic login [--token <jwt>] [--base-url <url>]`

Two modes:

- **Interactive** (default): binds a one-shot `127.0.0.1:0` loopback
  server, opens your browser at `/oauth/authorize`, awaits the callback,
  and exchanges the code for a JWT. PKCE state and `redirect_uri` are
  validated before any token request.
- **Headless** (`--token <jwt>`): persist a pre-issued JWT. The token is
  parsed locally (alg=HS256, fresh `exp`, present `sub`) but not
  verified against the server.

```bash
$ mnemonic login
opening browser: https://mcp.mnemonik.xyz/oauth/authorize?...
login OK
sub: 6ZsT...3kQp
expires: 2026-04-29T18:32:11.000Z
```

### `mnemonic sign <content> [--tags <list>] [--base-url <url>]`

Sign a memory. Content is read from the positional argument or — if
absent and stdin is piped — from stdin. Tags are comma-separated.

```bash
$ mnemonic sign "hello world" --tags=demo,test
attestation_id: 01HX9F2KQ7...
signed_at:      2026-04-28T11:14:22.901Z
status:         signed
content_hash:   blake3:6c7f...
```

### `mnemonic recall <query> [--top-k <n>] [--tag <tag>] [--base-url <url>]`

Semantic recall over your stored memories. Default `--top-k` is 5;
`--tag` filters to a single tag.

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
