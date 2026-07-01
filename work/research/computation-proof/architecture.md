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
    participant REG as Policy Registry
    actor AG as Agent (AI)
    participant EV as Evidence
    participant PR as Prover zigz
    participant CORE as core verify
    participant ST as Storage plus Anchor
    actor V as Verifier

    Note over P: ROOT OF AUTHORITY. Signs the mandate (Ed25519). Trust anchor for what was authorized.
    Note over REG: Compiled policy programs (guest ELF), addressed by policy_id = program_hash. Public.
    Note over AG: The agent's own code (LLM plus tools). May be buggy or compromised. NOT TRUSTED.
    Note over EV: zkTLS proves bytes came from the endpoint, not that the endpoint is honest.
    Note over PR: Runs the policy guest in the zkVM. Trusted only for SOUNDNESS: cannot prove a false statement.
    Note over CORE: Pure verifier (Rust plus wasm). TRUSTLESS, runs anywhere. Does hashing plus binding.

    P->>REG: pick policy_id (e.g. payment_mandate_v1)
    P->>P: sign INTENT with policy_id, params (cap, allowlist_root), expiry, nonce
    P-->>AG: signed Intent plus intent_hash
    Note over AG,EV: Agent decides an action, then must PROVE compliance. It cannot merely assert.
    AG->>EV: fetch authenticated evidence (merchant receipt)
    EV-->>AG: evidence plus attestation
    AG->>PR: witness = action, evidence, params; program = policy_id
    PR->>PR: execute policy guest, produce proof pi plus public_inputs
    PR-->>AG: pi binds program_hash, intent_hash, action_commitment, evidence_commitment
    AG->>AG: sign ACTION with agent key (cert in metadata)
    AG->>ST: store bytes (durability class) plus anchor batched root
    V->>ST: fetch by content hash
    V->>CORE: verify_correspondence(intent, action, cert)
    Note over V,CORE: Re-checks program_hash == intent.policy_id, every binding, and pi. No trust in agent/prover/storage.
    CORE-->>V: authorship + integrity + intent_link + POLICY + evidence  =>  policy_valid
