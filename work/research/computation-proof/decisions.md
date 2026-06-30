# Decisions — computation-proof

Append-only log. Each entry: date, who, what, why, downstream.

---

## 2026-06-29 — Feasibility memo created

Author: claude. Origin: thread proposing Gabbay-style (arXiv 2606.23768) SNARK
policy certificates on the Mnemonic envelope. Conclusion: buildable now; policy
circuits scale with rule complexity, not model size; slots into the
verifiable-trajectories "bind by hash, no prover in core/" decision as a new
proof_kind, adding one net-new primitive — a *verifier* in core/. See
`feasibility.md`.

---

## 2026-06-29 — Reframe: policy-compliance → intent–action correspondence

Author: claude. Trigger: competitive analysis of **Delta Network** (spender-side
settlement enforcement; zkTLS-native SDK; SP1 prover; rides AP2 Intent/Cart/
Payment Mandate model). Delta targets exactly the owner's goal — provable
correspondence between an agent's action and the principal's signed intent.

**What changes.** The feature's framing generalises from "prove action satisfies a
policy" to "prove action ⊨ the principal's signed Intent." The "policy" becomes a
principal-signed **`INTENT_V1`** mandate (aligned with AP2, not reinvented). See
`positioning.md` for the 5-layer decomposition and ownership table.

**Two earlier conclusions updated:**
- Oracle problem is answered by **zkTLS** (TLS transcript = attestation, no
  merchant integration). Supersedes the "wait for signed feeds" framing.
- The DSL→circuit path is now *one* option; a **zkVM (SP1)** — policy as a
  program, prove execution — is the rival, more pragmatic path for evolving logic.
  Mnemonic stays prover-agnostic and verifies whichever.

---

## 2026-06-29 — Decision: compose, do not compete (recommended; pending owner)

Mnemonic **owns layers 1 (Intent), 4 (binding/anchoring), 5 (knowledge link)**,
**verifies** layer 3 (correspondence proof) in core/ via new `proof_kind:
sp1 | zktls` alongside `snark`, and **does not build** layer 2 (Evidence/zkTLS) or
a zkVM prover.

Why: Mnemonic's defensible edges are open-source + permanent anchoring + the
knowledge layer Delta explicitly cedes. Rebuilding a funded competitor's closed,
specialised zkTLS+zkVM stack is the trap. Positioning: *Delta is the turnstile;
Mnemonic is the permanent, open, re-verifiable record that survives the
transaction.*

Optional later wave: an open-source self-hosted SP1 reference prover, so users
aren't forced through a closed hosted API. Full-compete rejected for now.

Downstream: extend `feasibility.md`'s envelope-binding design with `INTENT_V1`,
`intent_hash` linkage, public-input commitment to (intent_hash, action_hash,
evidence), and a `mnemonic_verify_correspondence` surface. The benchmark Delta
cites (28.8%→0%) is unverified — never repeat as fact.

---

## 2026-06-30 — Decision REVERSED: full compete (owner call, after rethink)

Author: claude, recording owner decision. Supersedes the 2026-06-29
"compose, do not compete" recommendation. Owner chose **full compete** after
rethinking: Mnemonic will *produce*, not merely verify, the intent–action
correspondence proof — including an Evidence Layer (zkTLS) and a zkVM prover.

**Scope taken on (the two net-new layers):**
- **Layer 2 — Evidence/zkTLS.** The hard, operationally heavy piece (MPC-TLS
  notaries or TEE attestation infra). Mitigation: build on **TLSNotary** (Rust,
  open) rather than from scratch; phase it behind an `EvidenceSource` trait so
  the stack runs end-to-end with a stubbed/trusted evidence step first.
- **Layer 3 — correspondence prover (zkVM).** Policy evaluation as a Rust guest
  program; prove execution. Backend candidates: **SP1** (matches the competitor;
  strong tooling; Groth16 wrapper for cheap verify) or **RISC Zero** (more
  mature). Decision pending.

