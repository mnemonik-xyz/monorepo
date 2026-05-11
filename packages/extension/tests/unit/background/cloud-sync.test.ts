// T18 — cloud-sync drain unit tests.
//
// D13 anchor: `alarm_drains_queue_on_reconnect`. Seeds 3 rows into
// the `pending_uploads` queue + the backing `attestations` store,
// drives `drainPendingUploads` with a fake `CloudSignClient` that
// returns success for every row, and asserts:
//   - the queue is empty after the drain,
//   - every row was updated with the returned `solana_tx` +
//     `arweave_tx`,
//   - the drain reports `attempted = 3, flushed = 3`.
//
// Extra coverage:
//   - `null` cloud client short-circuits (queue stays).
//   - `ReauthRequiredError` halts the drain mid-batch + emits
//     `re-auth-required` for the row that hit 401.
//   - `PermanentSyncError` stamps `sync_failed_permanent` + dequeues.
//   - `TransientSyncError` leaves the row queued and continues.

import "fake-indexeddb/auto";
import { describe, it, expect, vi } from "vitest";

import {
  drainPendingUploads,
  flushPending,
  REAUTH_EVENT_NAME,
  type CloudSignClient,
  type SyncEvent,
} from "../../../src/background/cloud-sync.js";
import {
  PermanentSyncError,
  ReauthRequiredError,
  TransientSyncError,
} from "../../../src/runtime/sync/cloud-client.js";
import { IndexedDbStore } from "../../../src/runtime/store/indexeddb.js";
import type { AttestationRow } from "../../../src/runtime/store/types.js";

let dbCounter = 0;
function freshDbName(label: string): string {
  dbCounter += 1;
  return `mnemonik-cs-${label}-${dbCounter}-${Math.random().toString(36).slice(2)}`;
}

function makeRow(
  id: string,
  overrides: Partial<AttestationRow> = {},
): AttestationRow {
  return {
    attestation_id: id,
    content: `content-${id}`,
    content_hash: "0".repeat(64),
    tags: ["source:chatgpt"],
    embedding: new Float32Array([1, 0, 0, 0]),
    cose_bytes: new Uint8Array([0x84, 0x01]),
    created_at: "2026-05-11T00:00:00Z",
    signer_pubkey: "SignerA",
    owner_pubkey: "OwnerA",
    solana_tx: `local:${id}`,
    arweave_tx: `local:${id}`,
    ...overrides,
  };
}

/** Build a fake CloudSignClient that records calls and dispenses a
 *  scripted result (or thrown error) per call. */
function makeClient(
  results: Array<
    | { ok: true; solana_tx: string; arweave_tx: string }
    | { ok: false; err: Error }
  >,
): CloudSignClient & {
  calls: Array<{ content: string; signer_pubkey: string }>;
} {
  const calls: Array<{ content: string; signer_pubkey: string }> = [];
  const client: CloudSignClient = {
    async signRemote(args, signer) {
      calls.push({
        content: args.content,
        signer_pubkey: signer.signer_pubkey,
      });
      const next = results.shift();
      if (!next)
        throw new Error("test: no scripted result for signRemote call");
      if (!next.ok) throw next.err;
      return {
        attestation_id: `srv-${calls.length}`,
        solana_tx: next.solana_tx,
        arweave_tx: next.arweave_tx,
      };
    },
  };
  return Object.assign(client, { calls });
}

async function seedQueue(store: IndexedDbStore, ids: string[]): Promise<void> {
  for (const id of ids) {
    await store.saveAttestation(makeRow(id));
    await store.enqueue(id);
  }
}

describe("drainPendingUploads · empty queue", () => {
  it("returns zeros without touching the cloud client", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("empty") });
    const client = makeClient([]);
    const result = await drainPendingUploads({ store, cloudClient: client });
    expect(result).toEqual({ attempted: 0, flushed: 0, errors: [] });
    expect(client.calls).toHaveLength(0);
  });
});

describe("drainPendingUploads · null client (no session)", () => {
  it("reports queue depth and leaves the queue intact", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("null") });
    await seedQueue(store, ["a", "b"]);
    const result = await drainPendingUploads({ store, cloudClient: null });
    expect(result).toEqual({ attempted: 2, flushed: 0, errors: [] });
    const pending = await store.listPending();
    expect(pending.map((p) => p.attestation_id).sort()).toEqual(["a", "b"]);
  });
});

