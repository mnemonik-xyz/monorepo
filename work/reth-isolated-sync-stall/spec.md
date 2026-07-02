# Spec: Single-neighbor node stops syncing after network "blinks"

- **Component:** `crates/net/network` (peer management)
- **Symptom:** A reth node running in an isolated environment, where it is allowed to
  reach exactly **one** neighbor and the rest of the internet is firewalled, stops
  syncing at random. Once it stops it never recovers on its own.
- **Status:** Root cause identified. Fix proposal below.

---

## 1. Reported behavior & hypothesis

The operator runs reth where security tooling blocks all outbound/inbound traffic
except to a single peer that serves blocks. Sync works, then intermittently halts.

Operator hypothesis:

> If the network "blinks" (a short connectivity loss), the peer that serves blocks
> is deleted from the peer list, and that is why the node stops syncing.

This hypothesis is **correct**. There is a code path that permanently evicts the
single serving peer after a handful of transient connection errors, and in an
isolated environment nothing ever re-adds it.

---

## 2. Root cause

### 2.1 The eviction path

The relevant code is `PeersManager::on_connection_failure`
(`crates/net/network/src/peers.rs:700`). It is invoked whenever an active session
is dropped (`on_active_session_dropped`, peers.rs:664) or an outgoing dial fails
(`on_outgoing_connection_failure`, peers.rs:675).

For a **non-fatal** error the peer is not touched immediately; instead a
per-peer counter is bumped and, once it crosses a threshold, the peer is deleted:

```rust
// crates/net/network/src/peers.rs:739
if let Some(kind) = err.should_backoff() {
    if peer.is_trusted() || peer.is_static() {
        // trusted/static: short, fixed backoff, counter never used
        backoff_until = Some(now + self.backoff_durations.low);
    } else {
        // Increment peer.backoff_counter
        if kind.is_severe() {                                    // Medium or High
            peer.severe_backoff_counter =
                peer.severe_backoff_counter.saturating_add(1);  // <-- climbs
        }
        backoff_until = Some(self.backoff_durations.backoff_until(kind, peer.severe_backoff_counter));
    }
}
...
// crates/net/network/src/peers.rs:776
if peer.severe_backoff_counter > self.max_backoff_count &&
    !peer.is_trusted() &&
    !peer.is_static()
{
    remove_peer = true;         // <-- permanent eviction of a Basic peer
}
...
if remove_peer {
    let (peer_id, _) = self.peers.remove_entry(peer_id).expect("peer must exist");
    self.queued_actions.push_back(PeerAction::PeerRemoved(peer_id));
}
```

`max_backoff_count` defaults to **5** (`crates/net/network-types/src/peers/config.rs:207`),
and the check is strictly-greater, so a **Basic** (non-trusted, non-static) peer is
removed on the **6th** severe backoff.

### 2.2 A network blink is classified as a *severe* backoff

What counts as "severe" is decided by `BackoffKind::is_severe`, which is true for
`Medium` and `High` (`crates/net/network-types/src/backoff.rs:24`).

The classification of a raw socket error lives in
`impl SessionError for io::Error` (`crates/net/network/src/error.rs:296`):

```rust
fn should_backoff(&self) -> Option<BackoffKind> {
    match self.kind() {
        ErrorKind::ConnectionReset | ErrorKind::BrokenPipe => Some(BackoffKind::Low),   // NOT severe
        ErrorKind::ConnectionRefused => Some(BackoffKind::High),                        // severe
        _ => Some(BackoffKind::Medium),                                                 // severe (catch-all)
    }
}
```

A "network blink" surfaces as exactly the errors that fall into the `_ => Medium`
catch-all (and `ConnectionRefused => High`):

- `ErrorKind::TimedOut` — read/write/connect timed out while the link was down
- `ErrorKind::HostUnreachable`, `ErrorKind::NetworkUnreachable`
- `ErrorKind::NetworkDown`
- `ErrorKind::ConnectionAborted`
- `ErrorKind::ConnectionRefused` — peer/port briefly unreachable → `High`

All of these are **severe**, so every blink increments `severe_backoff_counter`.
(Only a clean RST/`BrokenPipe` is treated as `Low` and does not count.)

### 2.3 The counter almost never resets

