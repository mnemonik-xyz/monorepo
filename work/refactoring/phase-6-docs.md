---
phase: 6
title: Docs + rules
status: ready
risk: low
depends_on: [5]
---

# Phase 6 — Docs + rules

## Objective

Make the documentation and the audit-enforced dependency rules describe the new
five-crate / five-package layout, then run the audit waves + QA gate so the
one-way DAG is *enforced*, not just *intended*.

## Files moved

None (docs already live in `docs/`). This phase **edits** docs and rule files to
match the new structure.

## Exact edits

### 6.1 — `CLAUDE.md`

Update these sections to the new layout (paths in the current `CLAUDE.md` still
say `core/`, `mcp/`, `mcp/src/...`):

- **"## Project" / "## Architecture":** replace the two-member description
  ("`core/` … `mcp/`") with the five Rust crates under `crates/`
  (`core` portable rlib, `native`, `wasm` cdylib, `prover`, `mcp`) and the five
  packages under `packages/` (`sdk`, `cli`, `extension`, `mcp`, `webapp`).
  State the one-way DAG: everything → portable `core`; `wasm` → core only;
  `mcp` → core + native; `prover` → core only.
- **"Hard architectural rules":** generalize rule 5 ("`core/` has zero
  references to anything in `mcp/`") into the full DAG invariant:
  `crates/core` references nothing else in the workspace;
  `crates/wasm` and `crates/prover` reference only `core`;
  `crates/native` references only `core`; `crates/mcp` may reference `core` +
  `native` (+ optionally `prover`). Update rule 1's payment-method file path
  `mcp/src/payment.rs` → `crates/mcp/src/payment.rs`; rule 3
  `pricing.rs lives in mcp/` → `crates/mcp/`.
- **Data-flow + storage paragraphs:** `core/src/...` paths → `crates/core/...`
  and `crates/native/...` (e.g. `storage::AttestationStore` is now
  `mnemonic_native::storage`).
- **"## Common commands":** `cargo test -p mnemonic-core --test ...` examples
  still work (name-addressed); add `cargo build -p mnemonic-native` and the
  wasm build (`bash scripts/build-wasm.sh` → `crates/wasm`). Update the
  `cargo run -p mnemonic-mcp` blocks (paths unchanged, name-addressed).
- **"## CI gate policy":** update the `cross-lang-build` description —
  it now builds `mnemonic-mcp`, the `mnemonic-native --example
  keychain-roundtrip` (was `mnemonic-core`), and the SDK WASM from
  `crates/wasm`. **Keep the yo-yo-prevention rule verbatim** — the
  `continue-on-error: true` on `cross-lang-keychain` stays permanent; this phase
  must not relax it.
- **"## Conventions":** add `feat(native):`, `feat(wasm):`, `feat(prover):`
  to the conventional-commit scope list alongside `feat(core):`/`feat(mcp):`.

### 6.2 — `AGENTS.md` + `.claude/skills/project-knowledge/references/`

- `AGENTS.md`: mirror the `CLAUDE.md` architecture + DAG edits.
- `.claude/skills/project-knowledge/references/architecture.md`: rewrite the
  module map for the five crates; update the data-model + dependency sections.
- `references/patterns.md`: update the git-workflow conflict-point note —
  the new conflict files are `crates/core/src/lib.rs`,
  `crates/mcp/src/tools.rs`, `crates/mcp/src/main.rs` (plus
  `crates/native/src/lib.rs`, `crates/wasm/src/lib.rs`).
- `references/deployment.md`: `Dockerfile`/`compose`/`Cross.toml`/`smithery.yaml`
  now under `deploy/`; the wasm build is one script targeting `crates/wasm`.

### 6.3 — `README.md` + `CONTRIBUTING.md`

- `README.md`: update the repo-layout tree and any `core/`/`mcp/`/`webapp/`
  build instructions to `crates/`/`packages/`.
- `CONTRIBUTING.md`: update "how to build" (single wasm script;
  `cargo build --workspace`), the crate map, and the per-crate scope guidance.

### 6.4 — Audit-enforced dependency rule (make the DAG mechanical)

Add a CI/audit check that fails on DAG violations, so the one-way graph can't
silently regress:

- Lightweight: a CI step running `cargo tree` greps per crate, e.g.
  `cargo tree -p mnemonic-core` must show **no** `mnemonic-native|mnemonic-wasm|
  mnemonic-mcp|mnemonic-prover` edge; `cargo tree -p mnemonic-wasm` and
  `-p mnemonic-prover` must show only `mnemonic-core` among workspace crates.
- This generalizes the existing "core has zero references to mcp" audit rule
  into the full DAG. Land it as a **non-flaky, gating** job (unlike the keychain
  job) — it's deterministic, so it belongs with `fmt`/`clippy`, not behind
  `continue-on-error`.

### 6.5 — Run the audit waves + QA gate

Per the spec-driven workflow (`work/<feature>/` waves): run the read-only
code/security/test audit waves against the refactored tree and append findings
to `work/refactoring/decisions.md`. Then the pre-deploy QA gate (the full "green
at every step" block + the new DAG audit) gates the final merge.

## Validation

```bash
cargo build --workspace
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk
npm run build --workspace=@mnemonik-xyz/cli

# New DAG audit (must pass):
cargo tree -p mnemonic-core   | grep -E 'mnemonic-(native|wasm|mcp|prover)' && echo VIOLATION || echo ok
cargo tree -p mnemonic-wasm   | grep -E 'mnemonic-(native|mcp|prover)'      && echo VIOLATION || echo ok
cargo tree -p mnemonic-prover | grep -E 'mnemonic-(native|wasm|mcp)'        && echo VIOLATION || echo ok

# Docs link check (the repo has docs-link-check.yml):
# ensure no doc still links to old core/src/<native> paths.
```

**cross-lang-build gate exercises:** unchanged from Phase 5; this phase edits
docs + adds the deterministic DAG audit job. Confirm the gate description in
`CLAUDE.md` now matches the actual `ci.yml` steps.

## Rollback

Docs/rules are reversible by `git revert`; the new DAG-audit CI job can be
removed if it proves noisy (but it shouldn't — it's deterministic). No source
risk.

## Definition of done / green check

- [ ] `CLAUDE.md`, `AGENTS.md`, README, CONTRIBUTING, and the
      `project-knowledge` references describe the five-crate / five-package
      layout + the one-way DAG.
- [ ] The audit-enforced dependency rule generalized from "core ≠> mcp" to the
      full DAG, with a deterministic gating CI job.
- [ ] The yo-yo-prevention `continue-on-error: true` on `cross-lang-keychain`
      is unchanged.
- [ ] Audit waves run; findings in `decisions.md`; QA gate green.
- [ ] Full "green at every step" block + DAG audit + `cross-lang-build` gate
      green.
