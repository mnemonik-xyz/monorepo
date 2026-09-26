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
    participant REG as PolicyRegistry
    actor AG as Agent
    participant EV as Evidence
    participant PR as ProverZigz
    participant CORE as CoreVerify
    participant ST as StorageAnchor
    actor V as Verifier

    Note over P: ROOT OF AUTHORITY.<br/>Signs the mandate with Ed25519.<br/>Trust anchor for<br/>what was authorized.
    Note over REG: Compiled policy programs,<br/>addressed by policy_id<br/>which equals program_hash.<br/>Public.
    Note over AG: The agent own code,<br/>LLM and tools. May be buggy<br/>or compromised.<br/>NOT TRUSTED.
    Note over EV: zkTLS proves bytes came<br/>from the endpoint, not that<br/>the endpoint is honest.
    Note over PR: Runs the policy guest<br/>in the zkVM. Trusted only<br/>for SOUNDNESS.
    Note over CORE: Pure verifier, Rust and wasm.<br/>TRUSTLESS, runs anywhere.<br/>Does hashing and binding.

    P->>REG: pick policy_id such <br>/as payment_mandate_v1
    P->>P: sign INTENT with policy_id, params, expiry, nonce
    P-->>AG: signed Intent and intent_hash
    Note over AG,EV: Agent decides an action, <br>/then must PROVE compliance.<br>/ It cannot merely assert.
    AG->>EV: fetch authenticated evidence,<br>/ a merchant receipt
    EV-->>AG: evidence and attestation
    AG->>PR: witness is action, evidence, <br>/params and program is policy_id
    PR->>PR: execute policy guest, <br>/produce proof pi and public_inputs
    PR-->>AG: pi binds program_hash, intent_hash, <br>/action_commitment, evidence_commitment
    AG->>AG: sign ACTION with agent key, cert in metadata
    AG->>ST: store bytes with durability <br>/class and anchor batched root
    V->>ST: fetch by content hash
    V->>CORE: verify_correspondence over intent, action, cert
    Note over V,CORE: Re-checks program_hash <br/>equals intent.policy_id, every binding,<br/> and pi. No trust in agent, prover, storage.
    CORE-->>V: authorship, integrity, intent_link, POLICY, evidence, then policy_valid
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
    subgraph GUEST["zigz GUEST (rv64im)<br/>POLICY LOGIC ONLY"]
        direction TB
        g1["arithmetic + aggregates<br/>(sum <= cap)"]
        g2["membership<br/>(scan / range)"]
        g3["equality to evidence"]
        g4["temporal / sequencing"]
        g5["*** NO in-guest<br/>hashing ***"]
    end
    subgraph RUST["Rust core/correspondence<br/>VERIFY ONLY<br/>(native + wasm)"]
        direction TB
        r1["blake3 hashing<br/>+ commitments"]
        r2["action_commitment<br/>binding"]
        r3["re-verify<br/>zigz proof"]
        r4["5-check<br/>verify_correspondence"]
    end
    GUEST -->|"proof +<br/>public_inputs"| RUST
    RUST -->|"policy_valid:<br/>Option bool"| OUT["verify<br/>result"]
```

**Why this split (measured, not aesthetic):** the recursion PoC found one real
Poseidon2 hash costs **~25.6k RISC-V steps in-VM**. So hashing is kept in Rust
(microseconds, native) and the guest does only policy arithmetic — which keeps the
policy test-case range broad and proving cheap. The guest that hashes is the guest
that's slow; we don't build those.

## 3. The five independent verifier checks

```mermaid
flowchart TD
    S(["verify_correspondence"]) --> C1{"intent COSE signature valid?"}
    C1 -- no --> X(["reject: policy_valid false"])
    C1 -- yes --> C2{"action COSE signature valid?"}
    C2 -- no --> X
    C2 -- yes --> C3{"action.intent_ref equals intent_hash?"}
    C3 -- no --> X
    C3 -- yes --> C4{"action_commitment matches cert AND zigz proof verifies?"}
    C4 -- no --> X
    C4 -- yes --> C5{"evidence attestation valid?"}
    C5 -- no --> X
    C5 -- yes --> OK(["safe true"])
```

## 4. Two guest-prover jobs: viable now vs deferred

```mermaid
flowchart TD
    subgraph JA["JOB A - policy proving<br/>(VIABLE NOW)"]
        a1["zigz guest proves a<br/>bounded deterministic policy"]
        a2["seconds to prove;<br/>broad test-case range"]
    end
    subgraph JB["JOB B - recursion / aggregation<br/>(DEFERRED)"]
        b1["verify a zigz proof<br/>INSIDE a guest"]
        b2["~25.6k RISC-V steps<br/>per Poseidon2 perm<br/>(measured)"]
    end
    JA -->|"unbounded<br/>intents"| CH["checkpoint<br/>state-chaining<br/>(no recursion needed)"]
    JB -.->|"blocked by"| GATE{{"A1: Poseidon2<br/>Lasso precompile"}}
    GATE -->|"unlocks<br/>later"| AGG["proof aggregation +<br/>on-chain STARK<br/>to SNARK wrap"]
```

## 5. Anchor is not storage — durability classes

```mermaid
flowchart TD
    OBJ["signed,<br/>content-addressed<br/>object"]
    OBJ --> ANC["ANCHOR<br/>batched Merkle root<br/>on a clock<br/>(default OpenTimestamps<br/>to Bitcoin)"]
    OBJ --> DUR{"durability<br/>class"}
    DUR --> D0["D0 self-custody<br/>(dev only,<br/>invalid for audit)"]
    DUR --> D1["D1 accountable relays<br/>(k signed receipts)"]
    DUR --> D2["D2 Filecoin PoSt<br/>(provable storage)"]
    DUR --> D3["D3 Arweave permanent<br/>(ANS-104 batched)"]
