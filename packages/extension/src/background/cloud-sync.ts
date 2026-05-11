// Alarm-driven drain of the cloud-sync queue. Invoked from the MV3
// service worker on every `cloud-sync-retry` alarm tick (5-min cadence
// per T10) and on the explicit `ui:flush-pending` gesture from the
// popup.
//
// **Contract:** read `pending_uploads` rows from `IndexedDbStore` →
// for each, run `CloudClient.signRemote(...)` → on success write the
// returned `solana_tx` + `arweave_tx` onto the local row + `dequeue`
// → on `PermanentSyncError` mark the row `sync_failed_permanent` and
// dequeue (drop out of the alarm-retry loop while keeping the local
// COSE envelope so the user can re-issue manually) → on
// `ReauthRequiredError` stop the drain and tell the SW to emit
// `mnemonik:re-auth-required` → on `TransientSyncError` leave the row
// in the queue for the next tick.
//
// **IDB discipline:** `IndexedDbStore` opens a fresh transaction per
// call. We do NOT share an `IDBDatabase` cursor across awaits — each
// `saveAttestation` / `dequeue` is its own short transaction. This
// mirrors the !Send rusqlite discipline from the server.

import type { IndexedDbStore } from "../runtime/store/indexeddb.js";
import type { AttestationRow } from "../runtime/store/types.js";
import {
  PermanentSyncError,
  ReauthRequiredError,
  TransientSyncError,
} from "../runtime/sync/cloud-client.js";
import type { SignRemoteResult } from "../runtime/sync/types.js";

/** CustomEvent name dispatched on `globalThis` when a 401 surfaces.
 *  Popup + options page subscribe to this and route the user back to
 *  the T16 Google sign-in flow. Tests assert against this constant so
 *  the wire spelling stays stable across refactors. */
export const REAUTH_EVENT_NAME = "mnemonik:re-auth-required";

/** Optional listener for sync failures the SW surfaces to the popup. */
export type SyncEventListener = (event: SyncEvent) => void;

export type SyncEvent =
  | { type: "re-auth-required"; attestation_id: string }
  | {
      type: "permanent-failure";
      attestation_id: string;
      status: number;
      message: string;
    };

/**
 * Minimal client surface the drain depends on. Lets tests inject a
 * fake without standing up a `fetch` mock. `CloudClient` satisfies
 * this structurally.
 */
export interface CloudSignClient {
  signRemote(
    args: { content: string; tags: string[] },
    signer: { cose_bytes: Uint8Array; signer_pubkey: string },
  ): Promise<SignRemoteResult>;
}

export interface DrainDeps {
  /** Backing store. The drain pulls rows out, never holds a cursor
   *  across awaits — IDB `!Send` discipline (see file header). */
  store: IndexedDbStore;
  /** Cloud client built per-drain — keeps the JWT freshness window
   *  tight. `null` short-circuits the drain (e.g. no session yet);
   *  equivalent to "queue stays untouched". */
  cloudClient: CloudSignClient | null;
  /** Optional event sink. The SW wires this to
   *  `chrome.runtime.sendMessage` / `globalThis.dispatchEvent`.
   *  Defaults to a no-op so the popup can call `drainPendingUploads`
   *  directly when it wants explicit control. */
  emit?: SyncEventListener;
}

export interface DrainResult {
  /** Rows the drain attempted to upload. */
  attempted: number;
  /** Rows that uploaded successfully (queue row removed, local row
   *  updated with solana_tx + arweave_tx). */
  flushed: number;
  /** Distinct categorised errors observed this tick. */
  errors: Array<{
    attestation_id: string;
    kind: "transient" | "permanent" | "reauth" | "missing-row";
    message: string;
  }>;
}

/** SW-facing return shape. Matches the deps signature installed in
 *  T10 so the service-worker alarm handler is unchanged. */
export interface FlushPendingResult {
  attempted: number;
  flushed: number;
}

/**
 * SW-facing deps for `flushPending`. The SW alarm handler composes
 * this per tick from its own deps (storeFactory + cloudClientProvider
 * + emitSyncEvent), and the `ui:flush-pending` handler reuses the
 * same shape so explicit-drain gestures produce the same observable
 * behaviour as the automatic tick.
 */
export interface FlushPendingDeps {
  store: IndexedDbStore;
  /** Pre-built `CloudClient` (or any `CloudSignClient`). `null` when
   *  there is no current session — the drain reports the queue depth
   *  and returns without touching anything. */
  cloudClient: CloudSignClient | null;
  /** Optional event sink. Production wiring forwards to
   *  `chrome.runtime.sendMessage` so the popup can react to 401 /
   *  permanent-failure events. */
  emit?: SyncEventListener;
}

// ────────────────────────────────────────────────────────────────────────
// SW-facing entrypoint
// ────────────────────────────────────────────────────────────────────────

/**
 * SW-facing drain entrypoint. Thin adapter over
 * `drainPendingUploads`: forwards every dep, plus auto-dispatches
 * the `mnemonik:re-auth-required` CustomEvent on `globalThis` when
 * the drain observes a 401 (in addition to forwarding the typed
 * event through `deps.emit`). Returns the projected `{attempted,
 * flushed}` snapshot the SW logs.
 *
 * Never throws — every error is surfaced via the returned counts
 * + `deps.emit`. The SW alarm `.catch` is purely defensive.
 */
export async function flushPending(
  deps: FlushPendingDeps,
): Promise<FlushPendingResult> {
  const result = await drainPendingUploads({
    store: deps.store,
    cloudClient: deps.cloudClient,
    emit: (event) => {
      if (event.type === "re-auth-required") dispatchReauthEvent();
      deps.emit?.(event);
    },
  });
  return { attempted: result.attempted, flushed: result.flushed };
}

