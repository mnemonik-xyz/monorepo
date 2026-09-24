# Mnemonic use cases

Mnemonic gives an AI agent a memory that is **yours** (signed with your own
key), **portable** (works across Claude, Cursor, VS Code) and **provable**
(anyone can check who wrote it and that it was not changed).

Each use case says what works **today** and what is **coming** (tracked in
`work/presentable-mvp/plan.md`).

Legend: ✅ works today · 🔜 planned (P0/P1) · 🧪 later (P2)

---

## 1. One memory across all your AI tools ✅

**Who:** a developer using more than one AI assistant.

**Problem:** you explain your project conventions to Claude, then again to
Cursor, then again tomorrow.

**How it works:** every tool on your machine talks to the same local
Mnemonic memory (`~/.mnemonic/attestations.db`). Local memory is free, works
offline and never leaves your machine.

**Try it (2 min):**
1. `npm i -g @mnemonik-xyz/mcp && mnemonik-mcp install` (today this also
   needs Node 20+ and the GitHub `gh` CLI; P0 removes the `gh` requirement)
2. In Claude Code: *"Remember: in this repo we use snake_case for API fields."*
3. Open Cursor: *"What naming do we use for API fields?"* → the agent recalls it.

---

## 2. Project memory that survives the session ✅

**Who:** anyone doing multi-day work with an agent.

**Problem:** a new chat forgets yesterday's decisions.

**Try it:** end a session with *"Save the decisions we made today."* Start a
new session tomorrow: *"What did we decide yesterday about the database?"*

---

## 3. Share a memory with a teammate by link 🔜 (P1)

**Who:** two people (or two agents) working on the same thing.

**Problem:** copy-pasting context loses where it came from and can be
edited silently.

**How it will work:** *"Share the deployment checklist with Anna."* Your agent
signs it with your key and returns a link like `mnemonik.xyz/m/…`. Anna opens
the link and sees the text, your identity, and a **"Signature valid"** badge
checked in her browser. No account needed.

**Try it (when shipped):**
1. You: *"Share my memory about the deployment checklist."* → link.
2. Anna opens the link → sees author + valid signature.
3. Anna's agent: *"Import this memory: <link>."* → her agent recalls it,
   labelled *"from did:sol:…"*.

---

## 4. Verify where a piece of knowledge came from 🔜 (P1)

**Who:** reviewers, auditors, anyone receiving agent output.

**Problem:** you cannot tell whether a note was written by the person it
claims, or changed afterwards.

**How it will work:** paste a link or share code into any agent with
Mnemonic (or open the web page). You get: author identity, whether the
content matches the signature, and (if anchored) the public timestamp.
No login.

---

## 5. Timestamp proof: "I knew this on date X" 🔜 (P0 free quota)

**Who:** researchers, founders, anyone who needs prior-art style proof.

**How it works:** *"Remember this and anchor it on-chain."* The memory's hash
is recorded on Solana and the signed bytes on Arweave (permanent storage).
Anyone can later check the hash and date.

**Today:** works, but needs a USDC-funded wallet. **Planned:** the first
anchored memories are free, so you can try it without a wallet.

---

## 6. Handoff between agents in a pipeline 🔜 (P1)

**Who:** builders of multi-agent systems.

**Example:** a research agent saves findings; a writing agent imports them
and knows exactly which agent produced each fact, and that none was
altered in transit.

---

## 7. Team knowledge space 🧪 (P2)

**Who:** a small team sharing one growing memory.

**Idea:** a named space (`team-alpha`) that members' agents write to and
recall from. Built only if P1 shows people want it.

---

## What each option costs

| Action | Cost | Needs account? | Needs wallet? |
|---|---|---|---|
| Save / recall locally | Free | No | No |
| Share by link (P1) | Free | No (your key signs) | No |
| Verify a link (P1) | Free | No | No |
| Anchor on-chain | Free quota (P0), then ~$0.001 | Yes | Only after quota |

## Glossary

- **Agent:** an AI assistant that can call tools (Claude, Cursor, …).
- **MCP (Model Context Protocol):** the standard those assistants use to
  call tools like Mnemonic.
- **Signature / key:** a private key on your machine (in the OS keychain)
  proves that you wrote a memory. Mnemonic asks to unlock it only when you
  share or anchor.
- **DID (Decentralized Identifier):** your public identity, e.g.
  `did:sol:…`. Safe to show.
- **Anchor:** recording a memory's fingerprint (hash) on a public
  blockchain so its existence and date can be checked by anyone.
- **Arweave / Solana:** permanent storage network / public blockchain used
  for anchoring.
- **USDC:** a US-dollar stablecoin used for paid anchoring.
