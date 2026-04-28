# Research Report: Importance-Based Memory Eviction

**Date:** 2026-03-31
**Status:** Research complete, architecture decision pending
**Addresses:** Gap #10 — Memory pruning/eviction

---

## 1. Problem Statement

Agent memory cannot grow forever. As the memory store accumulates entries:
- Index size grows linearly (compressed: 1 byte/dim at 8-bit, 0.5 at 4-bit)
- Stage 1 search time scales linearly (~58us/memory in pure Python)
- On-chain snapshot cost scales with blob size (Arweave: ~$5/GB)
- Retrieval quality degrades as noise memories dilute relevance

We need a strategy to bound memory size while preserving the memories that matter most.

---

## 2. Can This Be Solved Universally?

**Short answer: partially.**

The eviction *framework* can be universal — a scoring function, a budget, and a removal policy. But the *weights and thresholds* within that framework are necessarily case-specific.

### What is universal

- **Composite scoring formula**: all systems converge on a weighted combination of recency, importance, relevance, and access frequency
- **Budget enforcement**: all systems need a max memory count or max byte budget
- **Multi-stage pipeline**: temporal pass → consolidation → importance-based eviction works across use cases
- **Decay mechanics**: exponential time decay is the standard primitive

### What is case-specific

- **Weight tuning**: a monitoring agent weights recency heavily; a research agent weights relevance heavily
- **Importance definition**: what "important" means depends on the domain (a decision memory vs. an observation)
- **Privacy sensitivity**: some deployments need privacy-aware eviction (accelerate removal of sensitive content)
- **Consolidation policy**: whether to merge similar memories or just delete low-value ones

**Recommendation:** Build a universal eviction engine with pluggable scoring weights and policies. Ship sensible defaults. Let deployments override.

---

## 3. Eviction Strategies (State of the Art)

### 3.1 Simple Temporal Policies

| Policy | How it works | Pros | Cons |
|--------|-------------|------|------|
| **FIFO** | Sliding window, drop oldest | O(1), predictable | Discards old-but-valuable memories |
| **LRU** | Drop least-recently-accessed | Keeps hot memories | Cold but important memories die |
| **TTL** | Fixed expiry per memory | Simple, no scoring | No nuance |

These are baselines. They work for bounded buffers but fail for long-term agent memory where old memories can be critically important (e.g., a decision made weeks ago).

### 3.2 Importance Scoring (Stanford Generative Agents Model)

The foundational approach from Park et al. (2023):

```
score(m) = w_r * recency(m) + w_i * importance(m) + w_v * relevance(m, query)
```

Where:
- **Recency**: exponential decay, `recency(m) = decay_rate ^ (now - m.created_at)`
- **Importance**: scored at ingestion time (LLM-assigned or heuristic, 0.0–1.0)
- **Relevance**: cosine similarity to current query context
- Each dimension normalized to [0, 1], weights sum to 1.0

**Default weights:** equal (0.33 each). Stanford used `decay_rate = 0.995` per hour.

**For eviction:** memories with the lowest composite score are candidates for removal when the store exceeds budget.

### 3.3 Priority Decay (MaRS Framework)

A more sophisticated scoring from the MaRS cognitive memory architecture:

```
score(m) = (utility(m) - λ_priv * sensitivity(m)) / token_cost(m)
```

Where:
- **utility** combines type weight, recency, and access frequency
- **sensitivity** is a privacy weight (higher = evict sooner)
- **token_cost** normalizes by memory size (prefer evicting large low-value memories)

The key insight: **normalize by cost**. A 500-token memory with moderate utility should be evicted before a 50-token memory with slightly lower utility.

### 3.4 Reflection-Summary Consolidation

Instead of deleting, **merge** related low-value memories into a compressed summary:

1. Cluster memories by semantic similarity
2. Summarize each cluster into a single "reflection" memory
3. Replace the cluster with the summary
4. The summary inherits the max importance of its constituents

**Letta's approach:** recursive summarization where older messages are progressively compressed. "Older messages have progressively less influence on the summary than recent messages."

This preserves information at reduced cost. Trade-off: summaries lose detail.

### 3.5 Learned Eviction (AgeMem — January 2026)

The most recent approach: **let the agent learn when to delete** via reinforcement learning.

