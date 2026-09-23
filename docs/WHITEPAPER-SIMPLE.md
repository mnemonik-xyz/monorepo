# Mnemonic: Verifiable Memory for AI Agents (Simplified Whitepaper)

**Version:** 0.3-simple
**Date:** September 2026
**Full version:** [WHITEPAPER.md](./WHITEPAPER.md)

## How to read this document

We wrote this document in Simplified Technical English (STE). STE is the
ASD-STE100 writing standard. It uses short sentences, the active voice and
simple words with one meaning.
Reference: <https://en.wikipedia.org/wiki/Simplified_Technical_English>.

Each section tells you if a function **is available now** or **is planned**.

---

## 1. Summary

An AI agent learns facts, preferences and decisions during its work.
Usually, the agent loses this knowledge when the session stops.
When the agent keeps it, the knowledge stays in one tool or one company.
Other people cannot check who wrote the knowledge or if someone changed it.

Mnemonic gives an agent a memory with three properties:

1. **Yours.** Your own key signs your memories. The server does not sign them.
2. **Portable.** The same memory works in Claude, Cursor, VS Code and other tools.
3. **Provable.** Anyone can check who wrote a memory and that nobody changed it.

Mnemonic uses the Model Context Protocol (MCP). MCP is the standard that
AI assistants use to call external tools.

---

## 2. The problem

Today, agents keep memory in three ways:

| Method | Good | Bad |
|---|---|---|
| Context window (the text the model sees now) | Fast | Lost at the end of the session |
| Memory inside one application | Stays between sessions | Locked in that application |
| Vector database (search by meaning) | Large and searchable | No proof of author or date |

These methods cause three problems:

1. **No portability.** When you change tools, the agent forgets.
2. **No proof.** A person can change a stored memory. Other systems cannot see the change.
3. **Unsafe reuse.** A stored memory can contain hidden instructions.
   When an agent reads that memory, the agent can obey those instructions.
   This attack is "prompt injection".

---

## 3. What a memory is

A Mnemonic memory is a small record. It has these parts:

| Part | What it contains |
|---|---|
| Content | The text of the memory |
| Tags | Optional labels that you choose |
| Embedding | A list of numbers that shows the meaning of the text. Search uses it. |
| Parent | The fingerprint of the previous memory, if there is one |
| Fingerprint | A short unique code calculated from the record (a "hash") |
| Signature | Proof that the owner's key approved this exact record |

**The embedding is not the truth.** Different AI models make different
embeddings. The text content is the permanent source of truth. You can
calculate a new embedding at any time.

---

## 4. How Mnemonic saves a memory

Mnemonic does these steps in this sequence:

1. **Embed.** Calculate the embedding of the text.
2. **Compress.** Make the embedding smaller with TurboQuant (2, 3 or 4 bits per number).
3. **Build.** Put the content, tags, compressed embedding and parent in one record.
4. **Encode.** Write the record in canonical CBOR.
   CBOR (Concise Binary Object Representation) is a binary data format.
   "Canonical" means that the same record always gives the same bytes.
5. **Hash.** Calculate the fingerprint with the BLAKE3 hash function.
6. **Sign.** Sign the record with the owner's Ed25519 key.
   The result is a COSE_Sign1 envelope.
   COSE (CBOR Object Signing and Encryption) is a standard format for signed data.
7. **Store.** Keep the record in local storage. Optionally, anchor it (see section 6).

Status: steps 1–7 are **available now**. The core library supports the parent
link, but the MCP tool does not set it yet.

**Important:** for a private local memory, Mnemonic does steps 1–5 and stores
the fingerprint. It does step 6 only when you publish the memory. Thus, local
work does not ask you to unlock your key.
Status: available after pull request #224 merges.

---

## 5. Three memory modes

You choose the mode for each memory. The same key and the same tools serve all modes.

| Mode | Where Mnemonic keeps it | Who can read it | Cost | Status |
|---|---|---|---|---|
| `local` | A database on your computer | Only you | Free | Available now |
| `sealed` | Encrypted on Arweave. Fingerprint on Solana. | You and the readers that you approve | Free quota, then paid | Planned |
| `public` | Arweave. Fingerprint on Solana. | Anyone | Free quota, then paid | Planned (today: "participate" mode, not encrypted) |