`severe_backoff_counter` is reset to `0` in exactly **one** place —
`on_active_session_gracefully_closed` (`crates/net/network/src/peers.rs:627`),
i.e. only when an already-established session is closed **gracefully**:

```rust
// crates/net/network/src/peers.rs:612
pub(crate) fn on_active_session_gracefully_closed(&mut self, peer_id: PeerId) {
    ...
    peer.severe_backoff_counter = 0;   // the ONLY reset
    ...
}
```

Crucially it is **not** reset when a connection *succeeds*. `mark_connected`
(`crates/net/network-types/src/peers/mod.rs:63`) and
`on_active_outgoing_established` (`peers.rs:650`) leave the counter untouched. A
session that is established and then dies from a blink goes through
`on_active_session_dropped` → `on_connection_failure` (the `Dropped` path), which is
**not** a graceful close, so it does not reset the counter either.

Net effect with a single neighbor: successful sync in between blinks does **not**
clear the counter. The counter is monotonic across blinks and reaches 6, at which
point the only peer is deleted.

### 2.4 Why the node never recovers (the isolation trigger)

In a normal deployment the eviction is self-healing: discovery (discv4/discv5) or a
DNS/bootnode re-discovers the peer within seconds and `add_peer`
(`peers.rs:821`) re-inserts it, so removal is just a long backoff.

In the operator's isolated environment discovery is dead — every candidate except
the one neighbor is firewalled. After the peer is removed:

- `self.peers` no longer contains it, so `fill_outbound_slots` has nothing to dial.
- Discovery yields nothing to re-add it.
- `PeerAction::PeerRemoved` tears down the session; the sync pipeline loses its only
  block source and stalls.

The node is now permanently peerless and sync is stuck until manual restart.

### 2.5 Summary of the chain

```
network blink
  → active session drops with io::Error (TimedOut / *Unreachable / ConnectionRefused / aborted)
  → should_backoff() = Medium/High  → is_severe() = true
  → severe_backoff_counter += 1        (never reset by successful sync)
  → after 6 blinks: counter (6) > max_backoff_count (5)
  → peer is Basic (not trusted/static) → removed from self.peers + PeerRemoved
  → isolated env: discovery cannot re-add it
  → no peers left → sync stalls indefinitely
```

The behavior is confirmed by the existing unit test
`test_remove_on_max_backoff_count` (`crates/net/network/src/peers.rs:1929`), which
seeds `severe_backoff_counter = max_backoff_count` and asserts the peer is dropped.

---

## 3. Immediate mitigation (no code change)

The removal branch is explicitly skipped for **trusted** and **static** peers
(`peers.rs:776-779`, and `apply_reputation_change`/backoff give them fixed short
backoffs instead of the climbing counter). So the operator can work around the bug
today by pinning the single neighbor:

- Add it to **`--trusted-peers`** (`enode://…`) — a trusted peer is never removed by
  the backoff counter and is exempted from reputation slashing; the resolver keeps
  retrying it (`on_outgoing_connection_failure` → `trusted_peers_resolver`, peers.rs:690).
- Optionally set **`--trusted-only`** so the node does not waste slots on unreachable
  discovered peers.
- Adding it as a **static** peer via `admin_addPeer` (JSON-RPC) has the same
  protective effect (`PeerKind::Static`).

This is the recommended operational fix regardless of the code change below.

---

## 4. Proposed code fix

The root defect is that a *transient, environmental* failure is accounted the same
way as a *persistently bad peer*, and the accounting is effectively irreversible for
a long-lived single peer. Two complementary changes:

### Fix A (primary): reset the severe backoff counter on a successful, stable session

A peer that connects and serves us blocks for a meaningful period is not a "bad
peer"; its earlier blinks should not accumulate toward eviction. Reset the counter
once a session has been *usefully* established, not only on graceful close.

- Reset `severe_backoff_counter = 0` when a session has stayed up for at least a
  minimum uptime. `Peer` already tracks `connected_at` and exposes
  `connected_for_at_least(now, min_uptime)`
  (`crates/net/network-types/src/peers/mod.rs:73`) for exactly this kind of check.
