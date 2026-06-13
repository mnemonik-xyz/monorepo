---
created: 2026-06-13
status: research
size: M
branch: feat/delm-integration
sources:
  - DeLM paper (arXiv:2606.10662, "Decentralized Multi-Agent Systems with Shared Context", Mao & Mirhoseini, Stanford, 2026-06)
  - mnemonik-dev/DeLM @ 2d0ea48 (README, src/shared_lessons.py, src/verifier.py, src/memory_compactor.py, src/modes.py, src/agents/)
  - mnemonik-xyz/monorepo (docs/WHITEPAPER.md §5.7/§8.2, core/src/codec/schema.rs, core/src/lineage/mod.rs, mcp/src/tools.rs)
---

# DeLM vs. Mnemonic — Findings & Integration Direction

## TL;DR / Verdict

- **DeLM does not kill Mnemonic.** Despite the name "Decentralized Language Models,"
  the paper is *not* about decentralized model training and has **no blockchain or
  cryptography**. Its real title is "Decentralized Multi-Agent Systems with Shared
  Context." "Decentralized" means **no central orchestrator agent** — parallel agents
  coordinate through a shared context + task queue. It is a *coordination / test-time
  scaling algorithm*, not a *memory protocol*. Different layer of the stack.
- **The deep relationship is complementarity, anchored on two different meanings of
  "verified":** DeLM verifies *content veracity* (is this claim supported by its
  evidence? — LLM check + verbatim quote grounding). Mnemonic verifies *cryptographic
  authenticity* (are these exact bytes authentic, attributable, timestamped,
  untampered? — Ed25519 + BLAKE3 + lineage + anchor). Mnemonic's whitepaper §8.2
  **explicitly declares content veracity out of scope** — exactly the gate DeLM
  provides.
- **Highest-leverage move:** "DeLM-on-Mnemonic" — Mnemonic persists, signs, and
  anchors DeLM's shared context (and its gist→summary→raw hierarchy) via the existing
  lineage DAG. The paper's own future-work (cross-run automated research) is this use
  case.
- **Best low-risk borrows into Mnemonic:** (1) admission-time veracity gate *before
  signing*; (2) verbatim ref-tag claim grounding; (3) DeLM's deterministic zero-LLM
  compactor; (4) finer outcome-oriented typing.
- **Non-goal:** do **not** reimplement DeLM orchestration inside `core/` — it belongs
  in the agent runtime, and chasing "within-run scratchpad" undercuts Mnemonic's moat
  (cheaper without crypto in that lane).

---

## 1. What DeLM actually is (from the full paper)

**Problem.** Multi-agent systems (MAS) scale LLM reasoning at test time by splitting a
task into parallel subtasks. Most MAS use **centralized orchestration**: a main agent
assigns work, collects outputs, merges, rebroadcasts. As subtasks grow, that controller
is a serial communication + integration bottleneck, and it can dilute/omit/distort
useful findings in transit.

**Design.** DeLM replaces the controller with two global structures — a **shared
context `C`** and a **task queue `T`** — plus parallel agents. Five-stage loop:
1. initialize the task queue from the input,
2. run ready subtasks in parallel,
3. compress + verify + admit results as compact **gists `Gᵢ`** into `C`,
4. generate more subtasks when `C` is insufficient,
5. finalize the answer once the queue drains.

**Three principles:**
- **Shared state, not prompt-routing.** Progress persists in `C` as reusable problem
  state instead of being merged/rebroadcast by a manager.
- **Compact, global, *unfoldable* context.** `C` holds only compact gists. A 3-level
  hierarchy **raw `rᵢ` → reference-grounded summary `Sᵢ` → gist `Gᵢ`** lets agents read
  a cheap global view and **selectively unfold** (`UNFOLD`/`DEEP_UNFOLD`) to summary or
  raw on demand. They call it "demand paging"; only `Gᵢ` lives in `C`, `Sᵢ`/raw in
  backing stores `L`/`R`.
