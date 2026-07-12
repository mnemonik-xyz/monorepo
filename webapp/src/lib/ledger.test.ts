import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchArtifacts, fetchAttestationTimeline } from "./ledger";

const mockFetch = () => fetch as unknown as ReturnType<typeof vi.fn>;

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

  it("returns live rows on 200", async () => {
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ artifacts: [], total: 0 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const page = await fetchArtifacts();
    expect(page.artifacts).toEqual([]);
    expect(page.total).toBe(0);
  });

  it("passes the requested bounded page size to the backend", async () => {
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ artifacts: [], total: 0 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await fetchArtifacts({ limit: 200 });

    expect(mockFetch()).toHaveBeenCalledWith(
      expect.stringMatching(/\/artifacts\?limit=200$/),
      expect.any(Object),
    );
  });

  it("passes a source-aware page to the backend", async () => {
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ artifacts: [], total: 0 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await fetchArtifacts({ limit: 24, source: "on_chain" });

    expect(mockFetch()).toHaveBeenCalledWith(
      expect.stringMatching(/\/artifacts\?limit=24&source=on_chain$/),
      expect.any(Object),
    );
  });

  it("throws on non-OK status (no sample fallback)", async () => {
    mockFetch().mockResolvedValueOnce(new Response("oops", { status: 503 }));
    await expect(fetchArtifacts()).rejects.toThrow();
  });

  it("throws on network/timeout error (no sample fallback)", async () => {
    mockFetch().mockRejectedValueOnce(new TypeError("network down"));
    await expect(fetchArtifacts()).rejects.toThrow();
  });

  it("throws on a malformed body (missing artifacts array)", async () => {
    mockFetch().mockResolvedValueOnce(
      new Response(JSON.stringify({ total: 0 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    await expect(fetchArtifacts()).rejects.toThrow();
  });
});

describe("fetchAttestationTimeline", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns the live series on 200", async () => {
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
    expect(timeline.buckets).toEqual(live.buckets);
    expect(timeline.total_on_node).toBe(5);
  });

  it("throws on non-OK status (no sample fallback)", async () => {
    mockFetch().mockResolvedValueOnce(new Response("oops", { status: 500 }));
    await expect(fetchAttestationTimeline("90d")).rejects.toThrow();
  });

  it("throws on network/timeout error (no sample fallback)", async () => {
    mockFetch().mockRejectedValueOnce(new TypeError("network down"));
    await expect(fetchAttestationTimeline("12m")).rejects.toThrow();
  });
});
