# Mnemonic as ERC-8004 Reputation Layer

This document describes how the Mnemonic Protocol extends ERC-8004 ("Trustless Agents") with cryptographically verifiable reputation evidence. It covers the architecture, the `MNEMONIC_FEEDBACK_V1` document format, all interaction flows, and the verification procedure.

Related: `work/erc8004-reputation/tech-spec.md`, issue `#73`.

---

## The problem

ERC-8004 Reputation Registry accepts any rating from any wallet. A score of 98/100 stored on-chain carries no proof that the rater ever interacted with the rated agent, no proof the rater's identity has a history worth trusting, and no proof the specific claim was not fabricated after the fact.

Mnemonic closes that gap. Every agent interaction is signed with a long-lived Ed25519 identity and anchored on Arweave and Solana before the rating is written. The anchoring is time-ordered and content-addressed — a fabricated memory cannot be backdated, because the anchor transaction already exists at a block height that predates the rating.

---

## Architecture

```plantuml
@startuml erc8004-architecture
!theme plain
skinparam componentStyle rectangle
skinparam defaultFontSize 12

package "AI Agent (Rater)" {
  component [Ed25519 Identity\n(long-lived keypair)] as identity
  component [Signed Memory Store\n(SQLite + Arweave index)] as store
  component [EVM Wallet\n(submits giveFeedback tx)] as wallet
  component [@mnemonik-xyz/sdk\nprepareFeedback()] as sdk
}

package "Mnemonic Protocol" {
  component [MCP Server\n(sign_memory, recall)] as mcp
  component [Arweave Anchor\n(COSE envelope, ANS-104)] as arweave
  component [Solana Memo Anchor\n(SPL Memo, content hash)] as solana
}

package "Ethereum Mainnet" {
  component [Identity Registry\n0x8004A169...e432\n(ERC-721, one NFT per agent)] as idReg
  component [Reputation Registry\n0x8004BAa1...b63\ngiveFeedback()] as repReg
}

cloud "Off-chain Hosting" {
  component [Caller-hosted URI\n(HTTPS / IPFS / Arweave*)] as hosting
}

actor "Third-party Verifier\n(anyone)" as verifier

identity -right-> sdk : signs payload_hash\n(Ed25519)
store --> sdk : attestation_id\nblake3, anchor ref
sdk -right-> hosting : MNEMONIC_FEEDBACK_V1\n(plain JSON)
sdk -right-> wallet : giveFeedback calldata\n+ preflight warnings
wallet -down-> repReg : sendTransaction(to, data)
repReg -left-> idReg : ownerOf() / isApprovedForAll()\n(self-promo guard)

mcp --> arweave : COSE_Sign1 envelope
mcp --> solana : SPL Memo (blake3 + arweave_tx)
arweave --> store : anchor proof (tx id)
solana --> store : anchor proof (sig)

verifier -up-> repReg : readFeedback()\nfeedbakURI + feedbackHash
verifier --> hosting : GET document
verifier --> sdk : verifyFeedbackDocument()

note bottom of hosting
  * Arweave hosting is a planned
  additive flag (Round 2).
  Round 1: caller provides the URI.
end note

@enduml
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
    participant ETH as Ethereum<br/>Reputation Registry
    participant IR as Identity Registry

    Note over A: Has an anchored memory of<br/>working with Agent B

    A->>SDK: prepareFeedback({<br/>  agentId, value, valueDecimals,<br/>  attestationId, clientAddress,<br/>  feedbackUri, tags?<br/>})

    SDK->>SDK: build feedback object<br/>(JCS-validate all values)
    SDK->>SDK: payload_hash = keccak256(JCS(feedback))
    SDK->>SDK: Ed25519.sign("MNEMONIC_FEEDBACK_V1:0x<payload_hash>")<br/>→ ed25519 proof
    SDK->>SDK: assemble document<br/>{ schema, feedback, payload_hash, proofs }
    SDK->>SDK: feedbackHash = keccak256(JCS(document))
    SDK->>SDK: ABI-encode giveFeedback(..., feedbackHash)<br/>→ calldata (0xd5d1e4af...)
    SDK-->>A: PreparedFeedback {<br/>  documentJson, feedbackHash,<br/>  onchain: { chainId, to, data },<br/>  preflight: { requiredSender, warnings }<br/>}

    A->>Host: PUT documentJson at feedbackUri
    Host-->>A: 200 OK — URI live

    Note over A: Document must be reachable<br/>BEFORE the tx lands

    A->>ETH: wallet.sendTransaction({<br/>  to: 0x8004BAa1...b63,<br/>  data: calldata<br/>})
    ETH->>IR: ownerOf(agentId) → owner
    ETH->>IR: isApprovedForAll(owner, sender)
    ETH->>IR: getApproved(agentId)
    IR-->>ETH: results
    ETH->>ETH: require(sender ≠ owner, ...)<br/>self-promo guard
    ETH-->>A: tx confirmed<br/>emit NewFeedback(agentId, sender,<br/>  feedbackIndex, value, ..., feedbackHash)
```