- Concretely: in `on_active_session_dropped` / the `Dropped` arm of
  `on_connection_failure`, if the peer was connected for ≥ `min_uptime` (e.g. 30s)
  before the drop, reset the counter instead of (or before) incrementing it. This
  keeps genuine flappers penalized while preventing a stable single peer from ever
  crossing the threshold via unrelated blinks spread over hours.

This directly falsifies the failure chain: a node that keeps successfully syncing
between blinks keeps its counter at/near 0 and is never evicted.

### Fix B (defense in depth): never evict the last usable peer / protect the sole block source

Even with Fix A, an environment that only ever has one reachable peer should not be
able to reach zero peers through the backoff path.

- In the removal branch (`peers.rs:776`), additionally guard against removing a peer
  when doing so would leave the peer set empty **and** discovery is effectively
  unable to replenish it — e.g. skip removal when `self.peers.len() == 1`
  (downgrade to a long backoff instead of deletion). A backed-off-but-present peer
  is still re-dialable by `fill_outbound_slots`; a removed one is not.
- This bounds the worst case: a single-neighbor node degrades to "retry with
  backoff" rather than "peerless forever".

### Fix C (optional, ergonomics): make the eviction observable and configurable

- Emit a `warn!`/metric when a peer is removed due to
  `severe_backoff_counter > max_backoff_count` (today it is only `trace!` at
  peers.rs:788), so operators can see *why* they went to zero peers.
- The threshold is already configurable via
  `PeersConfig::with_max_backoff_count` and the counting itself is sound for the
  normal (discovery-enabled) case, so no default change is required if Fix A + B land.

### Non-goals / rejected

- **Reclassifying blink errors as non-severe** (moving the `io::Error` catch-all
  from `Medium` to `Low`) is rejected: `Low` is also used to *rate-limit* reconnects
  and the catch-all legitimately covers genuinely unreachable peers. The problem is
  the *irreversible accumulation*, not the per-event severity.
- Removing the eviction entirely is rejected: in discovery-enabled deployments it is
  a useful mechanism to evict truly dead peers and free slots.

---

## 5. Test plan

- **Unit (regression for Fix A):** in `peers.rs` tests, drive a peer through
  `max_backoff_count + 1` severe backoffs, but insert a "connected for ≥ min_uptime
  then dropped" event partway through; assert the counter resets and the peer is
  **not** removed. Mirror the structure of `test_remove_on_max_backoff_count`
  (peers.rs:1929).
- **Unit (Fix B):** with a single peer at `severe_backoff_counter = max_backoff_count`,
  trigger one more severe backoff and assert the peer is retained (backed off) rather
  than removed and `PeerRemoved` is **not** queued.
- **Integration:** extend `crates/net/network/tests/it/connect.rs` (which already
  uses `with_max_backoff_count`, connect.rs:722) with an isolated-topology test: one
  configured peer, repeatedly force session drops with a `Medium` io error, verify
  the node re-dials and retains the peer, and that sync-relevant `PeerAction`s are
  not permanently `PeerRemoved`.

## 6. Affected files (for the fix)

- `crates/net/network/src/peers.rs` — reset logic in `on_connection_failure` /
  `on_active_session_dropped`; single-peer removal guard; upgrade log level.
- `crates/net/network-types/src/peers/mod.rs` — helper for "connected long enough to
  reset" (reuse `connected_for_at_least`).
- `crates/net/network-types/src/peers/config.rs` — optional `min_uptime`-style knob if
  the reset threshold should be configurable.

## 7. Key references

| What | Location |
| --- | --- |
| Eviction on backoff counter | `crates/net/network/src/peers.rs:776-790` |
| Counter increment (severe only) | `crates/net/network/src/peers.rs:752-755` |
| Only reset point (graceful close) | `crates/net/network/src/peers.rs:627` |
| Blink error → severe backoff | `crates/net/network/src/error.rs:296-308` |
| `is_severe` = Medium/High | `crates/net/network-types/src/backoff.rs:24` |
| Trusted/static exemption | `crates/net/network/src/peers.rs:551-568`, `776-779` |
| `max_backoff_count` default = 5 | `crates/net/network-types/src/peers/config.rs:207` |
| `connected_for_at_least` helper | `crates/net/network-types/src/peers/mod.rs:73` |
| Existing removal test | `crates/net/network/src/peers.rs:1929` |
