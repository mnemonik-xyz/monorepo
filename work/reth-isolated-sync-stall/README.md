# reth isolated-environment sync stall — research

Investigation into why a reth node with a single reachable peer (rest of the
internet firewalled) intermittently stops syncing and never recovers.

- **`spec.md`** — root-cause analysis + fix proposal + test plan.
- **`../../reth`** — reth is vendored as a git submodule, pinned to upstream
  commit `3d76b93` (the exact base the analysis was performed against). Run
  `git submodule update --init reth` from the repo root to check it out; then the
  `crates/net/...:line` references in `spec.md` resolve directly.

## TL;DR

A network "blink" drops the session to the sole peer with a transient
`io::Error` that is classified as a *severe* backoff. The per-peer
`severe_backoff_counter` only resets on a graceful close, so it climbs
monotonically across blinks; after `max_backoff_count` (default 5) the `Basic`
peer is permanently removed. With discovery firewalled, nothing re-adds it, so
the node goes to zero peers and sync stalls. See `spec.md` for the annotated
code path and the proposed fix.

> Note: the spec was also committed inside the reth submodule locally at
> `reth/docs/specs/isolated-peer-removal-sync-stall.md`, but that commit is not
> pushed anywhere (no write access to `paradigmxyz/reth`), so this monorepo copy
> is the canonical, version-controlled deliverable.