---

### Flow 2 — Third party verifies a rating

```mermaid
sequenceDiagram
    participant V as Verifier (anyone)
    participant ETH as Ethereum<br/>Reputation Registry
    participant Host as Document Host
    participant SDK as @mnemonik-xyz/sdk
    participant ARW as Arweave

    V->>ETH: readFeedback(agentId, clientAddr, idx)
    ETH-->>V: feedbackURI, feedbackHash (bytes32)

    V->>Host: GET feedbackURI
    Host-->>V: MNEMONIC_FEEDBACK_V1 JSON

    V->>SDK: verifyFeedbackDocument({<br/>  document,<br/>  onchainFeedbackHash,<br/>  onchainSender?<br/>})

    SDK->>SDK: ① recompute feedbackHash<br/>keccak256(JCS(document))<br/>== onchainFeedbackHash ?
    SDK->>SDK: ② verify payload_hash<br/>keccak256(JCS(document.feedback))<br/>== document.payload_hash ?
    SDK->>SDK: ③ verify Ed25519 proof<br/>ed25519.verify(sig, "MNEMONIC_FEEDBACK_V1:0x<payload_hash>", pubkey)
    SDK->>SDK: ④ check sender binding<br/>onchainSender == feedback.clientAddress ?

    opt Arweave deep-check (optional)
        V->>ARW: fetch cose_envelope_uri
        ARW-->>V: COSE_Sign1 envelope
        V->>SDK: verifyFeedbackDocument(..., { checkArweave: true })
        SDK->>SDK: blake3(COSE bytes) == mnemonic.blake3 ?
    end

    SDK-->>V: VerifyFeedbackResult {<br/>  valid: true,<br/>  senderBinding: "verified" | "not-checked",<br/>  checks: { hash, payloadHash, ed25519, sender }<br/>}
```

---

### Flow 3 — Self-promotion guard (pre-flight)

```mermaid
flowchart TD
    A([prepareFeedback called]) --> B{clientAddress format\nvalid? EIP-55 checksum?}
    B -- no --> ERR1[throw: invalid clientAddress]
    B -- yes --> C{clientAddress == agentId address?\nzero address?}
    C -- yes --> ERR2[throw: obvious self-promotion]
    C -- no --> D{RPC URL available?}

    D -- no --> WARN[emit warning: self-promo check\nskipped — no RPC endpoint\ncontinue with calldata]
    D -- yes --> E[eth_call: ownerOf agentId]
    E --> F{sender == owner?}
    F -- yes --> ERR3[throw: sender is agent owner]
    F -- no --> G[eth_call: isApprovedForAll\nowner, sender]
    G --> H{sender is operator?}
    H -- yes --> ERR4[throw: sender is approved operator]
    H -- no --> I[eth_call: getApproved agentId]
    I --> J{sender is approved address?}
    J -- yes --> ERR5[throw: sender is approved address]
    J -- no --> OK([return PreparedFeedback])

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
timeline
    title Evidence chain for a Mnemonic-backed rating

    section Interaction happens
        T₀ : Agent A works with Agent B
           : Interaction recorded in Agent A's Mnemonic store

    section Anchoring (before the rating)
        T₁ : sign_memory called
           : Ed25519 signs the COSE_Sign1 envelope
           : blake3 hash computed
        T₂ : Arweave upload — COSE bytes pinned permanently
           : Solana SPL Memo — blake3 + arweave_tx anchored on-chain
           : Anchor timestamp is now immutable

    section Rating submitted (after anchoring)
        T₃ : prepareFeedback called
           : Document cites blake3 from T₁ (already anchored)
           : payload_hash signed with same Ed25519 key
           : feedbackHash = keccak256(JCS(document))
        T₄ : giveFeedback tx confirmed on Ethereum
           : feedbackHash stored immutably on-chain

    section Verification (any time after T₄)
        T₅ : Verifier checks feedbackHash on-chain
           : Downloads document, verifies hashes + sig
           : Optionally fetches Arweave anchor to confirm T₂ < T₄
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

4-byte selector: `0xd5d1e4af`
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

The canonical fixture at `packages/sdk/test/fixtures/erc8004-feedback-v1.json` is reproducible by any third party with no Mnemonic code:

```bash
# Recompute feedbackHash from the document bytes alone:
jq -j '.documentJson' packages/sdk/test/fixtures/erc8004-feedback-v1.json | cast keccak
# Must equal fixture.feedbackHash

# Verify the giveFeedback calldata selector:
cast 4byte 0xd5d1e4af
# Must return: giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)
```

The Rust parity test (`mcp/tests/erc8004_feedback_parity.rs`) reads the same fixture and asserts both hashes using `alloy_primitives::keccak256` — cross-ecosystem proof that the TypeScript JCS + keccak256 matches the Rust implementation.
