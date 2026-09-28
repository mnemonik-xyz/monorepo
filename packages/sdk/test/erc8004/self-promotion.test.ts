// Self-promotion guard tests — mocked eth_call via vi.stubGlobal(fetch).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkSelfPromotion,
  computeSelector,
  SEL_GET_APPROVED,
  SEL_IS_APPROVED_FOR_ALL,
  SEL_OWNER_OF,
} from "../../src/erc8004/self-promotion.js";

const CLIENT = "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B";
const OTHER  = "0x1234567890123456789012345678901234567890";
// EIP-55 checksum of other — simplified for tests; we only compare lowercase
const RPC    = "https://rpc.example.com";

// ── Selector sanity ────────────────────────────────────────────────────────

describe("ERC-721 selectors", () => {
  it("ownerOf(uint256) selector is pinned", () => {
    expect(computeSelector("ownerOf(uint256)")).toBe(SEL_OWNER_OF);
    expect(SEL_OWNER_OF).toBe("0x6352211e");
  });

  it("isApprovedForAll(address,address) selector is pinned", () => {
    expect(computeSelector("isApprovedForAll(address,address)")).toBe(
      SEL_IS_APPROVED_FOR_ALL
    );
    expect(SEL_IS_APPROVED_FOR_ALL).toBe("0xe985e9c5");
  });

  it("getApproved(uint256) selector is pinned", () => {
    expect(computeSelector("getApproved(uint256)")).toBe(SEL_GET_APPROVED);
    expect(SEL_GET_APPROVED).toBe("0x081812fc");
  });
});

// ── Offline checks ─────────────────────────────────────────────────────────

describe("checkSelfPromotion offline", () => {
  it("rejects zero address", async () => {
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: "0x0000000000000000000000000000000000000000",
    });
    expect(result.status).toBe("self");
  });

  it("rejects bad checksum address", async () => {
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: "0xab5801a7d398351b8be11c439e05c5b3259aec9b", // lowercase = wrong checksum
    });
    expect(result.status).toBe("self");
  });

  it("returns skipped when no rpcUrl", async () => {
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
    });
    expect(result.status).toBe("skipped");
    expect(result.warnings[0]).toMatch(/rpc/i);
  });
});

// ── Mocked RPC checks ──────────────────────────────────────────────────────

function mockRpc(responses: Record<string, string>) {
  vi.stubGlobal(
    "fetch",
    vi.fn((_url: string, opts: RequestInit) => {
      const body = JSON.parse(opts.body as string) as { params: [{ data: string }] };
      const data = body.params[0]?.data ?? "";
      const sel = data.slice(0, 10).toLowerCase();
      const result = responses[sel] ?? "0x" + "0".repeat(64);
      return Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ result }),
      });
    })
  );
}

function abiAddress(addr: string): string {
  return "0".repeat(24) + addr.slice(2).toLowerCase();
}

function abiBool(v: boolean): string {
  return "0".repeat(63) + (v ? "1" : "0");
}

beforeEach(() => vi.unstubAllGlobals());
afterEach(() => vi.unstubAllGlobals());

describe("checkSelfPromotion with RPC", () => {
  it("returns clean when owner is different and not approved", async () => {
    mockRpc({
      [SEL_OWNER_OF]: abiAddress(OTHER),
      [SEL_IS_APPROVED_FOR_ALL]: abiBool(false),
      [SEL_GET_APPROVED]: abiAddress(OTHER),
    });
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
      rpcUrl: RPC,
    });
    expect(result.status).toBe("clean");
  });

  it("detects owner match → self", async () => {
    mockRpc({ [SEL_OWNER_OF]: abiAddress(CLIENT) });
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
      rpcUrl: RPC,
    });
    expect(result.status).toBe("self");
  });

  it("detects isApprovedForAll → operator", async () => {
    mockRpc({
      [SEL_OWNER_OF]: abiAddress(OTHER),
      [SEL_IS_APPROVED_FOR_ALL]: abiBool(true),
    });
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
      rpcUrl: RPC,
    });
    expect(result.status).toBe("operator");
  });

  it("detects getApproved match → approved", async () => {
    mockRpc({
      [SEL_OWNER_OF]: abiAddress(OTHER),
      [SEL_IS_APPROVED_FOR_ALL]: abiBool(false),
      [SEL_GET_APPROVED]: abiAddress(CLIENT),
    });
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
      rpcUrl: RPC,
    });
    expect(result.status).toBe("approved");
  });

  it("returns rpc-error and warning when fetch throws", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network down"))));
    const result = await checkSelfPromotion({
      agentId: 42n,
      clientAddress: CLIENT,
      rpcUrl: RPC,
    });
    expect(result.status).toBe("rpc-error");
    expect(result.warnings[0]).toMatch(/network down/);
  });
});
