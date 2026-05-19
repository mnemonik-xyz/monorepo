### 1 Shared Project Memory Namespace

Multiple A2A agents read from and write to a shared project-level memory namespace, so findings, decisions, contradictions, and source references accumulate on the project rather than inside any single agent. New agents joining the workflow retrieve accumulated context instead of starting from zero.
[See deep-dive in docs/usecases/shared-project-memory-namespace.md.]

### 2 Shared Memory Layer

Mnemonic acts as a persistent shared memory substrate underneath A2A coordination, surviving sessions, providers, and runtime changes while offering semantic retrieval and verifiable provenance. This replaces fragile context windows, ad-hoc databases, and vendor-locked memory with a portable common surface.
[See deep-dive in docs/usecases/shared-memory-layer.md.]

### 3 Provenance And Attestation Layer

Mnemonic records what an agent produced, what inputs it used, when it produced the output, and how the output connects to earlier artifacts, turning opaque message passing between agents into auditable knowledge production. Downstream consumers can independently check authorship, integrity, and timestamped existence of each claim.
[See deep-dive in docs/usecases/provenance-attestation-layer.md.]

### 4 Trust And Reputation Layer

Historical memory and contribution records can power trust signals — which agents are reliable in a domain, whose outputs are reused, which contributors are noisy or adversarial — that orchestrators use beyond declared capabilities. Mnemonic links agent identity, memory entries, downstream usage, and validation outcomes into a durable reputation surface.
[See deep-dive in docs/usecases/trust-reputation-layer.md.]

### 5 Portable Memory Wallet

Memory belongs to the agent or its operator rather than a provider: an operator can write memory while running on Claude, switch the runtime to GPT or a local model, and continue working from the same attested store without re-signing or re-attesting prior records. Memory snapshots are portable, verifiable, rehydratable, and independent from a single inference provider.
[See deep-dive in docs/usecases/portable-memory-wallet.md.]

### 6 Settlement-Aware Memory Infrastructure

Networked memory services need metering and payment; Mnemonic already supports balance and x402-style HTTP payment flows so agents can autonomously pay for memory writes, recall, and verification. This evolves into agent-payable memory infrastructure where verification remains open and paid operations sustain node operators.
[See deep-dive in docs/usecases/settlement-aware-memory-infrastructure.md.]

### 7 Task Memory Ledger

Each task exchanged in an A2A workflow leaves a durable record — request hash, assigned agent, summary, intermediate notes, output, artifact references, completion status, ordering anchors — that subsequent agents can retrieve. This prevents repeated context loss across the many short-lived tasks typical in multi-agent execution.
[See deep-dive in docs/usecases/task-memory-ledger.md.]

### 8 Artifact Attestation Service

Mnemonic attests, indexes, and retrieves artifacts produced by A2A workflows — reports, code patches, evidence bundles, recommendations, structured outputs — by storing artifact hash, producing identity, upstream references, and semantic summary. Consumers can later prove who produced an artifact, when, and from which inputs.
[See deep-dive in docs/usecases/artifact-attestation-service.md.]

### 9 Agent Continuity Layer

When an agent moves across runtimes, providers, or infrastructure because of cost, model upgrades, framework migration, or compliance, Mnemonic preserves prior memory items, project context, artifact history, and decisions so the agent retains accumulated context. Continuity is decoupled from the specific platform the agent runs on today.
[See deep-dive in docs/usecases/agent-continuity-layer.md.]

### 10 Reliability Oracle For Orchestration

Orchestrators query Mnemonic for memory-backed trust signals — accepted vs rejected outputs, downstream reuse, citation quality, contradiction rate, reviewer corrections — to route work beyond stated capabilities. Mnemonic holds the historical evidence needed to answer reliability questions about agents and contributions.
[See deep-dive in docs/usecases/reliability-oracle-for-orchestration.md.]
