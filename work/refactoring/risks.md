---
title: Refactoring risks + mitigations
status: ready
---

# Refactoring risks + mitigations

The phases are mechanical, but the repo has three live tripwires: a hard CI
gate, a documented yo-yo rule, and a small set of public-ish contracts. This
file is the checklist to keep them green.

## 1. `cross-lang-build (gate)` is a hard required gate

`.github/workflows/ci.yml` → `cross-lang-build` is **never** `continue-on-error`
and blocks PRs. It exercises exactly:

```
cargo build -p mnemonic-mcp
cargo build -p mnemonic-core --example keychain-roundtrip   # → mnemonic-native in Phase 3
npm run build --workspace=@mnemonik-xyz/sdk                 # runs build-wasm.sh
npm run build --workspace=@mnemonik-xyz/cli
```

- **Phases 1, 2, 3 move exactly these paths/targets.** Mitigation: the workflow
  edit lands **in the same commit** as the move — never "fix CI in a follow-up".
- Phase 2 is the single riskiest: the SDK build shells to `build-wasm.sh`, which
  Phase 2 rewrites to target `crates/wasm`. Run all four gate steps **locally**
  before opening the Phase 2 PR.
- Phase 3 changes `--example keychain-roundtrip` from `-p mnemonic-core` to
  `-p mnemonic-native` (the example moves with the keychain code). Forgetting
  this single line turns the gate red.
- `wasm-pack` missing is the historical regression this gate exists to catch
  (PR #151). Do not assume a green local run implies a green runner — the gate
  installs `wasm-pack 0.14.0` via the prebuilt installer; keep that step intact.

## 2. `cross-lang-keychain (informational)` — the yo-yo rule

`cross-lang-keychain` is **permanently** `continue-on-error: true` until the
gnome-keyring daemon-coupling sub-test B is fixed upstream (CLAUDE.md
"## CI gate policy"; commit `fde7f72`). Phase 3 moves the keychain code into
`crates/native` and Phase 5 moves `tests/cross-lang/keychain.sh` →
`conformance/cross-lang/keychain.sh`.

- **Do NOT flip `continue-on-error` on either job** during any phase. The
  keychain code moving crates is exactly the change the rule warns against
  coupling to a toggle flip — flipping it previously masked the wasm-pack
  regression that shipped to main.
- Phase 3/5 may only edit the *paths* inside the job
  (`-p mnemonic-native --example keychain-roundtrip`,
  `bash conformance/cross-lang/keychain.sh`) — never its gate semantics.
- If you believe the test is now stable, follow the documented procedure
  (10 green runs → single PR removing the flag AND updating CLAUDE.md → reviewer
  sign-off). That is **out of scope** for this refactor.

## 3. Conflict-point files — schedule, don't parallelize

The files most likely to collide with concurrent feature work:

- `crates/core/src/lib.rs` — edited in Phase 2 (remove `pub mod wasm;`) and
  Phase 3 (remove the entire `cfg(not(wasm32))` native block).
- `crates/mcp/src/tools.rs` — edited in Phase 3 (re-point native imports;
  this file alone has arweave + embed + solana + storage uses).
- `crates/mcp/src/main.rs` — edited in Phase 3 (native imports + the
  `COMPRESS_SEED` cross-reference comment).
- Plus the new `crates/native/src/lib.rs` and `crates/wasm/src/lib.rs`.

**Order constraint:** run the wasm (Phase 2) and native (Phase 3) extraction
phases on a **clean branch off `main`, not in parallel with feature work**. Each
phase is one PR; merge it, rebase any in-flight feature branches onto the new
layout, then proceed. Parallel feature PRs touching these files will produce
ugly, error-prone merges precisely where a mistake silently breaks the wasm or
native build.

## 4. The `mnemonic-core` npm name + `core/pkg-web` public contract

- `packages/webapp/package.json` depends on `"mnemonic-core": "0.2.4"` (the
  published npm wasm package), and the SDK + golden tests consume the artifact
  at `core/pkg-web/` with fixed filenames `mnemonic_core.js`,
  `mnemonic_core_bg.wasm`, `mnemonic_core.d.ts`, `mnemonic_core_bg.wasm.d.ts`.
- These are public-ish contracts for SDK/webapp consumers. **Phase 2 must
  preserve the emitted package/module name and artifact filenames**, or bump the
  published version deliberately in a coordinated, separately-versioned change.
- Default chosen in Phase 2: keep the emitted `mnemonic_core_*` filenames stable
  even though the crate dir is `crates/wasm` and the member is `mnemonic-wasm`.
  Only the on-disk artifact *directory* moves (`core/pkg-web` →
  `crates/wasm/pkg-web`); JS module imports of the API are unchanged. Re-point
  every literal `core/pkg*` path in `packages/sdk/src/wasm.ts`,
  `packages/sdk/test/`, and `packages/webapp/src/`.

## 5. Cargo.lock / package-lock churn

- `Cargo.lock`: workspace path keys change as crates move/split; the churn is
  large but mechanical. Regenerate per phase (`cargo build --workspace`
  rewrites it) and commit it with the phase.
- `package-lock.json`: changes in Phase 1 (root `workspaces` glob drops
  `webapp`) and Phase 2 (build-script consolidation). Regenerate via
  `npm install --workspaces --include-workspace-root` and commit per phase.
- Do not hand-edit either lockfile; let the tools regenerate so the lock matches
  the manifests the CI gate resolves against.

## 6. Order constraint summary

1. Phase 0 (audit) before anything — it produces the move-list the riskier
   phases rely on.
2. Phases 1 → 2 → 3 are strictly ordered: group first, then extract wasm
   (so `core` can drop `cdylib`), then extract native (so `core` can drop the
   `cfg` block and prove portability via
   `cargo build -p mnemonic-core --target wasm32-unknown-unknown`).
3. Phase 4 (prover) depends on Phase 3's `core` being the stable portable
   kernel.
4. Phase 5 (non-code) and Phase 6 (docs/rules) come last; Phase 6's DAG audit
   only makes sense once all crates exist.
5. Each phase: one branch, one PR, merge, rebase in-flight work, then next.

## 7. Secondary tripwires (audit each phase)

- `.github/workflows/{release.yml, node-test.yml, deploy-mcp.yml,
  deploy-webapp.yml, ext-e2e.yml, nightly.yml, docs-link-check.yml}` — beyond
  `ci.yml`, these reference `core/`, `webapp/`, `tests/`, `smithery.yaml`,
  `Cross.toml`, `Dockerfile`, or `core/pkg`. Audit and update per phase
  (inventory in phase-0 §0.5).
- `cargo audit` (`.cargo/audit.toml`) and `gitleaks` (`.gitleaks.toml`) configs
  reference paths only loosely; confirm no allowlist path breaks after moves.
- `smithery.yaml` schema gate validates against `scripts/smithery-schema.yaml`;
  Phase 5 moves `smithery.yaml` to `deploy/` — update the `yamale` invocation.
- `keychain-roundtrip` / `keychain-read` examples (Phase 3) and `emit_golden` /
  `golden-keystore-gen` examples must end up in the crate that owns the modules
  they call, or their `[[example]]` entries fail to compile.