describe("drainPendingUploads · alarm_drains_queue_on_reconnect (D13 anchor)", () => {
  it("drains 3 queued rows and writes solana_tx + arweave_tx onto each row", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("drain3") });
    await seedQueue(store, ["att-1", "att-2", "att-3"]);

    const client = makeClient([
      { ok: true, solana_tx: "Sol-1", arweave_tx: "Ar-1" },
      { ok: true, solana_tx: "Sol-2", arweave_tx: "Ar-2" },
      { ok: true, solana_tx: "Sol-3", arweave_tx: "Ar-3" },
    ]);

    const result = await drainPendingUploads({ store, cloudClient: client });

    expect(result).toMatchObject({ attempted: 3, flushed: 3 });
    expect(result.errors).toEqual([]);

    // Queue empty.
    const pending = await store.listPending();
    expect(pending).toEqual([]);

    // Every row updated.
    const a1 = await store.findById("att-1");
    const a2 = await store.findById("att-2");
    const a3 = await store.findById("att-3");
    expect(a1?.solana_tx).toBe("Sol-1");
    expect(a1?.arweave_tx).toBe("Ar-1");
    expect(a2?.solana_tx).toBe("Sol-2");
    expect(a2?.arweave_tx).toBe("Ar-2");
    expect(a3?.solana_tx).toBe("Sol-3");
    expect(a3?.arweave_tx).toBe("Ar-3");

    // Drain visited each row exactly once in FIFO order.
    expect(client.calls.map((c) => c.content)).toEqual([
      "content-att-1",
      "content-att-2",
      "content-att-3",
    ]);
  });
});

describe("drainPendingUploads · 401 halts the drain and emits re-auth-required", () => {
  it("stops mid-batch and emits the event for the failing row only", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("reauth") });
    await seedQueue(store, ["att-1", "att-2", "att-3"]);
    const events: SyncEvent[] = [];

    const client = makeClient([
      { ok: true, solana_tx: "Sol-1", arweave_tx: "Ar-1" },
      { ok: false, err: new ReauthRequiredError("expired") },
      // The 3rd row is never reached because the drain halts.
    ]);

    const result = await drainPendingUploads({
      store,
      cloudClient: client,
      emit: (event) => events.push(event),
    });

    expect(result.attempted).toBe(2);
    expect(result.flushed).toBe(1);
    expect(result.errors).toHaveLength(1);
    expect(result.errors[0]).toMatchObject({
      attestation_id: "att-2",
      kind: "reauth",
    });

    // The 2nd + 3rd rows stay queued.
    const pending = await store.listPending();
    expect(pending.map((p) => p.attestation_id).sort()).toEqual([
      "att-2",
      "att-3",
    ]);

    // Only one event emitted, for the row that observed 401.
    expect(events).toEqual([
      { type: "re-auth-required", attestation_id: "att-2" },
    ]);
  });
});

describe("drainPendingUploads · permanent failure", () => {
  it("stamps sync_failed_permanent on the row + dequeues it", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("perm") });
    await seedQueue(store, ["att-1"]);
    const events: SyncEvent[] = [];

    const client = makeClient([
      { ok: false, err: new PermanentSyncError("expired correlation_id", 410) },
    ]);

    const result = await drainPendingUploads({
      store,
      cloudClient: client,
      emit: (event) => events.push(event),
    });

    expect(result.attempted).toBe(1);
    expect(result.flushed).toBe(0);
    expect(result.errors[0]?.kind).toBe("permanent");

    // Queue empty (permanent failure dequeues so we don't churn).
    const pending = await store.listPending();
    expect(pending).toEqual([]);

    // Row stamped.
    const row = await store.findById("att-1");
    expect(row?.sync_failed_permanent).toBe(true);

    // Emit fired with HTTP status forwarded.
    expect(events).toEqual([
      {
        type: "permanent-failure",
        attestation_id: "att-1",
        status: 410,
        message: "expired correlation_id",
      },
    ]);
  });
});

describe("drainPendingUploads · transient failure", () => {
  it("leaves the row queued and continues with the next one", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("transient") });
    await seedQueue(store, ["att-1", "att-2"]);

    const client = makeClient([
      { ok: false, err: new TransientSyncError("503 upstream", 503) },
      { ok: true, solana_tx: "Sol-2", arweave_tx: "Ar-2" },
    ]);

    const result = await drainPendingUploads({ store, cloudClient: client });

    expect(result.attempted).toBe(2);
    expect(result.flushed).toBe(1);
    expect(result.errors[0]?.kind).toBe("transient");

    // att-1 stays queued; att-2 was flushed.
    const pending = await store.listPending();
    expect(pending.map((p) => p.attestation_id)).toEqual(["att-1"]);
    expect((await store.findById("att-2"))?.solana_tx).toBe("Sol-2");
  });
});

describe("drainPendingUploads · missing row", () => {
  it("dequeues queue entries whose attestation_id was deleted from the store", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("missing") });
    // Enqueue without ever saving the row.
    await store.enqueue("ghost-id");
    const client = makeClient([]);

    const result = await drainPendingUploads({ store, cloudClient: client });

    expect(result.attempted).toBe(1);
    expect(result.flushed).toBe(0);
    expect(result.errors[0]).toMatchObject({
      attestation_id: "ghost-id",
      kind: "missing-row",
    });
    expect(await store.listPending()).toEqual([]);
    expect(client.calls).toHaveLength(0);
  });
});