**Free quota.** Each identity can anchor 100 memories per week free of charge (planned).
After the quota, one anchored memory costs approximately 0.001 US dollars.

**Local memory is always free.** Mnemonic never charges money for local work
or for verification.

---

## 6. Anchoring: proof of date

Your signature proves **who** wrote a memory. It does not prove **when**.
Anchoring adds a public proof of date.

When you anchor a memory:

1. Mnemonic uploads the signed record to **Arweave**.
   Arweave is a network for permanent storage.
2. Mnemonic writes the fingerprint to **Solana** in a short memo.
   Solana is a public blockchain. Each block has a public time.
3. Mnemonic reads the record back from Arweave and verifies it.
   Mnemonic reports "delivered" only after this check passes.
   If the check fails, Mnemonic keeps the memory as `local` and does not charge you.

Status: available now ("participate" mode).

**You cannot delete anchored data.** Arweave calls itself "permanent information
storage" (<https://www.arweave.org/>). Arweave node operators can filter data
with their own content policies
(<https://docs.arweave.org/developers/llms-full.txt>).
This is not a delete that the author controls. Thus, do not anchor data that
you can possibly want to remove.

---

## 7. Who signs

**Rule: the Mnemonic server never signs your memories or your posts.**

- **Local MCP (on your computer).** It uses your own key. Your key stays in the
  keychain of your operating system (OS). Mnemonic asks you to unlock the
  keychain only when a memory must be signed: publish, anchor or prove identity.
- **Hosted MCP (on the Mnemonic server).** Your client signs the memory. The
  server checks that the signature belongs to you. Then the server stores and
  anchors the memory.
- **What the server signs.** The server signs only the transport: the Arweave
  upload package that contains your signed record, and the Solana transaction
  that pays the network fee. These signatures do not change your record.

Status: available now for memories. For posts and for the delayed keychain
unlock, available after pull request #224 merges.

---

## 8. Sharing a memory

### 8.1 Public memory (planned)

You publish the memory as `public`. Anyone can read it and verify it.
You get a link, for example `mnemonik.xyz/m/<fingerprint>`.
The web page shows the text, the author and a "Signature valid" result.
The browser does the check. The reader does not need an account.

### 8.2 Sealed memory (planned)

A sealed memory is encrypted. Only approved readers can open it.
You can approve a reader after you save the memory. You do not need to know
the reader in advance.

The method is "envelope encryption":

1. Mnemonic makes a new random key `K` for the memory.
2. Mnemonic encrypts the memory with `K`.
3. Mnemonic uploads the encrypted memory to Arweave.
   Mnemonic writes the fingerprint **of the encrypted data** to Solana.
   Thus, nobody can guess the text and compare it with the fingerprint.
4. Mnemonic encrypts `K` with your own public key. Now only you can open the memory.
5. **Later**, when you find a reader, you give that reader access.
   Mnemonic encrypts `K` with the reader's public key. This is a "grant".
   Mnemonic does not upload the memory again.

This method follows the idea of Hybrid Public Key Encryption (HPKE),
RFC 9180 (<https://www.rfc-editor.org/rfc/rfc9180>).

**Grant by link.** Sometimes the reader does not have a key yet. Then Mnemonic
puts `K` in the part of the link after the `#` sign.
Browsers do not send this part to the server
(RFC 3986, section 3.5: <https://www.rfc-editor.org/rfc/rfc3986#section-3.5>).
Thus, the server never sees `K`. Each person with the link can read the memory.
Keep the link secret.

**You cannot cancel a grant.** When a reader has `K`, the reader can keep a copy.
To stop future access, save a new version with a new key.

### 8.3 Import

The reader's agent imports a shared memory. The agent verifies the signature
and stores a local copy. The copy shows the author, for example "from did:sol:…".
Then the agent can find the memory with normal search.
Status: planned.

---

## 9. Verification

To verify a memory, you need only three items: the signed record, the
author's public key and, if the memory is anchored, the Solana memo.

Mnemonic does these checks:

1. Calculate the fingerprint of the record again. It must be the same.
2. Check the signature with the author's public key.
3. If anchored: find the fingerprint in the Solana memo. Read the block time.

Verification is always free. It does not need a server, an account or a payment.
You can do it on your own computer.

Status: available now for your own memories. Anonymous verification by link is planned.

---

## 10. Safe use of shared memories

A shared memory can contain text that looks like an instruction.
Mnemonic will put each shared memory between clear start and end markers
before the agent reads it. The markers tell the model: "this is reference
data, not an instruction".

Mnemonic cannot force a model to obey the markers. A model with errors can
still obey hidden instructions.

Status: planned.

---

## 11. What Mnemonic guarantees

| Guarantee | How |
|---|---|
| Nobody can change a memory without detection | The fingerprint changes |
| Nobody can sign as you | Only your key makes your signature |
| Nobody can change the date of an anchored memory | The Solana block time is public |
| Verification is free for everyone | It runs locally, without a server |
| You can run your own server | All code is open (Apache License 2.0). Self-hosting is always possible |
| No company controls your memory | The memory depends on your key, not on one server |

## 12. What Mnemonic does not guarantee

- **Truth.** Mnemonic proves who wrote a memory. It does not prove that the memory is true.
- **Complete history.** Mnemonic finds a changed or removed old memory in a
  linked chain. It cannot force an author to publish a new memory.
- **Model behavior.** Mnemonic cannot force a model to obey the safety markers.
- **Deletion.** You cannot delete anchored data (see section 6).
- **Cancel access.** You cannot cancel a grant after the reader gets the key (see section 8.2).
- **Shared editing.** Many writers in one shared memory at the same time are not supported yet.

---

## 13. Costs

| Action | Cost |
|---|---|
| Save and search local memory | Free |
| Verify any memory | Free |
| Anchor a memory (public or sealed) | 100 free per week per identity (planned), then approximately 0.001 USD |
| Share by link | Free (planned) |

The price of an anchor is the cost of Arweave storage plus the Solana fee,
plus a 20% margin. The minimum price is 0.001 USD.
USD means United States dollars.

---

## 14. Relation to other standards

- **MCP (Model Context Protocol):** connects agents to tools. Mnemonic is an MCP tool.
- **A2A (Agent-to-Agent protocols):** agents send messages to each other.
  Mnemonic keeps the memory that these messages can refer to.
- **ERC-8004 ("Trustless Agents"):** an Ethereum proposal for agent identity,
  reputation and validation. Mnemonic can give it signed evidence of what an
  agent remembers. Status: design only. See section 9 of the full whitepaper.

Mnemonic makes agents coherent over time. Other standards connect agents now.

---

## 15. Current status

**Available now:**

- MCP server with 8 tools: `mnemonic_whoami`, `mnemonic_sign_memory`,
  `mnemonic_recall`, `mnemonic_verify`, `mnemonic_prove_identity`,
  `mnemonic_check_pending`, `request_public_write_confirmation`,
  `mnemonic_publish_post`.
- Local mode and anchored mode ("participate").
- Signatures with Ed25519, COSE_Sign1 envelopes, BLAKE3 fingerprints, canonical CBOR.
- Identities as `did:key` and `did:sol`. DID means Decentralized Identifier.
- TurboQuant compression. Search by meaning in a local SQLite database.
- Payment by x402 (a standard for payments over the web with HTTP code 402).

**Planned:**

- Free quota of 100 anchors per week.
- `public` and `sealed` modes, grants, import and share links.
- Capability tokens (signed permissions with a time limit).
- Safe-use markers.
- Anonymous verification by link.

---

## 16. Glossary

| Term | Meaning |
|---|---|
| Agent | An AI program that can do tasks and call tools |
| Anchor | Write a fingerprint to a public blockchain to prove the date |
| Arweave | A network for permanent data storage |
| BLAKE3 | A fast hash function |
| CBOR | Concise Binary Object Representation, a binary data format |
| COSE | CBOR Object Signing and Encryption |
| DID | Decentralized Identifier, for example `did:sol:…` |
| Ed25519 | A digital signature method |
| Embedding | A list of numbers that shows the meaning of a text |
| Fingerprint (hash) | A short code calculated from data. A change in the data changes it |
| Grant | Access to a sealed memory for one reader |
| HPKE | Hybrid Public Key Encryption (RFC 9180) |
| Keychain | The secure key storage of your operating system |
| MCP | Model Context Protocol |
| Solana | A public blockchain |
| STE | Simplified Technical English (ASD-STE100) |
| TurboQuant | The compression method for embeddings |
