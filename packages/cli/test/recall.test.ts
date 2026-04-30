// `mnemonic recall` — happy path, --top-k, --tag.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runRecall } from "../src/commands/recall.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import { ServerError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("runRecall", () => {
  let cleanup = (): void => {};
  let realFetch: typeof globalThis.fetch | undefined;
  let mock: ReturnType<typeof installWasmMock>;

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    mock = installWasmMock();
    realFetch = globalThis.fetch;
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    saveToken({
      jwt: makeJwt(kp.pubkey_base58),
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: kp.pubkey_base58,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    if (realFetch) globalThis.fetch = realFetch;
    cleanup();
  });

  function recallResponse(
    hits: Array<{
      attestation_id: string;
      similarity: number;
      content: string;
      tags?: string[];
    }>,
    total: number
  ): Response {
    return new Response(
      JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        result: {
          content: [
            {
              type: "text",
              text: JSON.stringify({ hits, total }),
            },
          ],
        },
      }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  }

  it("happy path: passes topK + tag through to /mcp", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body ?? "{}"));
        const args = body.params?.arguments ?? {};
        expect(args.query).toBe("hello");
        expect(args.top_k).toBe(3);
        expect(args.tags).toEqual(["foo"]);
        return recallResponse(
          [
            {
              attestation_id: "att-1",
              similarity: 0.9,
              content: "hi",
              tags: ["foo"],
            },
          ],
          1
        );
      }
    );
    globalThis.fetch = fetchMock as never;

    await runRecall("hello", {
      baseUrl: "http://test",
      topK: 3,
      tag: "foo",
    });
    expect(fetchMock).toHaveBeenCalled();
  });

  it("default top-k = 5 when not specified", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body ?? "{}"));
        const args = body.params?.arguments ?? {};
        expect(args.top_k).toBe(5);
        return recallResponse([], 0);
      }
    );
    globalThis.fetch = fetchMock as never;
    await runRecall("hi", { baseUrl: "http://test" });
  });

  it("empty query throws UserError", async () => {
    await expect(
      runRecall("", { baseUrl: "http://test" })
    ).rejects.toMatchObject({
      exitCode: 1,
    });
  });

  it("ServerError (exit 2) when /mcp returns 500", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ error: "internal" }), {
          status: 500,
          headers: { "content-type": "application/json" },
        })
    );
    globalThis.fetch = fetchMock as never;
    await expect(
      runRecall("hello", { baseUrl: "http://test" })
    ).rejects.toMatchObject({ exitCode: 2 });
    await expect(
      runRecall("hello", { baseUrl: "http://test" })
    ).rejects.toBeInstanceOf(ServerError);
  });
});
