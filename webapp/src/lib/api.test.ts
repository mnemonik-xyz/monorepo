import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchPublicStats } from "./api";

describe("fetchPublicStats", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns_parsed_body_on_200", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          unique_users: 12,
          saved_on_node: 345,
          saved_onchain: 67,
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      ),
    );

    const stats = await fetchPublicStats();
    expect(stats).toEqual({
      unique_users: 12,
      saved_on_node: 345,
      saved_onchain: 67,
    });

    expect(fetch).toHaveBeenCalledWith("/stats", { method: "GET" });
  });

  it("returns_null_on_5xx", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("oops", { status: 503 }),
    );

    const stats = await fetchPublicStats();
    expect(stats).toBeNull();
  });

  it("returns_null_on_network_error", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new TypeError("network down"),
    );

    const stats = await fetchPublicStats();
    expect(stats).toBeNull();
  });
});