- **Verified before admission.** Nothing enters `C` until an **LLM verifier** checks it
  against its cited evidence; unsupported/distorted updates are rejected or regenerated.

**Results.** SWE-bench Verified (Gemini 3 Flash): **65.7% Avg@1** (+10.5 pp over
AOrchestra), **77.4% pass@4**, **$0.12/task (~half** the agentic baselines); with Claude
Opus 4.6, 78.0% Avg@1. LongBench-v2 Multi-Doc QA: best average across four model
families (up to +5.7 pp). **Ablation headline: removing admission-time verification is
the single largest accuracy drop (60.1% → 55.2%)** — bigger than removing the whole
hierarchy (→57.7%). DeLM + RLM hybrid beats either alone (complementarity theme).

**Crucially, what DeLM is NOT:** no persistence across sessions (Appendix A.4: `C` is an
in-process structure — lock-free snapshots, atomic appends, KV-cache prefix reuse), no
provenance beyond a `thread_id` string, no cryptography. The paper files itself next to
MemGPT / Mem0 / MemOS / LongMem and differentiates only on "organizes shared info by
abstraction level."

## 2. What Mnemonic is (for contrast)

A **cryptographic memory substrate**: typed artifacts → embed → TurboQuant-compress →
canonical CBOR → BLAKE3 CID → Ed25519 `COSE_Sign1` → lineage DAG → optional
Arweave/Solana anchoring → exposed over MCP. "Verifiable" = deterministic, out-of-band,
$O(1)$ cryptographic verification of authorship, integrity, lineage, and timestamp, by
any third party. "Decentralized" = storage-agnostic, operator-plural, optionally
on-chain. Optimizes long-horizon trust & portability — **across** sessions, models,
operators.

---

## 3. Side-by-side

| Dimension | **DeLM** | **Mnemonic** |
|---|---|---|
| Category | Coordination / test-time scaling algorithm | Memory persistence & verification protocol |
| "Decentralized" means | No central orchestrator agent | Storage-agnostic, operator-plural, optionally on-chain |
| Layer | Orchestration / runtime (above the model) | Data substrate (beneath the agent) |
| Unit | A gist `Gᵢ` (FACT/FAIL/CLAIM/PATCH_SUMMARY…) | A signed typed artifact (rag.context/result, agent.state, receipt, memory) |
| Time horizon | **Within a single task run** (ephemeral RAM) | **Across sessions/models/operators** (durable, content-addressed) |
| Sharing scope | Threads inside one task, one process | Agents/runtimes/operators across trust boundaries |
| "Verified" | **Content veracity** — claim supported by cited evidence (LLM + verbatim quote) | **Cryptographic authenticity** — sig + hash + lineage + anchor |
| Persistence | In-memory list / optional JSONL | SQLite → Arweave → Solana |
| Provenance | `thread_id` string | Ed25519 / DID, non-repudiable |
| Retrieval | Regex dedup + token-budgeted priority window + unfold | Embedding cosine recall + decompress-on-recall |
| Cryptography | None | The entire point |

## 4. The crux: two orthogonal meanings of "verified"

This is the single most important finding and it drives every recommendation.

- **DeLM "verified"** answers *"is this claim TRUE / SUPPORTED?"* — an LLM verifier plus
  a **verbatim ref-tag** mechanism (`[ref: h…t]`: the first and last ≥5 words of the
  supporting span must appear, in order and verbatim, in the source). Caught failure:
  hallucinated / unsupported content. Trust model: trust the verifier model, within one
  run.
- **Mnemonic "verified"** answers *"are these exact bytes AUTHENTIC / UNTAMPERED /
  ATTRIBUTABLE / TIMESTAMPED?"* — Ed25519 `COSE_Sign1` + BLAKE3 CID + lineage DAG +
  optional anchor. Caught failure: tampering, forgery, repudiation, backdating. Trust
  model: trustless, out-of-band, by anyone, forever.

