# reth isolated-environment sync stall — research

Investigation into why a reth node with a single reachable peer (rest of the
internet firewalled) intermittently stops syncing and never recovers.

- **`spec.md`** — root-cause analysis + fix proposal + test plan.
- **`fix.patch`** — the implemented fix + regression tests, as a diff against the
  pinned upstream commit `3d76b93`. Apply with:

  ```bash
  git submodule update --init reth
  git -C reth apply ../work/reth-isolated-sync-stall/fix.patch
  ```

  (The fix is also committed locally in the submodule on branch
  `claude/fix-isolated-peer-eviction`, commit `d0a9c0dbb`.)
- **`../../reth`** — reth is vendored as a git submodule, pinned to upstream
  commit `3d76b93` (the exact base the analysis was performed against). Run
  `git submodule update --init reth` from the repo root to check it out; then the
  `crates/net/...:line` references in `spec.md` resolve directly.

## What the fix does (`crates/net/network/src/peers.rs`)

1. **Reset on stable session** — in `on_connection_failure`, if the peer had held
   an active session for ≥ `STABLE_SESSION_MIN_UPTIME` (30s) before the drop, its
   `severe_backoff_counter` is reset to 0 instead of incremented. A node that keeps
   syncing between blinks never accumulates toward eviction.
2. **Never evict the last peer** — the backoff-driven removal now also requires
   `total_peers > 1`. A firewalled node's sole block source is kept (backed off but
   still dialable) rather than deleted, since discovery cannot re-add it.
3. **Observability** — backoff-driven removals now log at `warn!` (were silent
   `trace!`), so operators can see why they reached zero peers.

Regression tests added: `test_stable_session_resets_severe_backoff_counter`,
`test_last_peer_not_removed_on_max_backoff`; the existing
`test_remove_on_max_backoff_count` was updated to use two peers (so the removal
path still exercises under the new last-peer guard).

> **Not compiled in this environment:** the sandbox proxy blocks crates.io (HTTP
> 403), so reth's dependencies can't be fetched and `cargo check` can't run here.
> The change was verified by hand against the crate's types/APIs. Run
> `cargo nextest run -p reth-network` in a networked environment to validate.

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
