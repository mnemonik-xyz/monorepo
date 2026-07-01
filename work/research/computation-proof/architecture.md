---
created: 2026-07-01
type: architecture-diagrams
status: viable-architecture (post recursion PoC)
---

# Mnemonic correspondence — viable architecture (flow diagrams)

The architecture we converged on after the zigz recursion PoC. Core principle:
**zigz proves the policy (no in-guest hashing); Rust does the hashing/binding and
verifies; recursion is deferred behind a Poseidon2 precompile.**

## 1. End-to-end flow (intent → action → proof → verify)

```mermaid
sequenceDiagram
    autonumber
    actor P as Principal
    actor A as Agent
    participant EV as EvidenceSource
    participant PR as Prover_zigz
    participant CO as core_correspondence
    participant ST as Storage_and_Anchor
    actor V as Verifier

    P->>P: sign INTENT (typed mandate)
    P-->>A: intent_hash
    A->>A: build ACTION (references intent_hash)
    A->>EV: collect evidence
    EV-->>A: evidence_commitment + attestation
    A->>CO: action_commitment = blake3(action fields)
    A->>PR: prove policy over (action, evidence, intent)
    PR-->>A: proof + public_inputs
    A->>A: sign ACTION (cert in metadata)
    A->>ST: store bytes + anchor batched Merkle root
    V->>ST: fetch bundle by hash
    V->>CO: verify_correspondence(intent, action, cert)
    CO-->>V: 5 checks -> policy_valid
```

## 2. Division of labor (the load-bearing decision)

```mermaid
flowchart LR
    subgraph GUEST["zigz GUEST (rv64im) — POLICY LOGIC ONLY"]
        direction TB
        g1["arithmetic and aggregates (sum ≤ cap)"]
        g2["membership (scan / range)"]
        g3["equality to evidence"]
        g4["temporal / sequencing"]
        g5["*** NO in-guest hashing ***"]
    end
    subgraph RUST["Rust core/correspondence — VERIFY ONLY (native + wasm)"]
        direction TB
        r1["blake3 hashing and commitments"]
        r2["action_commitment binding"]
        r3["re-verify zigz proof"]
        r4["5-check verify_correspondence"]
    end
    GUEST -->|"proof + public_inputs"| RUST
    RUST -->|"policy_valid: Option bool"| OUT["verify result"]
```

## 3. The five independent verifier checks

```mermaid
flowchart TD
    S([verify_correspondence]) --> C1{intent COSE signature valid}
    C1 -- no --> X([reject / policy_valid = false])
    C1 -- yes --> C2{action COSE signature valid}
    C2 -- no --> X
    C2 -- yes --> C3{action.intent_ref equals intent_hash}
    C3 -- no --> X
    C3 -- yes --> C4{recomputed action_commitment matches cert AND zigz proof verifies}
    C4 -- no --> X
    C4 -- yes --> C5{evidence attestation valid}
    C5 -- no --> X
    C5 -- yes --> OK([safe = true])
```

## 4. Two guest-prover jobs: viable now vs deferred

```mermaid
flowchart TD
    subgraph JA["JOB A — policy proving (VIABLE NOW)"]
        a1["zigz guest proves a bounded deterministic policy"]
        a2["seconds to prove; broad test-case range"]
    end
    subgraph JB["JOB B — recursion / aggregation (DEFERRED)"]
        b1["verify a zigz proof INSIDE a guest"]
        b2["~25.6k RISC-V steps per Poseidon2 perm (measured)"]
    end
    JA -->|"unbounded intents"| CH["checkpoint state-chaining (no recursion needed)"]
    JB -.->|"blocked by"| GATE{{"A1: Poseidon2 Lasso precompile"}}
    GATE -->|"unlocks later"| AGG["proof aggregation + on-chain STARK to SNARK wrap"]
```

## 5. Anchor is not storage — durability classes

```mermaid
flowchart LR
    OBJ["signed, content-addressed object"]
    OBJ --> ANC["ANCHOR: batched Merkle root on a clock (default OpenTimestamps to Bitcoin)"]
    OBJ --> DUR{durability class}
    DUR --> D0["D0 self-custody (dev only, invalid for audit)"]
    DUR --> D1["D1 accountable relays (k signed receipts)"]
    DUR --> D2["D2 Filecoin PoSt (provable storage)"]
    DUR --> D3["D3 Arweave permanent (ANS-104 batched)"]
```

## 6. Workspace / dependency DAG (one-way, everything points at portable core)

```mermaid
flowchart TD
    core["core (portable): codec, correspondence-VERIFY, identity, merkle"]
    prover["mnemonic-prover: zigz PRODUCE + evidence"]
    wasm["wasm exporter"]
    native["native: solana, arweave, storage, keychain"]
    mcp["mcp server: orchestrate"]
    sdk["SDK (TS + wasm)"]
    core --> prover
    core --> wasm
    core --> native
    core --> mcp
    prover --> mcp
    native --> mcp
    wasm --> sdk
```

## 7. Application layers + client surfaces (what runs where)

```mermaid
flowchart TB
    subgraph CLIENTS["CLIENT SURFACES"]
        web["Webapp (React): sign / recall / verify UI"]
        ext["Browser Extension: client-side keys + wasm verify"]
        cli["CLI: sign / verify / prove, keychain"]
        agent["Agent via MCP tools: prove / verify_correspondence"]
        aud["Auditor / Contract: VERIFY only"]
    end
    subgraph SDKL["SDK LAYER (TS + wasm)"]
        sdk["@mnemonik-xyz/sdk: wasm VERIFIER + envelope build"]
    end
    subgraph DOMAIN["DOMAIN (Rust)"]
        core["core: verify_correspondence, action_commitment, codec, identity (native + wasm)"]
        prover["mnemonic-prover: zigz PRODUCE + evidence"]
        mcpsrv["mcp server: orchestrate produce to bind to anchor"]
    end
    subgraph INFRA["STORAGE + ANCHOR"]
        store["durable store (D1..D3)"]
        anchor["anchor: OTS/Bitcoin, Solana, TSA"]
    end

    web --> sdk
    ext --> sdk
    aud --> sdk
    cli --> core
    agent --> mcpsrv
    sdk --> core
    mcpsrv --> core
    mcpsrv --> prover
    mcpsrv --> store
    mcpsrv --> anchor
    prover --> core
```

### What each level does

| Level | Surface / component | Job | Produce / Verify / Anchor |
|---|---|---|---|
| Client | Webapp, Extension | UX + hold keys + **verify client-side (wasm)** | Verify (+ trigger produce) |
| Client | CLI | sign / verify / prove; keychain identity | Produce + Verify |
| Client | Agent (MCP tools) | request prove/verify of correspondence | Produce (via server) |
| Client | Auditor / Contract | independent re-check | **Verify only** |
| SDK | @mnemonik-xyz/sdk | wasm verifier + envelope building | Verify |
| Domain | core | hashing, binding, `verify_correspondence` | **Verify** (never produces) |
| Domain | mnemonic-prover | zigz policy proof + evidence | **Produce** |
| Domain | mcp server | orchestrate produce → bind → anchor | Orchestrate |
| Infra | durable store + anchor | availability guarantee + timestamp | Anchor / Store |

**Invariant:** the verifier is a *library* that runs on every client surface
(browser, CLI, contract) — "verify everywhere." Only `mnemonic-prover` produces;
`core` only verifies; the server only orchestrates and never signs.
