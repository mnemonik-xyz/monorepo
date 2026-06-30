---
phase: 4
title: Land crates/prover
status: ready
risk: low
depends_on: [3]
---

# Phase 4 — Land `crates/prover`

## Objective

Give the zigz correspondence prover (currently a research spike under
`work/research/computation-proof/`) a first-class home as a workspace member
`crates/prover` (`mnemonic-prover`), depending only on portable `crates/core`.

## Source of the work

The prover does **not** exist as a compiled crate today — it lives as research +
spikes:

```
work/research/computation-proof/tech-spec.md
work/research/computation-proof/decisions.md
work/research/computation-proof/positioning.md
work/research/computation-proof/v1-agentic-payments.md
work/research/computation-proof/spikes/zigz-stateful-intent/payment_mandate.zig
work/research/computation-proof/spikes/zigz-stateful-intent/payment_mandate_guest.zig
work/research/computation-proof/spikes/zigz-stateful-intent/README.md
```

> **Decision gate (open question #3 from the high-level plan; record in
> decisions.md):** does the prover live *in this monorepo* as `crates/prover`,
> or in a separate `mnemonik-dev/zigz` repo that this monorepo depends on? This
> phase assumes the in-repo answer. If the owner chooses the external-repo path,
> Phase 4 collapses to: add a path/git dependency in the crate(s) that consume
> proofs and skip the `git mv` below.

## Exact `git mv` list (in-repo option)

There is no existing Rust crate to move — the spikes are `.zig` + specs. Phase 4
**creates** `crates/prover` and relocates the research artifacts that become the
crate's reference docs:

```
# Keep the research narrative in docs (Phase 5 will consolidate docs), but the
# spec the crate implements moves next to the crate:
git mv work/research/computation-proof/tech-spec.md  crates/prover/SPEC.md
git mv work/research/computation-proof/spikes        crates/prover/spikes
```

Leave `positioning.md`, `v1-agentic-payments.md`, `decisions.md` under
`work/research/computation-proof/` (research history, not crate source) — or
move them to `docs/research/` in Phase 5.

> If the prover is implemented fresh in Rust (zigz is a Zig project — the spikes
> are `.zig`), the `spikes/` stay as reference and `crates/prover/src/` is new
> Rust authored to the SPEC. No `git mv` of source then; just `crates/prover/`
> scaffolding + `SPEC.md`.

## Exact edits

### 4.1 — New `crates/prover/Cargo.toml`

```toml
[package]
name = "mnemonic-prover"
version = "0.1.0"
edition = "2021"
description = "Mnemonic Protocol correspondence prover (zigz) — verifiable computation over attestations"

[lib]
crate-type = ["rlib"]

[dependencies]
mnemonic-core = { path = "../core" }
serde = { version = "1", features = ["derive"] }
anyhow = "1"
# (add prover-specific deps as the implementation lands — keep the dep set
# minimal and portable; the prover must not pull native integrations.)

[features]
default = []
# Experimental until the computation-proof feature declares GA in its
# decisions.md — mirrors the trajectory-experimental pattern in core.
experimental = []
```

### 4.2 — `crates/prover/src/lib.rs`

Scaffold with a documented module that implements (or wraps) the correspondence
proof per `crates/prover/SPEC.md`. Depends only on `mnemonic_core::{codec,
merkle, identity}` — **never** on `mnemonic-native` or `mnemonic-wasm` (keeps
the DAG one-way: prover → core only).

### 4.3 — Root `Cargo.toml` — register the member, behind a non-default gate if needed

```toml
[workspace]
members = ["crates/core", "crates/native", "crates/wasm", "crates/mcp", "crates/prover"]
resolver = "2"
```

> If the prover starts experimental and shouldn't gate default builds, it is
> still a workspace member (so it builds in CI) but its `experimental` feature
> is off by default. `cargo build --workspace` compiles its lib skeleton; the
> heavy proof machinery sits behind `--features mnemonic-prover/experimental`.

### 4.4 — `crates/mcp` (optional consumer)

If `mcp` exposes a proof-verification tool, add
`mnemonic-prover = { path = "../prover" }` to `crates/mcp/Cargo.toml` and a
forwarding feature `prover-experimental = ["mnemonic-prover/experimental"]`.
Otherwise leave `mcp` untouched — the prover can land as a standalone member
first and be wired into `mcp` in a later feature PR.

### 4.5 — `.github/workflows/ci.yml`

`cargo build --workspace` / `cargo test --workspace` automatically pick up the
new member — no per-job edit needed unless the prover requires extra system deps
(e.g. a Zig toolchain to build `spikes/`). If so, add a dedicated, **non-gating**
informational job (mirror the `cross-lang-keychain` informational pattern; do
NOT make zig-spike builds a hard gate while the feature is experimental).

## Validation

```bash
cargo build --workspace                         # prover lib compiles, skeleton ok
cargo build -p mnemonic-prover --features experimental
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support

# DAG check — prover must not depend on native/wasm:
cargo tree -p mnemonic-prover | grep -E 'mnemonic-(native|wasm)' && echo "DAG VIOLATION" || echo "DAG ok"

# JS workspace + wasm artifact unaffected:
npm install --workspaces --include-workspace-root --no-audit --no-fund
npm run build --workspace=@mnemonik-xyz/sdk
```

**cross-lang-build gate exercises:** unchanged from Phase 3 — the prover is not
on the cross-lang path. The gate stays green as long as `cargo build -p
mnemonic-mcp` still links (it does; prover is optional/standalone).

## Rollback

Remove `crates/prover` from workspace members, delete the crate dir (restoring
the `git mv`'d spikes/spec back under `work/research/computation-proof/`), and
drop any `mnemonic-prover` dep from `mcp`. Single-commit revert.

## Definition of done / green check

- [ ] `crates/prover` is a workspace member; `cargo build --workspace` compiles
      it.
- [ ] `cargo tree -p mnemonic-prover` shows **no** edge to `mnemonic-native` or
      `mnemonic-wasm` (DAG one-way: prover → core only).
- [ ] In-repo-vs-external decision recorded in `decisions.md`.
- [ ] Experimental machinery gated behind a non-default feature; default
      workspace build stays fast.
- [ ] Full "green at every step" block + `cross-lang-build` gate green.