**Architecture-preserving constraint (hard).** The prover lives in a NEW
workspace member (`prover/`), NOT in `core/`. `core/` stays native-only,
pure, and **verify-only** — the "no prover in core/" rule (2026-06-27) and the
one-way `core → mcp` dependency both stand. MCP orchestrates: `prover/` produces
→ `core/` verifies + binds + anchors. The lightweight open verifier remains the
moat even while we add production.

**Differentiation (why compete is not a doomed clone).** Delta is closed alpha,
hosted, no permanence, no knowledge layer. Mnemonic-compete = the **open-source,
AP2-aligned, permanently-anchored, knowledge-linked** correspondence stack — "the
open Delta." Compose paths (binding third-party/Delta proofs) remain supported via
`proof_kind`, not removed — compete is additive.

**Biggest risk on record.** zkTLS operational + specialist cost. If notary/TEE
ops prove too heavy, fall back to compose for Layer 2 (bind external zkTLS proofs)
while still producing Layer 3 — i.e. partial compete. Revisit at end of Wave 3.

---

## 2026-06-30 — Decision: zigz is the prover backend; pure-Rust verifier (option C)

Author: claude, recording owner direction after cloning `mnemonik-dev/zigz`.

**zigz** (owner's own Jolt-inspired Zig zkVM: sumcheck + Lasso, Binary Merkle
commitments, **transparent / no trusted setup**, post-quantum, RISC-V RV64IM,
already hardened vs the Jolt "unfaithful-claims" Fiat-Shamir bug) **replaces the
SP1 assumption** as the correspondence-proof backend. The whole stack is now ours
and open — the actual substance of "compete." Transparency also **removes the
Groth16 trusted-setup concern** recorded in `feasibility.md`.

**Verifier path = option C: a pure-Rust re-implementation of the zigz verifier in
`core/`** — NOT FFI-to-Zig (a) and NOT CLI shell-out (b). Why C wins:
- Only C compiles to **WASM/browser** → preserves client-side verification, the
  stated direction and the moat. (a)/(b) cannot.
- Independent Rust verifier ⇒ **differential testing** against the Zig prover —
  the discipline that catches transcript/Fiat-Shamir bugs.
- Keeps `core/` pure-Rust, zero non-Rust build/runtime dependency.

**Enabling commitment:** because zigz is ours, we **freeze a versioned
`zigz-proof-v1` serialization**; the Rust verifier targets it; CI carries
differential conformance vectors `{program, public_inputs, π}` that BOTH verifiers
must agree on. This neutralises the only real argument against C (format churn).

**Tradeoffs on record:** zigz proofs are **~7–40 KB for policy-sized programs**
(measured 2026-06-30, zigz built + run: serialized ~7 KB for a 4-step program;
in-memory estimate ~29→77 KB over 16→4096 steps; verify ~11 ms, O(log n)) — a
non-issue for the
envelope (32-byte `proof_ref` only), the Solana memo, off-chain verify, and Arweave
storage; impractical only for on-chain contract verification → anchor π on Arweave,
verify off-chain (already the design). On-chain later needs a STARK→SNARK wrap
(reintroduces trusted setup). zigz is unaudited → stays behind
`correspondence-experimental`; no production claims.

Downstream: `proof_kind` gains `"zigz"`; `core/correspondence/zigz.rs` (feature
`corr-zigz`); `prover/src/prove/zigz.rs`; Wave 2 freezes the format + lands the
pure-Rust verifier with differential vectors. SP1 references removed from the spec.

---

## 2026-06-30 — Decision: v1 design-partner vertical = agentic payments

Author: claude, recording owner choice. v1 targets **agentic payments**: live
market, AP2/Delta momentum proves demand, zkTLS evidence path is well-trodden.

Pins the build: first zigz guest program = **`payment_mandate_v1`** (amount ≤ cap;
currency/category allowed; merchant ∈ allowlist; within expiry; **action fields ==
merchant-authenticated evidence** — the binding clause that makes it a proof about
reality, the Delta lesson). `INTENT_V1` aligns with the AP2 Intent Mandate;
`ACTION_V1` with Cart/Payment. Evidence: StubEvidence (W1–2) → zkTLS over
merchant/PSP (W3). Success metric = a published purchase-intent benchmark
(compliant verifies / non-compliant fails), no unverifiable % claims. Full scope:
`v1-agentic-payments.md`.

