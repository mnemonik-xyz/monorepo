# Mnemonic Protocol — Quickstart

Five steps to go from zero to "my AI agent on Device A remembers something my AI agent on Device B asks for."

## Links

- Install: <https://mnemonik.xyz/install>
- Webapp: <https://mnemonik.xyz>
- MCP endpoint: <https://mcp.mnemonik.xyz/mcp>
- Discord: <https://discord.gg/ws6wruJj>
- Source: <https://github.com/mnemonik-xyz/monorepo>

## 1. Open the install page

Go to <https://mnemonik.xyz/install> in your everyday browser. This page is the single entry point — it creates your identity, hands it to your IDE, and finishes the OAuth handshake in one flow.

## 2. Create your identity

The page generates an Ed25519 keypair locally in your browser. The private key never leaves the device — it stays in browser storage. Your public key is your identity from now on, across every IDE, every device, every model.

Write down (or save to a password manager) the recovery hint the page shows you. That is how you re-import this identity onto another device later.

## 3. Install Mnemonic into the IDE of your choice

Still on the install page, pick your IDE — Cursor, Claude Desktop, Windsurf, or any other Model Context Protocol client. Click the install button for that IDE.

The page registers the Mnemonic MCP server with the IDE and runs the OAuth handshake using the keypair you just created. When you reopen the IDE, the Mnemonic tools appear in its tool list automatically.

That is it for setup.

## 4. Ask your AI agent to store something

In your IDE's chat, just talk to it. Tell it what you want remembered.

Examples that work:

> "Remember that we decided to ship version 0.2 next Tuesday, with Alex as the release owner."

> "Save this code review summary as a memory tagged 'reviews' and 'v0.2'."

> "Store our current architecture decision: we are choosing Postgres over MongoDB because of the reporting requirements."

The agent calls `mnemonic_sign_memory` for you. Behind the scenes, the text is embedded, signed with your Ed25519 key, anchored on Solana, and copied to permanent decentralized storage. You will see a confirmation with an attestation identifier and links to the on-chain record.

## 5. Recall it from anywhere — different IDE, different device

This is the part that is genuinely new.

Open a different IDE on a different device. Repeat steps 1 to 3 — the install page will let you re-import the same identity using the recovery hint from step 2. Once that identity is loaded, the new IDE is connected to your same memory store.

Ask the agent in plain language:

> "What did we decide about the version 0.2 release?"

> "Remind me of our database choice and why."

> "Pull up the code review notes I tagged 'v0.2'."

The agent calls `mnemonic_recall`. It searches by meaning, not keyword. It returns the memories you signed earlier, signed by you, verifiable by anyone, available wherever you bring your identity.

That is the whole loop: portable, attestable memory for AI agents, across any tool, any device.

---

Help and questions: <https://discord.gg/ws6wruJj>
