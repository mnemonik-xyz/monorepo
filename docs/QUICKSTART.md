# Quickstart — 60 seconds to your first memory

> Three commands. No build. The hosted MCP server at `mcp.mnemonik.xyz` is free for the public beta.

## TL;DR

```bash
# 1. Open https://mnemonik.xyz/install in your browser
# 2. Click "Send to CLI" → copy the ticket UUID
# 3. Run:
npx @mnemonik-xyz/cli init --ticket <uuid>
npx @mnemonik-xyz/cli login
npx @mnemonik-xyz/cli sign "first memory"            # private, local write
npx @mnemonik-xyz/cli sign "public claim" --anchor   # signed + anchored on-chain
```

(If you prefer a standalone CLI-only keypair without webapp pairing, replace step 3's first command with `npx @mnemonik-xyz/cli init --standalone`.)

That's it. The first `sign` saves a private memory on the production Mnemonic server. The `--anchor` write is signed with your key and anchored on-chain, so anyone can verify it.

---

## Step-by-step

### 1. Set up your identity (10 seconds)

Two paths — pick one:

**Recommended — pair with the webapp** (keeps CLI + browser keypairs in sync, so signs from Cursor / Claude.ai / VS Code Just Work):

1. Open `https://mnemonik.xyz/install` in your browser.
2. Click **"Send to CLI"** → copy the ticket UUID it shows.
3. Run:

   ```bash
   npx @mnemonik-xyz/cli init --ticket <uuid>
   ```

   This imports the webapp's localStorage Ed25519 keypair into `~/.mnemonic/identity.json` (mode 0600). Same keypair on both sides → no `pending bundle owner mismatch` later.

**Standalone** — CLI-only, no webapp pairing (advanced; use only if you do not plan to use the webapp / IDE integrations):

```bash
npx @mnemonik-xyz/cli init --standalone
```

Generates a fresh Ed25519 keypair, also at `~/.mnemonic/identity.json` mode 0600. The key never leaves the host.

If `~/.mnemonic/identity.json` already exists, both modes refuse to overwrite. Pass `--force` to replace it (only if you have a backup — losing the key means losing access to memories signed under it).

### 2. Authenticate against the hosted server (10 seconds)

```bash
npx @mnemonik-xyz/cli login
```

The CLI signs the OAuth 2.1 + PKCE challenge with the Ed25519 keypair from step 1. No browser is necessary. The signature tells the server that the JSON Web Token (JWT) belongs to you. The CLI saves the JWT and a refresh token in `~/.mnemonic/token.json` (also mode 0600). Use `login --browser` to let the webapp sign the challenge instead.

You log in one time. The JWT expires after one hour, but the CLI renews it automatically with the refresh token. This renewal does not read your private key (available now, CLI 0.3.0).

### 3. Save your first memory (3 seconds)

```bash
npx @mnemonik-xyz/cli sign "first memory"
```

Output:

```
attestation_id: Qm9...
signed_at:      2026-05-02T10:00:00Z
status:         stored
write_mode:     local
content_hash:   <blake3 of your content>
```

The default write mode is `local`. The server keeps the memory for your identity only. There is no on-chain anchor and no charge. The CLI uses only your public key for this write, so the OS keychain does not show a prompt.

### 3b. Anchor a memory on-chain

```bash
npx @mnemonik-xyz/cli sign "public claim" --anchor
```

Output:

```
attestation_id: Qm9...
signed_at:      2026-05-02T10:00:00Z
status:         anchored
write_mode:     anchored
content_hash:   <blake3 of your content>
solana_tx:      <real Solana SPL Memo tx>      ← anchor on mainnet
arweave_tx:     <real Arweave tx>              ← bytes preserved
```

`--anchor` (alias `--anchored`) reads your private key and signs the memory locally. The server then anchors it on Arweave and Solana. This write can be paid. It is the only memory write that reads the private key.

An anchored memory:

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

Or, fully outside the Mnemonic ecosystem, anyone can verify an anchored (`--anchor`) memory using only public infrastructure:

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
client.setKeypairProvider(() => keypair); // called only when a signature is necessary

const note = await client.signMemory("first memory", { mode: "local", tags: ["demo"] });
console.log(note.attestationId, note.status);             // "stored", no key used

const claim = await client.signMemory("public claim", { mode: "anchored" });
console.log(claim.attestationId, claim.solanaTx, claim.arweaveTx);

const hits = await client.recall("first");
console.log(hits);
```

The SDK runs unmodified in Node 20+, Bun, Deno, and modern browsers. To renew the JWT without a new login, pass the refresh token from the login to `refreshAccessToken` in a `setTokenRefresher` callback. The SDK README shows the full example.

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
