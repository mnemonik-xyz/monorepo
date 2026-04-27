# Decisions Log — mnemonic-integrations

Append-only log of decisions and audit findings during execution.

---

## Task 3 — Webapp WASM build pipeline (T3-impl)

**Date:** 2026-04-26
**Status:** Implementation complete; smoke verify pending Task 2 (`wasm` feature on `core/Cargo.toml` + `[lib] crate-type = ["cdylib", "rlib"]`).

### What changed

- **New:** `webapp/scripts/build-wasm.sh` — `set -euo pipefail`, anchors to repo root, checks `wasm-pack` is on PATH, runs `wasm-pack build core --target web --out-dir webapp/src/wasm --release --features wasm`. Marked executable (`chmod 755`). Top-of-file comment documents the `cargo install wasm-pack` prerequisite, the Task 2 dependency on the `wasm` feature, and the expected output layout.
- **Modified:** `webapp/package.json` — added `"build:wasm": "bash scripts/build-wasm.sh"`, rewrote `build` to `npm run build:wasm && tsc -b && vite build`, and added a `"//"` field documenting the `cargo install wasm-pack` prerequisite (per task spec hint #5). Other scripts (`dev`, `preview`, `test:e2e`) untouched. No new runtime dependencies.
- **New:** `webapp/.gitignore` — excludes `node_modules/`, `dist/`, `src/wasm/` (the `wasm-pack` output, regenerated on every build), plus `playwright-report/`, `test-results/`, and standard editor/OS noise.
- **Unchanged:** `webapp/vite.config.ts` — left alone per task spec ("only modify if smoke testing surfaces a concrete error"). Vite ≥ 6 handles the `wasm-pack --target web` ESM shim natively; no `vite-plugin-wasm`, no `optimizeDeps.target = 'esnext'` needed.

### Verification

- `bash -n webapp/scripts/build-wasm.sh` — syntax OK
- `node -e "JSON.parse(...)"` on `webapp/package.json` — JSON valid
- `git check-ignore -v webapp/src/wasm/dummy.wasm` → matched on `webapp/.gitignore:8:src/wasm/` — gitignore rule wired correctly
- `cd webapp && npm run build:wasm` — npm script invokes `bash scripts/build-wasm.sh` (confirmed). The script reaches `wasm-pack build core ... --features wasm` and fails with `Error: crate-type must be cdylib to compile to wasm32-unknown-unknown` — **this is the expected pre-Task-2 state**: `core/Cargo.toml` does not yet declare the `wasm` feature nor `[lib] crate-type = ["cdylib", "rlib"]`. Once Task 2 lands those, the smoke command will succeed end-to-end.
- Local toolchain: `wasm-pack 0.13.1` is on PATH.

### Deviations

None. The script uses the repo-root-anchored form from the task spec body (Details section) rather than the path-relative form mentioned in the dispatcher prompt — the repo-root form is more robust (works regardless of caller's CWD) and matches the spec's authoritative implementation hint. Behavior is identical.

### Concerns / follow-ups

- **Smoke verification deferred:** the full `cd webapp && npm run build` cannot exit 0 until Task 2 has merged. The pipeline files (script + package.json + .gitignore) are independent and complete; they only need Task 2's Rust-side `wasm` feature + `cdylib` crate-type to produce artifacts.
- **CI reminder:** any future CI step that invokes `npm run build` on the webapp must also `cargo install wasm-pack` first (or use a pre-built image). This is documented in the script's prereq comment and the `package.json` `//` field; deployment.md prerequisites should reference it when CI integration lands (Task 14 territory).
- **Submodule note:** the dispatcher prompt referred to `webapp/` as a git submodule, but `git ls-tree HEAD webapp` reports `040000 tree` (regular tracked directory), not `160000 commit` (submodule pointer). Although `webapp/.git` exists locally, the parent monorepo has no `.gitmodules` entry and stores webapp files directly. Therefore the changes are committed as ordinary file additions in the monorepo, not via the two-step "submodule commit + pointer update" flow.
