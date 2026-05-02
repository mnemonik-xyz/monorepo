# Contributing to Mnemonic Protocol

Thanks for your interest. This document covers the practical bits. Repo conventions, architecture rules, and the spec-driven workflow are documented in [`CLAUDE.md`](./CLAUDE.md) — please skim it before opening a non-trivial PR.

## Branching

- Default branch is **`dev`**. All PRs target `dev`.
- Feature branches: **`feat/<short-name>`**. Bug fix branches: **`fix/<short-name>`**.
- `main` is reserved for tagged releases.

## Setup

Rust workspace plus npm packages.

```bash
# Rust workspace (core + mcp)
cargo build --workspace

# Webapp
cd webapp && npm install

# SDK and CLI
cd packages/sdk && npm install
cd packages/cli && npm install
```

The MCP server requires an embedder. The simplest local path:

```bash
STORAGE_MODE=local PAYMENT_MODE=none \
  cargo run -p mnemonic-mcp --release --features local-embed -- --transport http --port 3000
```

## Tests

```bash
cargo test --workspace --no-fail-fast       # full Rust suite
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

npm test                                     # in each npm package you touched
```

CI enforces fmt, clippy with `-D warnings`, and the full test suite. Run them locally before pushing.

## Commit format

Conventional Commits with a component scope, per `CLAUDE.md`:

```
feat(core): add lineage chain verification
fix(mcp):   reject expired JWT before tool dispatch
docs:       clarify storage mode semantics
chore:      bump rusqlite to 0.31
```

Allowed scopes include `core`, `mcp`, `cli`, `sdk`, `webapp`, plus type-only entries (`docs`, `chore`, `style`, `ci`, `test`, `refactor`).

## Filing bugs

Open a GitHub issue with: environment (OS, toolchain, version), exact reproduction steps, expected vs. actual behavior, and relevant logs. Minimal repros get fixed faster.

For security vulnerabilities, **do not open a public issue** — follow [`SECURITY.md`](./SECURITY.md).

## Proposing changes

For anything beyond a small fix, open a **GitHub Discussion** first to align on direction. Large changes without prior discussion are likely to be sent back for redesign.

## Spec-driven workflow

Non-trivial features live under `work/<feature>/` with:

- `user-spec.md` — what and why
- `tech-spec.md` — how (architecture, decisions, testing, tasks)
- `tasks/<n>.md` — atomic units
- `decisions.md` — append-only log

If you are contributing a feature, follow the same structure. It keeps reviews focused and the history navigable.

## License and DCO

This project is licensed under **Apache-2.0**. By submitting a contribution you agree it is licensed under the same terms (inbound = outbound). **No CLA and no DCO sign-off are required.**

## Code of Conduct

All participation is governed by [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md). Report concerns to **dev@mnemonik.xyz**.
