---
phase: 1
title: Group, don't split
status: ready
risk: low
depends_on: [0]
---

# Phase 1 — Group, don't split

## Objective

Relocate every Rust member under `crates/` and every JS member under
`packages/` (webapp included), with the **smallest possible diff and zero code
changes inside the crates** — proving the harness still builds after a pure
relocation. No `core/` split here; that is Phases 2–3.

## Exact `git mv` list

### Rust members → `crates/`

```
git mv core  crates/core
git mv mcp   crates/mcp
```

### JS member → `packages/`

```
git mv webapp packages/webapp
```

(`packages/sdk`, `packages/cli`, `packages/extension`, `packages/mcp` are
already under `packages/` — no move.)

## Exact edits

### 1.1 — Root `Cargo.toml` workspace members

```toml
[workspace]
members = ["crates/core", "crates/mcp"]
resolver = "2"
```

### 1.2 — `crates/mcp/Cargo.toml` path dependency

The mcp crate references core by relative path; it stays `../core` because both
moved together under `crates/` (sibling dirs):

```toml
# crates/mcp/Cargo.toml — unchanged, still resolves:
mnemonic-core = { path = "../core" }
```

> Verify after the move: `../core` from `crates/mcp/` resolves to `crates/core/`.
> It does (both are siblings under `crates/`). No edit needed — but confirm.

### 1.3 — Root `package.json` workspaces

`webapp` moves under `packages/`, so the `webapp` glob entry is redundant with
`packages/*`. Remove it and update the wasm script paths:

```json
{
  "name": "mnemonic-monorepo",
  "private": true,
  "version": "0.0.0",
  "workspaces": [
    "packages/*"
  ],
  "scripts": {
    "build:wasm:webapp": "bash packages/webapp/scripts/build-wasm.sh",
    "build:wasm:sdk": "bash packages/sdk/scripts/build-wasm.sh"
  }
}
```

> The two `build-wasm.sh` scripts still compute `REPO_ROOT` from
> `$SCRIPT_DIR/../..` (sdk) and `$SCRIPT_DIR/../..` (webapp). After the move the
> sdk script (`packages/sdk/scripts/`) still needs `../../..` to reach repo root,
> and the webapp script (now `packages/webapp/scripts/`) needs `../../..` instead
> of its current `../..`. **Fix `REPO_ROOT` in `packages/webapp/scripts/build-wasm.sh`**
> from `"$SCRIPT_DIR/../.."` to `"$SCRIPT_DIR/../../.."`. The sdk script is
> already `../../..` and is correct as-is. (Both scripts still target `core/...`
> wasm paths in Phase 1; those paths become `crates/core/...` — see below.)

### 1.4 — Wasm-script internal paths (`core/` → `crates/core/`)

Both `build-wasm.sh` scripts run `wasm-pack build core ...` and read/write
`core/pkg*`. After Phase 1 the crate lives at `crates/core`, so update the
`core` references inside both scripts:

- `packages/sdk/scripts/build-wasm.sh`: `wasm-pack build core` →
  `wasm-pack build crates/core`; every `core/pkg`, `core/pkg-web`,
  `core/pkg-nodejs` path → `crates/core/pkg*`.
- `packages/webapp/scripts/build-wasm.sh`: same `core` → `crates/core`
  substitutions; `webapp/src/wasm` → `packages/webapp/src/wasm`.

> These are interim edits; Phase 2 replaces both scripts with a single one
> targeting `crates/wasm`. Keeping Phase 1 a pure relocation means we still fix
> the paths so the wasm build stays green between phases.

### 1.5 — `.github/workflows/ci.yml`

- `webapp` job: `working-directory: webapp` → `working-directory: packages/webapp`.
- `cross-lang-build` + `cross-lang-keychain`:
  `cargo build -p mnemonic-core --example keychain-roundtrip` is **unchanged**
  (it addresses the crate by `-p mnemonic-core`, not by path — the move is
  invisible to cargo). `npm run build --workspace=@mnemonik-xyz/sdk|cli` is
  also path-agnostic (workspace name).
- `paths-ignore` filters unaffected (no `core/`/`mcp/` path filters exist today;
  the workflow runs on all non-doc paths).
- Audit `node-test.yml`, `release.yml`, `deploy-webapp.yml`, `ext-e2e.yml` for
  literal `webapp/`, `core/`, `mcp/` path references and update to
  `packages/webapp/`, `crates/core/`, `crates/mcp/`. Cargo `-p <name>` and
  `--workspace=<npmname>` references need **no** change.

### 1.6 — `Dockerfile`

```dockerfile
# before:
COPY core/ core/
COPY mcp/ mcp/
# after:
COPY crates/ crates/
```

`cargo build --release -p mnemonic-mcp --features local-embed` is unchanged
(`-p` name, not path). `COPY Cargo.toml Cargo.lock* ./` unchanged.

> Note for Phase 5: the Dockerfile itself stays at repo root in Phase 1 so
> `docker-compose.yml`'s `context: .` + `dockerfile: Dockerfile` keep working.
> Moving it to `deploy/` is Phase 5.

### 1.7 — `docker-compose.yml`

```yaml
# nginx volume:
- ./packages/webapp/dist:/usr/share/nginx/html:ro   # was ./webapp/dist
```

`build context: .`, `dockerfile: Dockerfile`, `./nginx.conf`, `./ollama`,
`./keypair` unchanged in Phase 1.

## Validation

```bash
# Rust workspace still builds + lints + tests from the new layout:
cargo build --workspace
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support

# JS workspace resolves with webapp under packages/*:
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk      # exercises packages/sdk/scripts/build-wasm.sh → crates/core
npm run build --workspace=@mnemonik-xyz/cli
npm run build --workspace=mnemonic-webapp        # exercises packages/webapp/scripts/build-wasm.sh

# Container path sanity (optional, if Docker available locally):
docker build -f Dockerfile -t mnemonic-mcp:phase1 .
```

**cross-lang-build gate exercises:** `cargo build -p mnemonic-mcp` and
`cargo build -p mnemonic-core --example keychain-roundtrip` (both pass
unchanged — path move is invisible to cargo `-p`), then the sdk + cli
`npm run build` (pass because §1.4 fixed the wasm-script `core` → `crates/core`
paths). Regenerate `Cargo.lock` (paths in lock are name-keyed, minimal churn)
and `package-lock.json` (workspace globs changed).

## Rollback

Single-commit revert: `git mv` back (`crates/core` → `core`, `crates/mcp` →
`mcp`, `packages/webapp` → `webapp`) and restore the four edited config files
(`Cargo.toml`, `package.json`, `ci.yml`, `Dockerfile`, `docker-compose.yml`,
both `build-wasm.sh`). No source code changed, so revert is mechanical.

## Definition of done / green check

- [ ] `crates/core`, `crates/mcp`, `packages/webapp` exist; old paths gone.
- [ ] Root `Cargo.toml` members = `["crates/core", "crates/mcp"]`.
- [ ] Root `package.json` workspaces = `["packages/*"]`; wasm script paths fixed.
- [ ] Both `build-wasm.sh` scripts target `crates/core` and produce the same
      `pkg-web` artifact bytes as before (the published `mnemonic-core` npm
      contract is unchanged in Phase 1).
- [ ] Full "green at every step" block passes, including webapp build.
- [ ] `cross-lang-build` gate green (verified by running its four steps locally).
- [ ] `Cargo.lock` + `package-lock.json` regenerated and committed.