- Agent has explicit `delete(memory_id)` tool
- RL reward includes a maintenance signal: `R_maintenance = 1 if (update or delete performed)`
- Agent learns autonomous deletion as a valuable strategy
- On Qwen3-4B, delete calls went from 0 to 0.22/episode after RL training

**Insight:** The agent itself is the best judge of what's obsolete. But this requires RL infrastructure and is not practical for all deployments.

### 3.6 Multi-Stage Pipeline (Recommended)

The most effective approach composes stages sequentially:

```
1. Temporal pass     → remove memories past hard TTL (e.g., > 90 days with no access)
2. Consolidation     → merge semantically similar low-importance memories
3. Importance eviction → score remaining, remove lowest until under budget
4. Privacy pass      → accelerate removal of high-sensitivity items (optional)
```

Each stage reduces the candidate set. This avoids a single scoring function needing to capture everything.

---

## 4. Scoring Function Design for Mnemonic

### Proposed composite score

```python
def eviction_score(memory, query_context, now):
    # Higher score = more worth keeping
    age_hours = (now - memory.created_at).total_seconds() / 3600
    recency = decay_rate ** age_hours                          # [0, 1]

    access_freq = min(memory.access_count / freq_cap, 1.0)    # [0, 1]

    importance = memory.importance_score                       # [0, 1] set at ingestion

    # Relevance to recent query distribution (optional, expensive)
    relevance = avg_similarity_to_recent_queries(memory)       # [0, 1]

    # Type bonus: decisions and semantic > episodic > observations
    type_bonus = TYPE_WEIGHTS.get(memory.memory_type, 0.0)    # [0, 0.3]

    raw = (w_r * recency
         + w_f * access_freq
         + w_i * importance
         + w_v * relevance
         + w_t * type_bonus)

    # Normalize by token cost (prefer evicting large low-value memories)
    return raw / max(memory.token_count, 1)
```

### Recommended default weights

| Weight | Value | Rationale |
|--------|-------|-----------|
| `w_r` (recency) | 0.25 | Recent memories are likely relevant |
| `w_f` (frequency) | 0.15 | Frequently accessed = actively useful |
| `w_i` (importance) | 0.30 | Ingestion-time importance is the primary signal |
| `w_v` (relevance) | 0.15 | Keeps memories aligned with current context |
| `w_t` (type bonus) | 0.15 | Structural preference for decisions over noise |

### Type weights

| Memory type | Bonus |
|-------------|-------|
| `decision` | 0.30 |
| `semantic` | 0.20 |
| `episodic` | 0.10 |
| `observation` | 0.00 |

### Decay rate

- Default: `0.995` per hour (half-life ~138 hours / ~6 days)
- Aggressive: `0.990` per hour (half-life ~69 hours / ~3 days)
- Conservative: `0.998` per hour (half-life ~346 hours / ~14 days)

---

## 5. Budget Enforcement

### Hard limits