They are **orthogonal and composable**. A signed-but-false memory is worthless; a
grounded-but-ephemeral note can't cross a trust boundary or a session. Real agent memory
needs both. And **Mnemonic's whitepaper §8.2 lists "Semantic Veracity of Content" as an
explicit non-guarantee** — DeLM's admission-time verifier is precisely the missing piece.

---

## 5. The four questions

### 5.1 Does Mnemonic's idea die with DeLM? — No.
Different category (within-run coordination vs. cross-time/runtime verifiable memory).
DeLM's `C` is single-run RAM with no crypto and no cross-session persistence; it is a
coordination algorithm, not a protocol. Overlap is only the surface phrase "shared
context." If anything DeLM *validates* the thesis that agent memory must be curated
before reuse.

**Strategic caveat (honest):** DeLM shows that for the *within-task* scratchpad, a cheap
in-RAM verified blackboard wins and crypto would only add cost. So Mnemonic should not
position itself as "the shared scratchpad for a multi-agent run" — that lane belongs to
DeLM/AOrchestra/blackboard MAS. Mnemonic's moat is exactly what DeLM disclaims: when
findings must outlive the run, move to another runtime, or be proven to a third party.

### 5.2 Can Mnemonic contribute TO DeLM? — Yes (strongest fit).
DeLM's gaps map 1:1 onto Mnemonic primitives, and the gist→summary→raw hierarchy is a
perfect fit for Mnemonic's lineage DAG (parent-child content links):
- ephemeral `C`/`S`/raw → durable typed artifacts with lineage edges gist→summary→raw;
- `thread_id` provenance → Ed25519 authorship + anchored timestamp;
- verbatim ref-tag → a *signed* claim-to-source binding that survives the process;
- the paper's own future-work (automated research: "prevent agents from repeatedly
  reading the same papers or rerunning the same failed analyses") is a *cross-run
  persistent* shared context — i.e. Mnemonic, not DeLM.

### 5.3 Can Mnemonic BORROW from DeLM? — Yes (named, low-risk).
- **Admission-time veracity gate before signing** (DeLM §3.2/A.3). Mnemonic's
  `participate` mode already gates on a recall+verify round-trip (*does it read back?*)
  but never on *is the content supported?*. Add a DeLM-style evidence gate as a pre-sign
  filter — fills the §8.2 hole and avoids paying to anchor unsupported claims.
- **Verbatim ref-tag grounding** — a cheap **deterministic** (non-LLM) claim→span bind
  for `rag.result`, fitting Mnemonic's determinism ethos better than an LLM check.
- **Deterministic zero-LLM compactor** (`memory_compactor.py`) — regex extraction of
  edited files / failing tests / exception classes / line refs / PATCH_SUMMARY. Directly
  reusable as a pre-embed content normalizer/compactor.
- **Outcome-oriented typing** (FACT/FAIL/CLAIM/PATCH_SUMMARY) enriches the thin `memory`
  schema.

### 5.4 Implement the idea? — Integrate beneath it; don't reimplement in core.
Reimplementing DeLM orchestration inside `core/` would violate the `core`/`mcp` split
and Mnemonic's no-orchestration scope. Build the adapter + the borrows. We own both
repos (`mnemonik-xyz/monorepo`, `mnemonik-dev/delm`), so a joint prototype is low
friction.

---

## 6. Concrete integration anchors (grounded in real code)

> Note: Mnemonic's **code** is narrower than its whitepaper. `core/src/codec/schema.rs`
> today defines only `rag.context`, `rag.result`, `agent.state`, `receipt`, `memory`.
> The five cognitive types (episodic/semantic/procedural/working/identity) and
> `capability.token` are whitepaper-aspirational, not yet built. Anchors below reference
> what exists.

**DeLM side (`mnemonik-dev/delm`, Python):**
- `src/shared_lessons.py` — `SharedLessons` blackboard: `read()` / `admit`/write, async
  `asyncio.Lock`, JSONL persistence, regex semantic dedup, priority window. *Integration
  seam for the adapter.*