```

### Reading the flow — the questions this answers

**Where does the policy live?** The policy is a **compiled program** (the zigz
guest, e.g. `payment_mandate_v1`), content-addressed by `program_hash` and
published in the **policy registry**. The **Principal's signed Intent names the
`policy_id` (= program_hash) + parameters** (cap, allowlist root, expiry). So the
policy code is public and immutable, and *which* policy applies is fixed by the
Principal's signature. The verifier checks the proof's `program_hash` equals the
Intent's `policy_id` — you cannot silently swap in a weaker policy.

**Who is the agent, and does it run its own code?** The Agent is the autonomous
AI (LLM + tools) acting for the Principal. It runs **arbitrary, untrusted code** —
its reasoning is *not* proven. What gets proven is only that the **recorded action
satisfies the policy given the evidence**. The agent drives the prover but cannot
make it prove a false statement. Treat the agent as potentially adversarial — that
is the whole design premise.

**What is the trust assumption?** In one line: **trust the Principal's signature
and the soundness of the proof system; trust nothing about the agent, the prover's
honesty, or storage.**

### Trust assumptions

| Party | Trusted for | NOT trusted for |
|---|---|---|
| **Principal** | defining the mandate (root of authority), via Ed25519 sig | — |
| **Policy registry** | serving the correct program for a `program_hash` (content-addressed, self-verifying) | — |
| **Agent (AI)** | **nothing** | honesty, correctness, non-compromise |
| **Prover (zigz)** | **soundness** (valid π ⇒ statement holds) | honesty (can't fake π). *Caveat: zigz unaudited → experimental* |
| **Evidence (zkTLS)** | bytes came from *that TLS endpoint* | that the endpoint told the truth |
| **core verifier** | correctness of the verify code (differential-tested) | — (trustless to run) |
| **Storage / relay** | **nothing** (content-addressed) | availability → the durability class provides it |
| **Anchor chain** | existence + time + ordering of a root | data (never holds data) |

### Attack surfaces (and mitigation)

| # | Attack | Mitigation |
|---|---|---|
| 1 | Agent lies about action data (fakes a compliant amount) | **evidence binding** — action fields must equal merchant-authenticated evidence |
| 2 | Agent swaps in a weaker policy | verifier checks `program_hash == intent.policy_id`; Intent is Principal-signed |
| 3 | Agent forges the Intent | Ed25519 signature — unforgeable |
| 4 | Replay an old Intent / proof | `nonce` + `expiry` in the Intent; anchored timestamp |
| 5 | Prover fakes a proof of a false result | **soundness** of the proof system (can't). Residual: zigz unaudited |
| 6 | Agent fabricates evidence | zkTLS transcript is unforgeable (stub evidence is a *dev-only* trust hole) |
| 7 | Merchant/endpoint itself lies | **NOT mitigated** — zkTLS proves origin, not truth; residual trust in the source |
| 8 | Producer deletes the record to dodge audit | **durability class D1–D3** — independent custody; producer not sole holder |
| 9 | Compromised agent/principal key | out of scope of the proof — identity / key-management layer |
| 10 | Buggy verifier accepts bad proofs | pure-Rust verifier + **differential conformance** vs the Zig prover |

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

**Why this split (measured, not aesthetic):** the recursion PoC found one real
Poseidon2 hash costs **~25.6k RISC-V steps in-VM**. So hashing is kept in Rust
(microseconds, native) and the guest does only policy arithmetic — which keeps the
policy test-case range broad and proving cheap. The guest that hashes is the guest
that's slow; we don't build those.

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
    subgraph EDGE["CLIENT SURFACES — UNTRUSTED EDGE (hold keys, verify locally)"]
        web["Webapp (React)<br/>sign / recall / verify UI<br/>holds user key (non-custodial)<br/>VERIFIES via wasm"]
        ext["Browser Extension<br/>client-side keys + local recall<br/>VERIFIES via wasm"]
        cli["CLI<br/>sign / verify / PROVE<br/>keychain identity"]
        agent["Agent (MCP tools)<br/>requests prove/verify<br/>runs UNTRUSTED code"]
        aud["Auditor / Smart Contract<br/>independent re-check<br/>VERIFY ONLY (trustless)"]
    end
    subgraph SDKL["SDK LAYER — TS + wasm (verification runs at the edge)"]
        sdk["@mnemonik-xyz/sdk<br/>wasm VERIFIER + envelope build<br/>no secrets, no server trust"]
    end
    subgraph DOMAIN["DOMAIN — Rust (operator infra)"]
        core["core (portable, native + wasm)<br/>verify_correspondence + action_commitment<br/>hashing + binding — VERIFY ONLY, never signs"]
        prover["mnemonic-prover<br/>zigz PRODUCE + evidence<br/>trusted only for SOUNDNESS"]
        mcpsrv["mcp server<br/>orchestrate produce to bind to anchor<br/>signs NOTHING (non-custodial)"]
    end
    subgraph INFRA["STORAGE + ANCHOR — NEUTRAL, UNTRUSTED"]
        store["durable store (D1..D3)<br/>content-addressed, untrusted<br/>durability = the guarantee"]
        anchor["anchor: OTS/Bitcoin, Solana, TSA<br/>root only, never data"]
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

**Trust boundary.** Everything the relying party needs to trust is *cryptographic*,
not *operational*: the **Principal's signature** and **proof soundness**. The
client edge holds keys and **verifies locally** (wasm), so it never trusts the
server; the operator infra **produces + orchestrates but signs nothing**
(non-custodial); storage + anchor are **neutral and untrusted** (content-addressed
+ durability-guaranteed). An auditor or smart contract at the edge re-checks
everything with the same open verifier — no operator trust required.

### What each level does

| Level | Surface / component | Job | P/V/A | Trust |
|---|---|---|---|---|
| Client | Webapp, Extension | UX + hold keys + **verify client-side (wasm)** | Verify (+ trigger produce) | untrusted edge; verifies locally |
| Client | CLI | sign / verify / prove; keychain identity | Produce + Verify | holds user key (non-custodial) |
| Client | Agent (MCP tools) | request prove/verify | Produce (via server) | **UNTRUSTED** (adversarial) |
| Client | Auditor / Contract | independent re-check | **Verify only** | trustless relying party |
| SDK | @mnemonik-xyz/sdk | wasm verifier + envelope building | Verify | no secrets, no server trust |
| Domain | core | hashing, binding, `verify_correspondence` | **Verify** (never produces) | trustless to run |
| Domain | mnemonic-prover | zigz policy proof + evidence | **Produce** | trusted for **soundness only** |
| Domain | mcp server | orchestrate produce → bind → anchor | Orchestrate | signs nothing (non-custodial) |
| Infra | durable store + anchor | availability guarantee + timestamp | Anchor / Store | untrusted (content-addressed) |

**Invariant:** the verifier is a *library* that runs on every client surface
(browser, CLI, contract) — "verify everywhere." Only `mnemonic-prover` produces;
`core` only verifies; the server only orchestrates and never signs.
