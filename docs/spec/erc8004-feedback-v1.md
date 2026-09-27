# Mnemonic as ERC-8004 Reputation Layer

This document describes how the Mnemonic Protocol extends ERC-8004 ("Trustless Agents") with cryptographically verifiable reputation evidence. It covers the architecture, the `MNEMONIC_FEEDBACK_V1` document format, all interaction flows, and the verification procedure.

Related: `work/erc8004-reputation/tech-spec.md`, issue `#73`.

---

## The problem

ERC-8004 Reputation Registry accepts any rating from any wallet. A score of 98/100 stored on-chain carries no proof that the rater ever interacted with the rated agent, no proof the rater's identity has a history worth trusting, and no proof the specific claim was not fabricated after the fact.

Mnemonic closes that gap. Every agent interaction is signed with a long-lived Ed25519 identity and anchored on Arweave and Solana before the rating is written. The anchoring is time-ordered and content-addressed — a fabricated memory cannot be backdated, because the anchor transaction already exists at a block height that predates the rating.

---

## Architecture

```mermaid
flowchart TD
    subgraph AgentBox["AI Agent (Rater)"]
        identity["🔑 Ed25519 Identity\n(long-lived keypair)"]
        store["🗄️ Signed Memory Store\n(SQLite + Arweave index)"]
        wallet["💳 EVM Wallet\n(submits giveFeedback tx)"]
        sdk["📦 @mnemonik-xyz/sdk\nprepareFeedback()"]
    end

    subgraph MnemonicBox["Mnemonic Protocol"]
        mcp["⚙️ MCP Server\n(sign_memory, recall)"]
        arweave["🌊 Arweave Anchor\n(COSE envelope, permanent)"]
        solana["☀️ Solana Memo Anchor\n(SPL Memo, content hash)"]
    end

    subgraph EthBox["Ethereum Mainnet"]
        idReg["🪪 Identity Registry\n0x8004A169...e432\n(ERC-721)"]
        repReg["⭐ Reputation Registry\n0x8004BAa1...b63\ngiveFeedback()"]
    end

    hosting[("🌐 Caller-hosted URI\nHTTPS / IPFS / Arweave")]
    verifier(["👁️ Third-party Verifier\n(anyone)"])

    identity -->|"signs payload_hash (Ed25519)"| sdk
    store -->|"attestation_id + blake3"| sdk
    sdk -->|"MNEMONIC_FEEDBACK_V1 JSON"| hosting
    sdk -->|"giveFeedback calldata + preflight"| wallet
    wallet -->|"sendTransaction"| repReg
    repReg -->|"ownerOf / isApprovedForAll\n(self-promo guard)"| idReg

    mcp --> arweave
    mcp --> solana
    arweave -->|"anchor proof (tx id)"| store
    solana -->|"anchor proof (sig)"| store

    verifier -->|"readFeedback()\nfeedbackURI + feedbackHash"| repReg
    verifier -->|"GET document"| hosting
    verifier -->|"verifyFeedbackDocument()"| sdk
```

---

## Two-layer trust model

Mnemonic adds a distinct trust category to the ERC-8004 ecosystem, which already has:

| Trust type | Providers | How it works |
|---|---|---|
| TEE attestation | Phala, Marlin | Hardware-signed execution proof |
| Crypto-economic | Staking protocols | Slashable bond as collateral |
| **Signed memory** | **Mnemonic** | **Ed25519 identity + time-anchored evidence** |

The third category is unclaimed. The key property: a staking deposit can be posted today with a fresh wallet. A year of anchored memory history cannot be backdated — it would require rewriting Arweave blocks that are already globally replicated.

---

## The `MNEMONIC_FEEDBACK_V1` document

A plain JSON file hosted at `feedbackURI`. Any viem/ethers consumer can `JSON.parse` it. The Mnemonic signature lives inside it as a `proofs[]` entry; no CBOR stack is needed to read or verify the document.