- `src/verifier.py` — `verify_notes` (deterministic phrase filter) + `verify_notes_llm`
  (LLM evidence check). *Borrow target #1 (admission gate).*
- `src/memory_compactor.py` — `extract_structured_summary(records)`: deterministic,
  zero-LLM regex compactor. *Borrow target #3.*
- `src/agents/{planner_agent,swebench_implementer_agent}.py`, `src/modes.py`
  (`ModeSpec`). *Orchestration — stays in DeLM, not ported.*

**Mnemonic side (`mnemonik-xyz/monorepo`, Rust):**
- `core/src/codec/schema.rs` — `ArtifactType` enum, `ParentRef { artifact_id, role }`
  (roles `context|state|trigger|dependency`), `MAX_PARENTS=16`, `MAX_DEPTH=64`. *A
  DeLM gist becomes a `rag.result`/`memory` artifact; gist→summary→raw becomes a
  3-node lineage chain via `ParentRef`.*
- `core/src/lineage/mod.rs` — `record_parents`, `get_parents`, `get_children`,
  `traverse_lineage(Direction)`, chain verification. *Backs the unfold hierarchy.*
- `mcp/src/tools.rs` — `WriteMode` (`Local`/`Participate`), `resolve_write_mode`,
  the `participate` recall+verify gate. *Where the pre-sign veracity gate (borrow #1)
  slots in.*
- `mcp/src/tools.rs` MCP tools `mnemonic_sign_memory` / `mnemonic_recall`. *The
  read/write API the DeLM adapter calls over MCP.*

**Mapping (DeLM → Mnemonic):**

| DeLM concept | Mnemonic representation |
|---|---|
| gist `Gᵢ` admitted to `C` | signed `rag.result` (or `memory`) artifact |
| summary `Sᵢ` (backing `L`) | signed artifact, parent of the gist (`role:"context"`) |
| raw `rᵢ` (backing `R`) | signed `rag.context` artifact, parent of the summary |
| `UNFOLD` / `DEEP_UNFOLD` | `traverse_lineage(Ancestors)` + recall |
| admission-time verify | pre-sign veracity gate, then COSE sign |
| `thread_id` provenance | Ed25519 producer / DID in the COSE envelope |
| verbatim ref-tag | citation field on `rag.result`, deterministically checkable |

## 7. Recommended next steps

Pattern after the existing `work/a2a-bridge` feature (schema additions in `core` + a thin
adapter + MCP tools/SDK helpers — zero new crypto primitives, all reuse).

1. **Adapter (highest leverage):** a thin layer so DeLM's `SharedLessons.read()/admit()`
   read/write through `mnemonic_recall` / `mnemonic_sign_memory` over MCP, persisting the
   gist→summary→raw hierarchy as lineage-linked artifacts. Prototype lives in the `delm`
   repo (client side); no `core/` changes required for a first cut.
2. **Borrow — pre-sign veracity gate:** spec a content-veracity check in the
   `participate` path (`mcp/src/tools.rs`) before COSE signing. Closes whitepaper §8.2.
3. **Borrow — deterministic compactor + ref-tags:** port `memory_compactor.py`'s
   approach as a pre-embed normalizer; add a deterministically-checkable `citations`
   binding on `rag.result`.

**Non-goals:** no DeLM orchestration in `core/`; don't market Mnemonic as a within-run
scratchpad; don't change TurboQuant bit width or any existing schema version.

## 8. Sources

- DeLM paper, arXiv:2606.10662 (full text reviewed).
- `mnemonik-dev/DeLM` @ `2d0ea48` — README, `src/shared_lessons.py`, `src/verifier.py`,
  `src/memory_compactor.py`, `src/modes.py`, `src/agents/`.
- `mnemonik-xyz/monorepo` — `docs/WHITEPAPER.md` (§5.7 economics, §8.2 non-guarantees),
  `core/src/codec/schema.rs`, `core/src/lineage/mod.rs`, `mcp/src/tools.rs`.
