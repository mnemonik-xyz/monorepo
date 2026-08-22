---
phase: 5
title: Consolidate non-code
status: ready
risk: low
depends_on: [4]
---

# Phase 5 — Consolidate non-code

## Objective

Give the cross-language conformance vectors one home (`conformance/`), move all
deploy/infra artifacts out of the root into `deploy/`, and keep `docs/` as the
single docs home. No Rust/JS source logic changes — only relocation + the
config/workflow edits that follow the moved paths.

## Exact `git mv` list

### Conformance (cross-language golden vectors → single source)

Today these are scattered: `tests/cross-lang/keychain.sh`,
`tests/fixtures/{generate-legacy.mjs,legacy-identity.json}`, plus per-package
golden fixtures (e.g. `packages/extension/tests/fixtures/golden/`,
`packages/sdk/test*/` golden fixtures, the Rust golden emitters).

```
git mv tests/cross-lang   conformance/cross-lang
git mv tests/fixtures      conformance/fixtures
```

> Per-package golden fixtures (`packages/extension/tests/fixtures/golden/`,
> `packages/sdk/tests/`) are **consumed locally by each package's test runner**.
> The high-level plan wants "one source of truth"; the safe, low-risk move is:
> keep the package-local fixture *copies* where the test runners expect them,
> but make `conformance/` the **generator + canonical vectors** that those
> copies are derived from. Aggressively relocating package-local fixtures risks
> breaking each package's vitest config — defer that to a follow-up unless
> Phase 0 confirmed the test configs read from a shared path. Record the chosen
> scope in `decisions.md`.

### Deploy / infra artifacts → `deploy/`

```
git mv Dockerfile          deploy/Dockerfile
git mv docker-compose.yml  deploy/docker-compose.yml
git mv nginx.conf          deploy/nginx.conf
git mv smithery.yaml       deploy/smithery.yaml
git mv Cross.toml          deploy/Cross.toml
git mv ollama              deploy/ollama
```

### Docs

`docs/` already exists and is the docs home — no move. In Phase 5, optionally
relocate research history (`work/research/computation-proof/{positioning,
v1-agentic-payments,decisions}.md`) under `docs/research/` if the owner wants
research findings discoverable; otherwise leave under `work/`.

## Exact edits

### 5.1 — `deploy/docker-compose.yml` — relative paths after the move

The compose file moves into `deploy/`, so its relative paths change. Either (a)
keep `context` pointed at the repo root, or (b) run compose from `deploy/`.
Recommended: keep build context at the repo root by going up one level.

```yaml
services:
  nginx:
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf:ro          # now sits beside compose in deploy/
      - ../packages/webapp/dist:/usr/share/nginx/html:ro
      # ...
  mcp:
    build:
      context: ..                  # repo root (Cargo.toml + crates/ live there)
      dockerfile: deploy/Dockerfile
    volumes:
      - ../keypair:/run/secrets/keypair:ro
  ollama:
    build:
      context: ./ollama            # deploy/ollama after the move
      dockerfile: Dockerfile
```

> Decide and document (decisions.md): compose is now run as
> `docker compose -f deploy/docker-compose.yml ...` from the repo root, OR
> `cd deploy && docker compose up`. The `context: ..` form above assumes the
> former (run from root). Pick one and make the paths consistent.

### 5.2 — `deploy/Dockerfile` — build context unchanged content, path-of-file changed

`Dockerfile` content is unchanged (it already does `COPY crates/ crates/` from
Phase 1, `COPY Cargo.toml Cargo.lock* ./`, `cargo build -p mnemonic-mcp`). Only
its **location** moves to `deploy/Dockerfile`; every reference to it
(`docker-compose.yml`, `deploy-mcp.yml`) updates to `deploy/Dockerfile`.

### 5.3 — `.github/workflows/ci.yml`

- `smithery-schema` job:
  `yamale -s scripts/smithery-schema.yaml smithery.yaml` →
  `yamale -s scripts/smithery-schema.yaml deploy/smithery.yaml`
  (the schema file `scripts/smithery-schema.yaml` stays in `scripts/` unless
  also moved; only the validated `smithery.yaml` moved).
- `test-stdio` job + any cross-lang job referencing `tests/cross-lang/keychain.sh`
  → `conformance/cross-lang/keychain.sh`. (The `cross-lang-keychain` job's final
  step runs `bash tests/cross-lang/keychain.sh` — update to
  `bash conformance/cross-lang/keychain.sh` in the **same commit**; this string
  lives inside the informational job but the path must still resolve.)
- `paths-ignore`: add `deploy/**` is NOT desired (infra changes should still run
  CI); but `conformance/**` changes *should* run CI (they gate cross-lang). Do
  not add either to `paths-ignore`.

### 5.4 — Other workflows

- `deploy-mcp.yml`: any `Dockerfile` / `docker-compose.yml` / `smithery.yaml`
  reference → `deploy/...`.
- `deploy-webapp.yml`: `webapp/dist` already became `packages/webapp/dist` in
  Phase 1; confirm no `nginx.conf` / compose root references remain.
- `release.yml`: `Cross.toml` reference (cross-compilation) →
  `deploy/Cross.toml`; confirm `cross` is invoked with the new config path
  (`CROSS_CONFIG=deploy/Cross.toml` or `--config`).
- `nightly.yml`: `smithery.yaml` / fixtures references → new paths.

### 5.5 — Root cleanup

After moves, the repo root holds: `Cargo.toml`, `package.json`, lockfiles,
`crates/`, `packages/`, `conformance/`, `deploy/`, `docs/`, `work/`, top-level
markdown (`README.md`, `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING.md`,
`CODE_OF_CONDUCT.md`, `SECURITY.md`, `LICENSE`), `.github/`, `rust-toolchain.toml`,
`scripts/`, `.gitleaks.toml`, `.cargo/`. Notably **out of root**: `Dockerfile`,
`docker-compose.yml`, `nginx.conf`, `smithery.yaml`, `Cross.toml`, `ollama/`,
`tests/`.

## Validation

```bash
# Rust/JS unaffected (no source moved):
cargo build --workspace
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk

# Infra paths resolve from new locations:
docker build -f deploy/Dockerfile -t mnemonic-mcp:phase5 .          # context = repo root
docker compose -f deploy/docker-compose.yml config                  # paths validate
yamale -s scripts/smithery-schema.yaml deploy/smithery.yaml         # schema gate
bash conformance/cross-lang/keychain.sh                             # if keyring available locally
```

**cross-lang-build gate exercises:** unchanged cargo + npm steps — Phase 5 does
not touch the build graph. The `cross-lang-keychain` informational job's
`bash conformance/cross-lang/keychain.sh` path must resolve (same-commit edit).

## Rollback

Mechanical `git mv` reversal of `deploy/*` → root and `conformance/*` → `tests/`,
plus reverting the workflow + compose path edits. Single-commit revert.

## Definition of done / green check

- [ ] `conformance/` holds the cross-lang vectors + fixtures; canonical
      generator location decided + recorded.
- [ ] `deploy/` holds `Dockerfile`, `docker-compose.yml`, `nginx.conf`,
      `smithery.yaml`, `Cross.toml`, `ollama/`; root is free of them.
- [ ] `docker build -f deploy/Dockerfile .` and
      `docker compose -f deploy/docker-compose.yml config` succeed.
- [ ] All workflows referencing moved infra/test paths updated in the same PR.
- [ ] Full "green at every step" block + `cross-lang-build` gate green.