```

## 6. Workspace / dependency DAG (one-way, everything points at portable core)

```mermaid
flowchart TD
    core["core (portable)<br/>codec, correspondence-VERIFY,<br/>identity, merkle"]
    prover["mnemonic-prover<br/>zigz PRODUCE<br/>+ evidence"]
    wasm["wasm<br/>exporter"]
    native["native<br/>solana, arweave,<br/>storage, keychain"]
    mcp["mcp server<br/>orchestrate"]
    sdk["SDK<br/>(TS + wasm)"]
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
    subgraph EDGE["CLIENT SURFACES - UNTRUSTED EDGE (hold keys, verify locally)"]
        web["Webapp (React)<br/>sign / recall / verify UI<br/>holds user key (non-custodial)<br/>VERIFIES via wasm"]
        ext["Browser Extension<br/>client-side keys + local recall<br/>VERIFIES via wasm"]
        cli["CLI<br/>sign / verify / PROVE<br/>keychain identity"]
        agent["Agent (MCP tools)<br/>requests prove/verify<br/>runs UNTRUSTED code"]
        aud["Auditor / Smart Contract<br/>independent re-check<br/>VERIFY ONLY (trustless)"]
    end
    subgraph SDKL["SDK LAYER - TS + wasm (verification runs at the edge)"]
        sdk["@mnemonik-xyz/sdk<br/>wasm VERIFIER + envelope build<br/>no secrets, no server trust"]
    end
    subgraph DOMAIN["DOMAIN - Rust (operator infra)"]
        core["core (portable, native + wasm)<br/>verify_correspondence + action_commitment<br/>hashing + binding - VERIFY ONLY, never signs"]
        prover["mnemonic-prover<br/>zigz PRODUCE + evidence<br/>trusted only for SOUNDNESS"]
        mcpsrv["mcp server<br/>orchestrate produce to bind to anchor<br/>signs NOTHING (non-custodial)"]
    end
    subgraph INFRA["STORAGE + ANCHOR - NEUTRAL, UNTRUSTED"]
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

## 8. Policy lifecycle — authoring a policy into the registry

The registry is a **trust root**: the Intent binds a `policy_id`, and the whole
system is only as sound as the policy behind it. This is the *provenance of the
policy itself* — a distinct flow from the runtime prove/verify.

```mermaid
flowchart TD
    A["1. Author writes<br/>policy SOURCE<br/>(Rust/Zig guest,<br/>e.g. payment_mandate_v1)"]
    A --> C["2. REPRODUCIBLE<br/>compile (zigz build)<br/>deterministic -> RISC-V ELF"]
    C --> H["3. program_hash =<br/>blake3(ELF)<br/>content address = policy_id"]
    H --> R["4. Independent AUDIT<br/>of source<br/>(logic is TRUSTED - a bad<br/>policy approves bad actions)"]
    R --> P["5. Publish REGISTRY entry:<br/>name, policy_id, params_schema,<br/>version, publisher_sig<br/>entry ANCHORED<br/>(tamper-evident, versioned)"]
    P --> U["6. Principal Intent binds<br/>policy_id (immutable)<br/>Verifier checks<br/>program_hash == intent.policy_id"]
```

### Why it needs its own flow — the properties that make the registry trustworthy

- **Immutable + content-addressed.** A policy *is* its `program_hash`. A "new
  version" is a *new* hash, never an edit. So a registered policy can't be
  silently changed.
- **Reproducible build = verifiable trust.** Anyone can take the published source,
  rebuild deterministically, and confirm it hashes to `program_hash`. Trust is
  **checkable**, not authority-based. (Requires a pinned, reproducible build
  pipeline — a hard requirement, called out.)
- **Audit is the real trust root.** The hash proves *which* code ran; it does NOT
  prove the code is *safe*. A policy that always returns `ok=1` would pass anything.
  So a policy must be **audited** before registration — that human-audit judgment is
  what the `publisher_sig` attests.
- **No per-policy ceremony.** Because zigz is **transparent** (no trusted setup),
  registering a policy is just *publish source + reproducible ELF + hash* — no
  Groth16-style per-circuit ceremony. A genuine advantage over SNARK backends.
- **Intents bind the hash, not the name.** The signed Intent commits to
  `policy_id (= program_hash)` directly, so even if the registry's human name
  mapping (`payment_mandate_v1` → hash) is later repointed, **already-signed
  Intents are unaffected.** The name is a convenience; the hash is the contract.

### Governance choice (open question)

- **Open** — anyone publishes; Principals/Verifiers choose which policies /
  publishers they accept (like package registries). Maximally decentralized.
- **Curated** — a publisher authority signs a vetted policy set. Easier for
  regulated buyers who want an accountable auditor behind each policy.
- Recommended: **signed-by-publisher + open-source + reproducible**, with the
  relying party free to pin the exact `program_hash`(es) it trusts. Content
  addressing makes both models safe against silent tampering.

### Registry attack surfaces

| # | Attack | Mitigation |
|---|---|---|
| 1 | Backdoored policy that looks strict | open-source + independent audit + reproducible build (anyone re-derives the hash from source) |
| 2 | Registry operator repoints a name to a weaker hash | name→hash mapping is signed + anchored; Intents bind the **hash**, not the name |
| 3 | Non-deterministic build → hash ≠ source | pinned, reproducible build pipeline; published recipe |
| 4 | Params confusion (policy fed wrong-shaped params) | `params_schema` is part of the registry entry and bound into the Intent |
| 5 | Stale/withdrawn policy still used | `version` + optional revocation list; Verifier policy on accepted versions |