/**
 * Drain the `pending_uploads` queue. Idempotent: rerunning after a
 * partial failure resumes where the previous tick left off.
 *
 * Stop conditions:
 *   - Empty queue → returns `{attempted: 0, flushed: 0, errors: []}`.
 *   - `cloudClient` is `null` (no session) → reports the queue depth
 *     so the SW can log "waiting for sign-in" without crashing.
 *   - 401 on any row → drain stops; remaining rows stay queued.
 *
 * Never throws — every error is captured as an entry in `errors[]`.
 */
export async function drainPendingUploads(
  deps: DrainDeps,
): Promise<DrainResult> {
  const result: DrainResult = { attempted: 0, flushed: 0, errors: [] };
  const pending = await deps.store.listPending();
  if (pending.length === 0) return result;
  if (deps.cloudClient === null) {
    result.attempted = pending.length;
    return result;
  }

  for (const row of pending) {
    result.attempted += 1;
    // Fresh per-row look-up so a tx aborted by a previous iteration
    // does not poison this one. IDB discipline: short transactions
    // only; never hold one across the `await` for the network call.
    const attestation = await deps.store.findById(row.attestation_id);
    if (!attestation) {
      // The queue references an attestation that no longer exists in
      // the store (e.g. user deleted it). Drop the queue entry so we
      // don't spin on it forever.
      await deps.store.dequeue(row.attestation_id);
      result.errors.push({
        attestation_id: row.attestation_id,
        kind: "missing-row",
        message: "attestation row vanished; dequeued",
      });
      continue;
    }
    try {
      const upload = await deps.cloudClient.signRemote(
        {
          content: attestation.content,
          tags: [...attestation.tags],
          ...(attestation.source_meta
            ? { source_meta: attestation.source_meta }
            : {}),
        },
        {
          cose_bytes: attestation.cose_bytes,
          signer_pubkey: attestation.signer_pubkey,
        },
      );
      // Persist the cloud-side tx ids onto the existing row. Keep
      // `attestation_id` stable — the server may return its own UUID
      // but the local store has been keying off the client-derived id
      // since T11; rewriting it would break Recall + Verify rows that
      // already reference it from `lineage_edges`. The tx ids are
      // exactly what D2 asks the drain to write back.
      const updated: AttestationRow = {
        ...attestation,
        solana_tx: upload.solana_tx,
        arweave_tx: upload.arweave_tx,
      };
      // Clear any prior `sync_failed_permanent` flag — a successful
      // retry supersedes a previous permanent-failure stamp.
      if (updated.sync_failed_permanent) {
        delete updated.sync_failed_permanent;
      }
      await deps.store.saveAttestation(updated);
      await deps.store.dequeue(row.attestation_id);
      result.flushed += 1;
    } catch (err) {
      if (err instanceof ReauthRequiredError) {
        deps.emit?.({
          type: "re-auth-required",
          attestation_id: row.attestation_id,
        });
        result.errors.push({
          attestation_id: row.attestation_id,
          kind: "reauth",
          message: err.message,
        });
        // Halt — every subsequent row would hit the same 401.
        return result;
      }
      if (err instanceof PermanentSyncError) {
        // Mark the row permanently failed so the popup can render the
        // "sync failed — recreate this memory" hint; also dequeue so
        // the alarm doesn't churn on it.
        await markPermanentFailure(deps.store, attestation);
        await deps.store.dequeue(row.attestation_id);
        deps.emit?.({
          type: "permanent-failure",
          attestation_id: row.attestation_id,
          status: err.status,
          message: err.message,
        });
        result.errors.push({
          attestation_id: row.attestation_id,
          kind: "permanent",
          message: err.message,
        });
        continue;
      }
      if (err instanceof TransientSyncError) {
        result.errors.push({
          attestation_id: row.attestation_id,
          kind: "transient",
          message: err.message,
        });
        // Leave the row queued; the next alarm tick retries.
        continue;
      }
      // Unknown error type — treat as transient so we don't lose
      // data, but tag explicitly in the log. Caught here rather than
      // re-thrown so a single bad row doesn't abort the whole drain.
      const message = err instanceof Error ? err.message : String(err);
      result.errors.push({
        attestation_id: row.attestation_id,
        kind: "transient",
        message: `unexpected error: ${message}`,
      });
    }
  }
  return result;
}

// ────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────

/** Stamp the row `sync_failed_permanent = true`. Idempotent. */
async function markPermanentFailure(
  store: IndexedDbStore,
  row: AttestationRow,
): Promise<void> {
  if (row.sync_failed_permanent === true) return;
  const updated: AttestationRow = { ...row, sync_failed_permanent: true };
  await store.saveAttestation(updated);
}

/**
 * Dispatch the `mnemonik:re-auth-required` CustomEvent on `globalThis`.
 * Best-effort: silently no-ops if the host environment lacks
 * `dispatchEvent` / `CustomEvent` (e.g. an isolated SW context). The
 * popup + options page register listeners; the SW alarm handler
 * fans this out to any open extension page.
 */
function dispatchReauthEvent(): void {
  const g = globalThis as {
    dispatchEvent?: (e: Event) => boolean;
    CustomEvent?: typeof CustomEvent;
  };
  if (typeof g.dispatchEvent !== "function") return;
  const Ctor = g.CustomEvent;
  if (typeof Ctor !== "function") return;
  try {
    g.dispatchEvent(new Ctor(REAUTH_EVENT_NAME));
  } catch {
    // Hostile / read-only globals — non-fatal.
  }
}
