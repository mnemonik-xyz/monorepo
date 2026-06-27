import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fetchArtifacts,
  fetchAttestationTimeline,
  sampleArtifacts,
  sampleTimeline,
  type TimeRange,
} from "./ledger";

const mockFetch = () => fetch as unknown as ReturnType<typeof vi.fn>;

describe("sampleTimeline", () => {
  const cases: Array<[TimeRange, number]> = [
    ["30d", 30],
    ["90d", 90],
    ["12m", 12],
  ];

  it.each(cases)("has the right bucket count for %s", (range, count) => {
    expect(sampleTimeline(range).buckets).toHaveLength(count);
  });

  it.each(cases)(
    "reports totals matching the sum of buckets for %s",
    (range) => {
      const t = sampleTimeline(range);
      const node = t.buckets.reduce((s, b) => s + b.on_node, 0);
      const chain = t.buckets.reduce((s, b) => s + b.on_chain, 0);
      expect(t.total_on_node).toBe(node);
      expect(t.total_on_chain).toBe(chain);
    },
  );

  it("is flagged as sample data", () => {
    expect(sampleTimeline("30d").sample).toBe(true);
  });

  it("emits ascending ISO day-granularity dates", () => {
    const buckets = sampleTimeline("90d").buckets;
    for (const b of buckets) {
      expect(b.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    }
    const dates = buckets.map((b) => b.date);
    expect([...dates].sort()).toEqual(dates);
  });

  it("is deterministic across calls (no wall-clock dependence)", () => {
    expect(sampleTimeline("12m")).toEqual(sampleTimeline("12m"));
  });
});

describe("sampleArtifacts", () => {
  it("mixes both write modes", () => {
    const modes = new Set(sampleArtifacts().map((a) => a.write_mode));
    expect(modes).toContain("local");
    expect(modes).toContain("participate");
  });

  it("includes both real and local: / null anchors", () => {
    const rows = sampleArtifacts();
    const hasReal = rows.some(
      (a) => a.solana_tx !== null && !a.solana_tx.startsWith("local:"),
    );
    const hasUnanchored = rows.some(
      (a) => a.solana_tx === null || a.solana_tx.startsWith("local:"),
    );
    expect(hasReal).toBe(true);
    expect(hasUnanchored).toBe(true);
  });

  it("uses blake3-style 64-hex hashes and ISO timestamps with varied tags", () => {
    const rows = sampleArtifacts();
    for (const a of rows) {
      expect(a.content_hash).toMatch(/^[0-9a-f]{64}$/);
      expect(a.created_at).toMatch(
        /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/,
      );
      expect(a.tags.length).toBeGreaterThan(0);
    }
    const allTags = new Set(rows.flatMap((a) => a.tags));
    expect(allTags.size).toBeGreaterThan(1);
  });

  it("is deterministic across calls", () => {
    expect(sampleArtifacts()).toEqual(sampleArtifacts());
  });
});

describe("fetchArtifacts", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("maps the live backend shape (attestation_id -> id, local: tx preserved)", async () => {
    // The live /artifacts handler keys each row on `attestation_id` and emits
    // `local:`-prefixed tx ids for unanchored rows. fetchArtifacts must remap the
    // id so the page's <li key={a.id}> is usable, and leave the tx fields intact.
    const backendRow = {
      attestation_id: "5f3a0b9c-1111-4a00-9c01-aaaaaaaaaaaa",
      content: "Live recalled artifact",
      content_hash:
        "abc1230000000000000000000000000000000000000000000000000000000000",
      tags: ["live", "recall"],
      solana_tx: "local:offline-abcd",
      arweave_tx: "local:offline-abcd",
      created_at: "2026-06-26T12:00:00.000Z",
      write_mode: "local",
    };
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ artifacts: [backendRow], total: 1 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const page = await fetchArtifacts();
    expect(page.sample).toBe(false);
    expect(page.total).toBe(1);
    const [a] = page.artifacts;
    expect(a).toBeDefined();
    expect(a!.id).toBe(backendRow.attestation_id);
    expect(a!.content).toBe(backendRow.content);
    // local:-prefixed anchors pass through verbatim for the explorer-link helper.
    expect(a!.solana_tx).toBe("local:offline-abcd");
    expect(a!.arweave_tx).toBe("local:offline-abcd");
    // The transient `attestation_id` field is not carried onto the Artifact.
    expect(
      (a as unknown as Record<string, unknown>).attestation_id,
    ).toBeUndefined();
  });

  it("returns live rows with sample false on 200", async () => {
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ artifacts: [], total: 0 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const page = await fetchArtifacts();
    expect(page.sample).toBe(false);
    expect(page.artifacts).toEqual([]);
  });

  it("degrades to sample true on non-OK status", async () => {
    mockFetch().mockResolvedValueOnce(new Response("oops", { status: 503 }));
    const page = await fetchArtifacts();
    expect(page.sample).toBe(true);
    expect(page.artifacts).toEqual(sampleArtifacts());
  });

  it("degrades to sample true on network/timeout error", async () => {
    mockFetch().mockRejectedValueOnce(new TypeError("network down"));
    const page = await fetchArtifacts();
    expect(page.sample).toBe(true);
    expect(page.artifacts.length).toBeGreaterThan(0);
  });

  it("filters sample rows by query when degraded", async () => {
    mockFetch().mockRejectedValueOnce(new TypeError("offline"));
    const page = await fetchArtifacts({ q: "turboquant" });
    expect(page.sample).toBe(true);
    expect(page.artifacts.length).toBeGreaterThan(0);
    for (const a of page.artifacts) {
      const hay = (a.content + a.tags.join(" ")).toLowerCase();
      expect(hay).toContain("turboquant");
    }
  });
});

describe("fetchAttestationTimeline", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns the live series with sample false on 200", async () => {
    const live = {
      buckets: [{ date: "2026-06-01", on_node: 5, on_chain: 2 }],
      total_on_node: 5,
      total_on_chain: 2,
      unique_users: 3,
    };
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify(live), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const timeline = await fetchAttestationTimeline("30d");
    expect(timeline.sample).toBe(false);
    expect(timeline.buckets).toEqual(live.buckets);
    expect(timeline.total_on_node).toBe(5);
  });

  it("degrades to the sample series on non-OK status", async () => {
    mockFetch().mockResolvedValueOnce(new Response("oops", { status: 500 }));
    const timeline = await fetchAttestationTimeline("90d");
    expect(timeline.sample).toBe(true);
    expect(timeline).toEqual(sampleTimeline("90d"));
  });

  it("degrades to the sample series on network/timeout error", async () => {
    mockFetch().mockRejectedValueOnce(new TypeError("network down"));
    const timeline = await fetchAttestationTimeline("12m");
    expect(timeline.sample).toBe(true);
    expect(timeline.buckets).toHaveLength(12);
  });
});
