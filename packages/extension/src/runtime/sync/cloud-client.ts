// Cloud-tier sync client — placeholder owned by T18 (cloud-tier sync via
// deferred signing). The service worker (T10) imports `flushPending` so
// the alarm-driven drain has a stable call site before T18 lands.
//
// T18 will:
//   1. Iterate `IndexedDbStore.listPending()` in FIFO order.
//   2. Load the corresponding `AttestationRow` from `attestations`.
//   3. POST the COSE_Sign1 envelope to `/mcp` `mnemonic_sign_memory`
//      (deferred-signing flow) and then `/api/sign-callback`.
//   4. On success: update the row with the returned solana_tx / arweave_tx
//      and `dequeue(attestation_id)`.
//   5. On transient failure: leave the row enqueued; the next alarm tick
//      retries with exponential back-off bounded at 1h.
//
// Until then, this stub is a no-op so the SW build stays green and the
// alarm-drain unit test has a spy target.

import type { IndexedDbStore } from "../store/indexeddb.js";

export interface FlushPendingDeps {
  /** Backing store, supplied so tests can inject a fake. */
  store: IndexedDbStore;
}

export interface FlushPendingResult {
  /** Rows the call attempted to drain. */
  attempted: number;
  /** Rows successfully uploaded and dequeued. */
  flushed: number;
}

/**
 * Drain the cloud-sync queue. **Stub** — T18 owns the real implementation.
 *
 * The shape (`Promise<FlushPendingResult>` + injectable deps) is the
 * contract T18 is expected to honour so the SW caller stays unchanged.
 */
export async function flushPending(
  deps: FlushPendingDeps
): Promise<FlushPendingResult> {
  const pending = await deps.store.listPending();
  // T18: actually POST to MCP server and dequeue on success. For now we
  // just report the queue depth so the alarm logs something useful.
  return { attempted: pending.length, flushed: 0 };
}
