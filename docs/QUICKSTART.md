# Quickstart — 60 seconds to your first signed memory

> Three commands. No build. The hosted MCP server at `mcp.mnemonik.xyz` is free for the public beta.

## TL;DR

```bash
npx @mnemonik-xyz/cli init
npx @mnemonik-xyz/cli login
npx @mnemonik-xyz/cli sign "first memory"
```

That's it. You now have a verifiable, persistent memory anchored against the production Mnemonic server.

---

## Step-by-step

### 1. Generate your identity (5 seconds)

```bash
npx @mnemonik-xyz/cli init
```

Creates an Ed25519 keypair at `~/.mnemonic/identity.json` (mode 0600 — only readable by you). This key proves *you* signed every memory; it never leaves the host.

If `~/.mnemonic/identity.json` already exists, the command refuses to overwrite. Pass `--force` to replace it (only if you have a backup — losing the key means losing access to memories signed under it).

### 2. Authenticate against the hosted server (10 seconds)

```bash
npx @mnemonik-xyz/cli login
```

Opens your default browser to the OAuth 2.1 + PKCE authorization page at `mcp.mnemonik.xyz`. Approve in the browser, return to the terminal — the CLI captures the JWT and persists it at `~/.mnemonic/token.json` (also mode 0600).

The browser-side step is required because the OAuth challenge is signed with the same Ed25519 keypair you just created — that's how the server knows the JWT belongs to you.

### 3. Sign your first memory (3 seconds)

```bash
npx @mnemonik-xyz/cli sign "first memory"
```

Output:

```
attestation_id: Qm9...
signed_at:      2026-05-02T10:00:00Z
status:         signed
content_hash:   <blake3 of your content>
solana_tx:      <real Solana SPL Memo tx>      ← anchor on mainnet
arweave_tx:     <real Arweave tx>              ← bytes preserved
```

You now have a memory that:

- Anyone can semantically search.
- Anyone with the `solana_tx` can independently verify (no Mnemonic-server dependency).
- Will outlive the laptop you signed it on.

### 4. Recall semantically (1 second)

```bash
npx @mnemonik-xyz/cli recall "memory"
```

Returns the top-k most similar attestations by cosine similarity — *not* keyword match.

### 5. Verify (independently)

```bash
npx @mnemonik-xyz/cli verify <attestation_id>
```

Or, fully outside the Mnemonic ecosystem, anyone can verify your memory using only public infrastructure:

```bash
# Fetch raw COSE bytes from Arweave (any gateway)
curl -sS "https://arweave.net/<arweave_tx>" -o memory.cose

# Recompute the hash + verify the COSE signature
# (sample verifier code in packages/sdk/src/verify.ts)

# Confirm Solana SPL Memo tx contains the same hash
solana confirm <solana_tx> --url https://api.mainnet-beta.solana.com
```

The point of the protocol is that your claim ("I signed this memory at this time") is third-party-verifiable — no central authority required.

---

## From an MCP-aware AI client (one click)

Mnemonic is exposed over the Model Context Protocol, which means Claude, Cursor, VS Code, and Windsurf can use it directly as a memory backend with no glue code.

Install via the one-click connector:

- **mnemonik.xyz/install** — picks the right deeplink for your client.

After install, the client will run the same OAuth handshake on first call, and from there every chat turn can call `mnemonic_sign_memory`, `mnemonic_recall`, and the rest as normal MCP tools.

---

## From an SDK (TypeScript / Node / Bun / Deno / browsers)

```ts
import { MnemonicClient, LocalSigner, Keypair } from "@mnemonik-xyz/sdk";

const keypair = Keypair.generate();                      // or load from disk
const signer = new LocalSigner(keypair);
const client = new MnemonicClient({ baseUrl: "https://mcp.mnemonik.xyz", signer });

// (Run OAuth flow elsewhere, persist the JWT, then:)
client.setJwt(jwtFromOauth);
client.setKeypair(keypair);

const result = await client.signMemory("first memory", { tags: ["demo"] });
console.log(result.attestationId, result.solanaTx, result.arweaveTx);

const hits = await client.recall("first");
console.log(hits);
```

The SDK runs unmodified in Node 20+, Bun, Deno, and modern browsers.

---

## Self-host (later, optional)

The hosted server is free. If you'd rather run it yourself:

```bash
git clone https://github.com/mnemonik-xyz/monorepo.git
cd monorepo
cargo build --release -p mnemonic-mcp --features local-embed

STORAGE_MODE=local PAYMENT_MODE=none \
  ./target/release/mnemonic-mcp --transport http --port 3000
```

`STORAGE_MODE=local` keeps everything on disk (no chain, no payment). Flip to `STORAGE_MODE=full` once you have a funded keypair to anchor on Solana mainnet.

Full self-host docs: `.claude/skills/project-knowledge/references/deployment.md`.

---

## Help

- **Discord:** [discord.gg/ws6wruJj](https://discord.gg/ws6wruJj)
- **Issues:** [github.com/mnemonik-xyz/monorepo/issues](https://github.com/mnemonik-xyz/monorepo/issues)
- **Whitepaper:** [docs/WHITEPAPER.md](./WHITEPAPER.md)
- **How it works:** [docs/how-it-works.md](./how-it-works.md)