```jsonc
{
  "schema": "MNEMONIC_FEEDBACK_V1",
  "feedback": {
    // Which registry and which agent is being rated.
    "agentRegistry": "eip155:1:0x8004BAa17C55a88189AE136b182e5fdA19dE9b63",
    "agentId": "42",                         // decimal STRING — uint256 exceeds JSON safe range

    // The rater's EVM address. MUST equal msg.sender of the giveFeedback tx.
    // Lives inside the hashed payload, so it is signed.
    "clientAddress": "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",

    "createdAt": "2026-09-27T12:00:00Z",     // RFC 3339, UTC, second precision
    "value": 9800,                           // safe-range integer
    "valueDecimals": 2,                      // → score is 98.00
    "tags": {                                // optional; omit keys rather than setting null
      "tag1": "quality",
      "tag2": "research"
    },
    "endpoint": "https://agent.example.com/a2a",  // optional: rated agent's service endpoint

    // The Mnemonic evidence. The cited attestation must have been anchored
    // BEFORE this document is created — that is the tamper-evident property.
    "mnemonic": {
      "schema": "MEMORY_V1",
      "attestation_id": "mn_01j...",
      "blake3": "<64 lowercase hex chars>",  // content hash of the cited attestation
      "ed25519_pubkey": "<base58>",          // rater's long-lived identity key
      "cose_envelope_uri": "ar://<txid>",    // optional: direct Arweave link
      "anchor": {                            // optional; anchor-agnostic shape
        "chain": "solana",
        "kind":  "spl-memo",
        "ref":   "<solana sig>"
      }
    },
    "note": "Completed 3 research tasks on time."   // optional, ≤ 280 chars
  },

  // keccak256 of JCS(document.feedback). This is what the Ed25519 key signs.
  "payload_hash": "0x<32 bytes hex>",

  // One or more proofs. The Ed25519 proof is always present.
  // The EIP-191 proof is optional — the caller produces it with their own wallet.
  "proofs": [
    {
      "type": "ed25519",
      "kid":  "<base58 pubkey>",
      "sig":  "<base64, 64 raw bytes>"
    },
    {
      "type":    "eip191",                   // optional
      "address": "0x<EIP-55>",
      "sig":     "0x<65 bytes>"
    }
  ]
}
```

### Why two hashes?

A single hash fails either way:

- Put the signatures **inside** the hashed bytes → circular dependency (you can't sign bytes that contain the signature).
- Put the signatures **outside** → an adversary strips or replaces them while the on-chain `feedbackHash` still matches.

The two-level design resolves this:

```
payload_hash  = keccak256(JCS(document.feedback))   ← what Ed25519 signs
feedbackHash  = keccak256(JCS(document))             ← covers payload_hash + proofs
```

The on-chain `feedbackHash` covers the proofs. Stripping a proof changes the document, which changes `feedbackHash`, which no longer matches the chain.

---

## Interaction flows

### Flow 1 — Agent submits a rating

```mermaid
sequenceDiagram
    participant A as AI Agent (Rater)
    participant SDK as @mnemonik-xyz/sdk
    participant Host as Document Host
    participant ETH as Reputation Registry
    participant IR as Identity Registry

    Note over A: Has an anchored memory of<br/>working with Agent B

    A->>SDK: prepareFeedback({<br/>  agentId, value, valueDecimals,<br/>  attestationId, clientAddress,<br/>  feedbackUri, tags?<br/>})
    SDK->>SDK: build feedback object
    SDK->>SDK: payload_hash = keccak256(JCS(feedback))
    SDK->>SDK: Ed25519.sign("MNEMONIC_FEEDBACK_V1:0x…") → proof
    SDK->>SDK: assemble document + proofs
    SDK->>SDK: feedbackHash = keccak256(JCS(document))
    SDK->>SDK: ABI-encode giveFeedback(…) → calldata
    SDK-->>A: PreparedFeedback { document, feedbackHash, onchain, preflight }

    A->>Host: PUT document at feedbackUri
    Host-->>A: 200 OK — URI live

    Note over A: Document must be reachable BEFORE tx lands

    A->>ETH: wallet.sendTransaction({ to, data: calldata })
    ETH->>IR: ownerOf(agentId)
    ETH->>IR: isApprovedForAll(owner, sender)
    ETH->>IR: getApproved(agentId)
    IR-->>ETH: results
    ETH->>ETH: require(sender ≠ owner …) self-promo guard
    ETH-->>A: tx confirmed — NewFeedback event emitted
```

---

### Flow 2 — Third party verifies a rating

```mermaid
sequenceDiagram
    participant V as Verifier (anyone)
    participant ETH as Reputation Registry
    participant Host as Document Host
    participant SDK as @mnemonik-xyz/sdk
    participant ARW as Arweave

    V->>ETH: readFeedback(agentId, clientAddr, idx)
    ETH-->>V: feedbackURI, feedbackHash (bytes32)
    V->>Host: GET feedbackURI
    Host-->>V: MNEMONIC_FEEDBACK_V1 JSON
    V->>SDK: verifyFeedbackDocument({ document, onchainFeedbackHash, onchainSender? })
    SDK->>SDK: ① keccak256(JCS(document)) == feedbackHash ?
    SDK->>SDK: ② keccak256(JCS(feedback)) == payload_hash ?
    SDK->>SDK: ③ Ed25519.verify(sig, payload_hash, pubkey) ?
    SDK->>SDK: ④ onchainSender == clientAddress ?
    opt Arweave deep-check (optional)
        V->>ARW: fetch cose_envelope_uri
        ARW-->>V: COSE_Sign1 envelope
        SDK->>SDK: blake3(COSE bytes) == mnemonic.blake3 ?
    end
    SDK-->>V: VerifyFeedbackResult { valid, senderBinding, checks }
```

---

### Flow 3 — Self-promotion guard (pre-flight)

```mermaid
flowchart TD
    A([prepareFeedback called]) --> B{clientAddress valid?\nEIP-55 checksum OK?}
    B -- no --> ERR1[❌ throw: invalid clientAddress]
    B -- yes --> C{zero address?}
    C -- yes --> ERR2[❌ throw: zero address]
    C -- no --> D{RPC URL available?}
    D -- no --> WARN[⚠️ warning: self-promo check skipped\ncontinue with calldata]
    D -- yes --> E[eth_call: ownerOf agentId]
    E --> F{sender == owner?}
    F -- yes --> ERR3[❌ throw: sender is agent owner]
    F -- no --> G[eth_call: isApprovedForAll]
    G --> H{sender is operator?}
    H -- yes --> ERR4[❌ throw: sender is approved operator]
    H -- no --> I[eth_call: getApproved agentId]
    I --> J{sender is approved?}
    J -- yes --> ERR5[❌ throw: sender is approved address]
    J -- no --> OK([✅ return PreparedFeedback])

    style ERR1 fill:#fdd,stroke:#c00
    style ERR2 fill:#fdd,stroke:#c00
    style ERR3 fill:#fdd,stroke:#c00
    style ERR4 fill:#fdd,stroke:#c00
    style ERR5 fill:#fdd,stroke:#c00
    style WARN fill:#ffe,stroke:#aa0
    style OK fill:#dfd,stroke:#080
```

---

### Flow 4 — Evidence chain (why Mnemonic ratings are harder to fake)

```mermaid
flowchart LR
    T0["**T₀** Interaction\nAgent A works with Agent B"]
    T1["**T₁** Sign memory\nEd25519 signs COSE envelope\nblake3 hash computed"]
    T2["**T₂** Anchor\nArweave upload → permanent\nSolana SPL Memo → immutable timestamp"]
    T3["**T₃** prepareFeedback\nCites blake3 from T₁\nEd25519 signs payload_hash"]
    T4["**T₄** giveFeedback tx\nfeedbackHash on-chain\nimmutable forever"]
    T5["**T₅** Verification\nAnyone checks chain + document\nOptional: confirm T₂ < T₄ on Arweave"]

    T0 --> T1 --> T2 --> T3 --> T4 --> T5

    style T0 fill:#e8f4f8,stroke:#5b9bd5
    style T1 fill:#e8f4f8,stroke:#5b9bd5
    style T2 fill:#d4edda,stroke:#28a745
    style T3 fill:#fff3cd,stroke:#ffc107
    style T4 fill:#fff3cd,stroke:#ffc107
    style T5 fill:#f0e6ff,stroke:#6f42c1
```

---

## Contract addresses (pinned)

Deployed via CREATE2 — same address on all mainnets, same address on all testnets.

| Registry | Mainnet | Testnet (Sepolia / Base Sepolia) |
|---|---|---|
| Identity Registry (ERC-721) | `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` | `0x8004A818BFB912233c491871b3d84c89A494BD9e` |
| Reputation Registry | `0x8004BAa17C55a88189AE136b182e5fdA19dE9b63` | `0x8004B663056A597Dffe9eCcC1965A193B7388713` |

Source: `qntx/erc8004` `networks.rs`, fetched 2026-09-27.

### `giveFeedback` ABI

```solidity
function giveFeedback(
    uint256 agentId,       // ERC-721 token ID of the rated agent
    int128  value,         // signed fixed-point score
    uint8   valueDecimals, // decimal places (0–18)
    string  tag1,          // plain string, passed through directly
    string  tag2,
    string  endpoint,      // optional: rated agent's service endpoint
    string  feedbackURI,   // URI of the MNEMONIC_FEEDBACK_V1 document
    bytes32 feedbackHash   // keccak256(JCS(document)) — the on-chain commitment
) external
```

4-byte selector: `0x3c036a7e`
(`keccak256("giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)")[0:4]`)

**Deviations from the original backlog spec (`work/a2a-bridge/backlog.md` Path 3):**

| Parameter | Backlog (incorrect) | Actual contract |
|---|---|---|
| `value` | `uint256` | `int128` (signed) |
| `tag1`, `tag2` | `bytes32` | `string` |
| `endpoint` | not listed | extra `string` param |
| `feedbackHash` | `blake3(canonical JSON)` | `keccak256(JCS(document))` |

---

## Signed message format

Both proof types sign the same string — one line, shell-safe, copy-pasteable into `cast wallet sign`:

```
MNEMONIC_FEEDBACK_V1:0x<payload_hash lowercase hex>
```

The Ed25519 proof is a raw 64-byte signature over the UTF-8 bytes of that string. Not COSE: a verifier with `@noble/ed25519` or `ed25519-dalek` can verify in three lines and no CBOR stack.

The optional EIP-191 proof is a standard personal_sign over the same string. The caller produces it with their own wallet; Mnemonic never holds the EVM key.

---

## What this proves (and what it does not)

### Proves

- The document has not been modified since `feedbackHash` was committed on-chain (hash binding).
- The Ed25519 key named in `mnemonic.ed25519_pubkey` endorsed this exact payload at creation time (signature binding).
- The cited memory existed on Arweave before the rating was written, if `mnemonic.anchor` is present and the verifier checks Arweave.
- The rater's EVM address is the same entity that holds the Ed25519 key, for this specific rating (conditional on the sender check).

### Does not prove

- That the cited memory is *about* Agent B. `blake3` and the anchor prove existence and time, not semantic content.
- Anti-Sybil protection. A single operator can hold many identities. Mnemonic adds cost: a long history cannot be backdated. It does not make new identities impossible.
- A permanent rater-to-address binding. The EVM address is committed per-document, not globally linked to the Ed25519 key forever.

---

## Reproducible fixture verification

```bash
# Recompute feedbackHash from the document bytes alone:
jq -j '.documentJson' packages/sdk/test/fixtures/erc8004-feedback-v1.json | cast keccak
# Must equal fixture.feedbackHash

# Verify the giveFeedback calldata selector:
cast 4byte 0x3c036a7e
# Must return: giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)
```
