import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MCP_BASE, fetchPublicStats, hostedOperatorBase } from "./api";

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

    expect(fetch).toHaveBeenCalledWith(`${MCP_BASE}/stats`, { method: "GET" });
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

describe("hostedOperatorBase", () => {
  it("accepts mnemonik.xyz operators and loopback dev servers", () => {
    expect(hostedOperatorBase("https://mcp2.mnemonik.xyz")).toBe("https://mcp2.mnemonik.xyz");
    expect(hostedOperatorBase("https://mcp.mnemonik.xyz/")).toBe("https://mcp.mnemonik.xyz");
    expect(hostedOperatorBase("http://localhost:3000")).toBe("http://localhost:3000");
  });

  it("rejects other hosts and malformed values", () => {
    for (const raw of [
      "",
      "https://evil.example",
      "https://mnemonik.xyz.evil.example",
      "https://evilmnemonik.xyz",
      "http://mcp.mnemonik.xyz",
      "https://user@mcp.mnemonik.xyz",
      "https://mcp.mnemonik.xyz/api",
      "not a url",
    ]) {
      expect(hostedOperatorBase(raw)).toBeNull();
    }
  });
});