---

## 2026-06-30 — Finding: zigz has no recursion/folding today; v1 doesn't need it

Author: claude, from source inspection of the cloned `mnemonik-dev/zigz`.

**Finding.** zigz does **not** support proof recursion or folding/IVC today. All
references are roadmap, not code: `MODULES.md` lists `src/recursion/` under
"Future Extensions" (and the directory does not exist); `VERIFIER.md` puts
"Proof compression and recursion" under "Remaining work (Phase 10)";
`CONTRIBUTING.md` lists "Proof aggregation/recursion" as a Low-Priority
contribution. Today zigz produces **monolithic proofs of a single RISC-V program
execution**.

**Impact on complex/stateful intents — none for v1.** A bounded multi-action
stateful intent is one bigger RISC-V program (loop over actions, accumulate,
check aggregate + per-step + sequencing) → one monolithic proof. Cost is ~O(log n)
in size (measured: 4096 steps ≈ 77 KB), fine for dozens–thousands of actions.

**Unbounded / long-running intents — covered without recursion via checkpoint
state-chaining.** Prove in bounded checkpoint batches; each batch carries the
accumulator as a public-input commitment; `batch[i].out_state ==
batch[i+1].in_state`; verifier checks each batch + the linkage. Reuses the
`verifiable-trajectories` checkpoint / root-of-roots machinery. Cost: O(batches)
to store/verify (vs one constant-size IVC proof). Available today.

**Extending zigz with recursion/folding (owner R&D track, NOT v1-blocking):**
- **Path A — recursion via verifier-as-guest (recommended first).** Compile the
  existing O(log n) zigz verifier to a RISC-V guest and prove its execution →
  aggregate/compress proofs. No new proof system. Main cost = in-VM hashing;
  mitigated by Poseidon2 (already available via the `hash-zig` dep) + a hash
  precompile. "Engineering hard," not "research hard." Est: weeks–months.
- **Path B — folding (Nova/ProtoStar-style).** Research-grade: zigz's hash-based
  Merkle commitments + sumcheck/Lasso do **not** fit classic homomorphic-
  commitment Nova folding; needs an accumulation scheme compatible with
  sumcheck+lookups (ProtoStar-ish / split-accumulation). Higher effort + risk.
- Recursion also unlocks the on-chain STARK→SNARK wrap and proof aggregation
  (many attestations → one proof) — product value at scale, later.

**Decision:** v1 = monolithic-bounded proofs + checkpoint state-chaining for
unbounded intents. Recursion/folding is a **deferred zigz R&D track** (Path A
first), tracked on the zigz side; it is a performance/scale upgrade, not a
capability prerequisite. Flag as a feature request in `mnemonik-dev/zigz`.

---

## 2026-06-30 — Spike result: stateful multi-action intent proves+verifies on zigz

Author: claude. Built + ran a real zigz guest (`payment_mandate`) proving a
mandate over a *sequence* of payments: stateful `Σ amounts ≤ cap` + per-action
cap + vendor allowlist membership + non-decreasing timestamps. Source + full
table: `spikes/zigz-stateful-intent/`.

**Measured (zig 0.15.2, ReleaseSmall guest):** compliant → committed ok=1; each
violation (over aggregate cap / off-allowlist / out-of-order ts) → ok=0, correct
totals. Verify 35–89 ms (flat, O(log n)); proof ~31 KB (4 pays) → ~53 KB (50
pays); prove 0.8–1.5 s small, **~24 s at 2096 steps (50 pays)**.

**Conclusions:**
- "Intent more complex than a tx" is **validated** — stateful, multi-action,
  multi-constraint policy proves and verifies, with the policy actually computed.
- Verify cost + proof size are non-issues; **proving is ~linear in steps** and is
  the scaling bottleneck on the current unoptimized VM.
- v1-sized intents (handful of actions) are fine today (~1–2 s). Large/long
  intents → empirical motivation for the recursion/aggregation track
  (`zigz-recursion/spec.md`) and checkpoint-batching. No blocker for v1.