// Regression for test-reviewer F3: source_meta MUST round-trip
// through the drain into signRemote args. The drain spreads
// row.source_meta conditionally; a regression that drops the spread
// would silently keep the cloud row's source attribution empty.
describe("drainPendingUploads · forwards source_meta", () => {
  it("passes through source_meta when the local row carries one", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("source-meta") });
    const rowWithMeta: AttestationRow = makeRow("att-meta", {
      source_meta: {
        platform: "chatgpt",
        model: "gpt-4o",
        chat_id: "ch-abc",
        url: "https://chatgpt.com/c/ch-abc",
      },
    });
    await store.saveAttestation(rowWithMeta);
    await store.enqueue(rowWithMeta.attestation_id);

    // Capture the full args object on the client side so we can
    // assert source_meta arrived intact.
    const captured: Array<Record<string, unknown>> = [];
    const client: CloudSignClient = {
      async signRemote(args) {
        // Spread the whole `args` so we don't drop optional fields:
        // the assertion below reads `source_meta` off the record.
        captured.push({ ...(args as unknown as Record<string, unknown>) });
        return {
          attestation_id: "srv-1",
          solana_tx: "Sol-meta",
          arweave_tx: "Ar-meta",
        };
      },
    };

    const result = await drainPendingUploads({ store, cloudClient: client });
    expect(result.flushed).toBe(1);
    expect(captured).toHaveLength(1);
    expect(captured[0]?.["source_meta"]).toEqual({
      platform: "chatgpt",
      model: "gpt-4o",
      chat_id: "ch-abc",
      url: "https://chatgpt.com/c/ch-abc",
    });
  });

  it("omits source_meta when the row has none (does not ship undefined)", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("no-meta") });
    // Default makeRow has no source_meta.
    await seedQueue(store, ["att-x"]);
    const captured: Array<Record<string, unknown>> = [];
    const client: CloudSignClient = {
      async signRemote(args) {
        captured.push({ ...args });
        return { attestation_id: "srv", solana_tx: "S", arweave_tx: "A" };
      },
    };
    await drainPendingUploads({ store, cloudClient: client });
    expect(captured).toHaveLength(1);
    // Must not include the key at all (D9 hygiene: no implicit
    // `undefined` fields traveling over JSON).
    expect(
      Object.prototype.hasOwnProperty.call(captured[0], "source_meta"),
    ).toBe(false);
  });
});

// Regression for test-reviewer F1: `flushPending` (the SW-facing
// wrapper) dispatches a `mnemonik:re-auth-required` CustomEvent on
// `globalThis` when the drain observes a 401. Previously only the
// `emit` callback was covered; the CustomEvent dispatch path had
// zero direct coverage.
describe("flushPending · re-auth CustomEvent dispatch", () => {
  it("fires mnemonik:re-auth-required on globalThis when the drain sees 401", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("flush-reauth") });
    await seedQueue(store, ["att-1"]);

    const client: CloudSignClient = {
      async signRemote() {
        throw new ReauthRequiredError("expired");
      },
    };

    // The unit-test environment is `node`, which doesn't expose
    // EventTarget on globalThis. Install a `dispatchEvent` mock
    // (the brief allows this) plus a `CustomEvent` stub so the SW's
    // `new g.CustomEvent(NAME)` constructor call resolves. Captures
    // the dispatched event type without touching jsdom.
    const g = globalThis as {
      dispatchEvent?: unknown;
      CustomEvent?: unknown;
    };
    const prevDispatch = g.dispatchEvent;
    const prevCustom = g.CustomEvent;
    const dispatchSpy = vi.fn().mockReturnValue(true);
    g.dispatchEvent = dispatchSpy;
    class CustomEventStub {
      public readonly type: string;
      public constructor(type: string) {
        this.type = type;
      }
    }
    g.CustomEvent = CustomEventStub as unknown as typeof CustomEvent;
    try {
      const result = await flushPending({ store, cloudClient: client });
      expect(result.attempted).toBe(1);
      expect(result.flushed).toBe(0);
    } finally {
      g.dispatchEvent = prevDispatch;
      g.CustomEvent = prevCustom;
    }
    expect(dispatchSpy).toHaveBeenCalledTimes(1);
    const [event] = dispatchSpy.mock.calls[0] as [{ type: string }];
    expect(event.type).toBe(REAUTH_EVENT_NAME);
  });

  it("also forwards re-auth via the deps.emit callback (both signals fire)", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("flush-both") });
    await seedQueue(store, ["att-1"]);

    const client: CloudSignClient = {
      async signRemote() {
        throw new ReauthRequiredError("expired");
      },
    };
    const emitted: SyncEvent[] = [];
    const result = await flushPending({
      store,
      cloudClient: client,
      emit: (e) => emitted.push(e),
    });
    expect(result.attempted).toBe(1);
    expect(emitted).toEqual([
      { type: "re-auth-required", attestation_id: "att-1" },
    ]);
  });
});
