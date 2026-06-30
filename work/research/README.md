# Research — verifiable agent actions

Design + research corpus (June 2026) for proving that an AI agent's **actions
cryptographically correspond to the principal's signed intent**. Grouped here as a
self-contained body of work (pre-staging an eventual carve-out to its own repo).

- **`protocol/`** — `design.md` (self-verifying objects, no nodes, pluggable
  anchor, durability classes) + `business-model.md` (who pays; open = the wedge).
- **`computation-proof/`** — the build: `feasibility.md`, `positioning.md` (vs
  Delta / AP2), `tech-spec.md` (intent–action correspondence, zigz backend,
  pure-Rust verifier), `v1-agentic-payments.md`, `decisions.md` (append-only log),
  and `spikes/zigz-stateful-intent/` (measured proof that stateful multi-action
  intents prove + verify on zigz).
- **`zigz-recursion/`** — `spec.md`: scaling zigz with recursion / folding /
  aggregation (intended for the `mnemonik-dev/zigz` repo).

Reading order: `protocol/design.md` → `protocol/business-model.md` →
`computation-proof/tech-spec.md` → `computation-proof/decisions.md`.

Related (left in `work/`, repo-hygiene not research): `monorepo-refactor/`.