- **Max memory count**: configurable, default 10,000
- **Max index bytes**: configurable, default 10MB compressed index
- **Eviction trigger**: when store exceeds 90% of budget
- **Eviction target**: reduce to 75% of budget (don't evict one-at-a-time)

### Eviction batch process

```
1. Compute eviction_score for all memories
2. Sort ascending (lowest score = least worth keeping)
3. Remove from bottom until store is at target capacity
4. Rebuild quantized index
5. Log evicted memory IDs for audit trail
```

### Interaction with on-chain commitments

- Evicted memories are NOT removed from prior Arweave snapshots (those are immutable)
- New snapshot after eviction reflects the reduced store
- The eviction log should be included in the snapshot metadata
- This means old snapshots can be used to recover evicted memories if needed

---

## 6. Architecture Decision: Universal Framework, Pluggable Policy

```
┌─────────────────────────────────────────┐
│           EvictionEngine                │
│                                         │
│  ┌───────────┐  ┌────────────────────┐  │
│  │  Budget    │  │  EvictionPolicy    │  │
│  │  Monitor   │  │  (pluggable)       │  │
│  │           │  │                    │  │
│  │  - max_n  │  │  - score(memory)   │  │
│  │  - max_mb │  │  - should_merge()  │  │
│  │  - trigger│  │  - merge(cluster)  │  │
│  │  - target │  │  - on_evict(id)    │  │
│  └───────────┘  └────────────────────┘  │
│                                         │
│  Built-in policies:                     │
│  - DefaultPolicy (weighted composite)   │
│  - RecencyOnlyPolicy (pure LRU)         │
│  - AggressivePolicy (fast decay + TTL)  │
│  - ConservativePolicy (slow decay, high │
│    importance threshold)                │
│  - CustomPolicy (user-defined weights)  │
└─────────────────────────────────────────┘
```

### Interface

```python
class EvictionPolicy:
    def score(self, memory: MemoryItem, context: EvictionContext) -> float:
        """Higher = more worth keeping."""
        ...

    def should_consolidate(self, cluster: List[MemoryItem]) -> bool:
        """Whether to merge similar memories instead of deleting."""
        return False

    def consolidate(self, cluster: List[MemoryItem]) -> MemoryItem:
        """Merge a cluster into a single summary memory."""
        ...
```

### Configuration example

```json
{
  "eviction": {
    "policy": "default",
    "max_memories": 10000,
    "trigger_pct": 0.90,
    "target_pct": 0.75,
    "weights": {
      "recency": 0.25,
      "frequency": 0.15,
      "importance": 0.30,
      "relevance": 0.15,
      "type_bonus": 0.15
    },
    "decay_rate": 0.995,
    "consolidation_enabled": false,
    "ttl_hours": null
  }
}
```

---

## 7. Answer: Universal or Case-Specific?

| Aspect | Universal | Case-specific |
|--------|-----------|---------------|
| Scoring formula structure | Yes | |
| Budget enforcement logic | Yes | |
| Multi-stage pipeline | Yes | |
| Exponential decay primitive | Yes | |
| Weight values | | Yes |
| Decay rate | | Yes |
| Type bonus mapping | | Yes |
| Consolidation policy | | Yes |
| Privacy sensitivity | | Yes |
| TTL thresholds | | Yes |

**The framework is universal. The configuration is not.**

Build the engine once, parameterize everything, ship sensible defaults.

---

## 8. Recommendation for Mnemonic

### Phase 1 (MVP eviction)
- Implement `DefaultEvictionPolicy` with the weighted composite score
- Add `max_memories` budget to `MemoryStore`
- Trigger eviction at 90%, target 75%
- Log evicted IDs
- Track `access_count` and `last_accessed_at` on `MemoryItem`
- No consolidation yet

### Phase 2 (after benchmarks)
- Add consolidation (merge similar memories)
- Tune weights based on retrieval quality metrics
- Add per-deployment policy configuration
- Add TTL support for regulatory/compliance use cases

### Phase 3 (if needed)
- Learned eviction via RL (AgeMem-style)
- Privacy-weighted eviction for sensitive deployments
- Tiered storage (evicted memories archived to cold storage, not deleted)

---

## 9. Sources

- [AgeMem: Agentic Memory — Unified LTM/STM Management (Jan 2026)](https://arxiv.org/abs/2601.01885) — RL-based learned memory deletion
- [MaRS: Forgetful but Faithful — Cognitive Memory with Privacy-Aware Eviction](https://arxiv.org/html/2512.12856v1) — multi-policy eviction with privacy weighting
- [Letta: Agent Memory — How to Build Agents that Learn and Remember](https://www.letta.com/blog/agent-memory) — recursive summarization, context engineering
- [AWS AgentCore: Agent Memory Strategies](https://dev.to/aws-builders/agent-memory-strategies-building-believable-ai-with-bedrock-agentcore-kn6) — Stanford-style importance scoring
- [Redis: LFU vs LRU Cache Eviction Policies](https://redis.io/blog/lfu-vs-lru-how-to-choose-the-right-cache-eviction-policy/) — hybrid cache strategies
- [Memory Architectures for Long-Term AI Agent Behavior](https://www.gocodeo.com/post/memory-architectures-for-long-term-ai-agent-behavior) — multi-tier memory design
- [Evaluating Memory Structure in LLM Agents (Feb 2026)](https://arxiv.org/pdf/2602.11243) — empirical comparison of memory strategies
- [Memory in the Age of AI Agents: A Survey](https://github.com/Shichun-Liu/Agent-Memory-Paper-List) — comprehensive paper list
