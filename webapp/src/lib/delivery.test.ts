import { describe, expect, it } from "vitest";
import { checkDelivery, isArweaveId } from "./delivery";

const ID = "rBjKMGzX-t-oYMOaKkXOE42q-1UN-YkPT7fFuVhNJDc";

function fake(graphql: unknown, turbo: { status: number; body?: unknown }): typeof fetch {
  return (async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.endsWith("/graphql")) return new Response(JSON.stringify(graphql), { status: 200 });
    if (url.includes("/v1/tx/")) {
      return new Response(turbo.body === undefined ? "" : JSON.stringify(turbo.body), {
        status: turbo.status,
      });
    }
    throw new Error(`unexpected ${url}`);
  }) as typeof fetch;
}

const pending = { data: { transaction: null } };

describe("checkDelivery", () => {
  it("reports permanent with the arweave.net link once the gateway shows a block", async () => {
    const s = await checkDelivery(
      ID,
      fake({ data: { transaction: { block: { height: 2018533 } } } }, { status: 200, body: {} }),
    );
    expect(s).toEqual({
      stage: "permanent",
      blockHeight: 2018533,
      readUrl: `https://arweave.net/${ID}`,
    });
  });

  it("reports uploaded and links the early gateway while Turbo says new", async () => {
    const s = await checkDelivery(
      ID,
      fake(pending, { status: 200, body: { status: "CONFIRMED", info: "new" } }),
    );
    expect(s).toEqual({ stage: "uploaded", readUrl: `https://turbo-gateway.com/${ID}` });
  });

  it("reports bundled with the bundle id while the bundle waits to be mined", async () => {
    const s = await checkDelivery(
      ID,
      fake(pending, { status: 200, body: { info: "pending", bundleId: "b".repeat(43) } }),
    );
    expect(s.stage).toBe("bundled");
    expect(s.bundleId).toBe("b".repeat(43));
    expect(s.readUrl).toBe(`https://turbo-gateway.com/${ID}`);
  });

  it("reports unknown without a link when Turbo does not know the id", async () => {
    expect(await checkDelivery(ID, fake(pending, { status: 404 }))).toEqual({
      stage: "unknown",
      readUrl: null,
    });
  });

  it("rejects ids that are not Arweave ids without any request", async () => {
    const never = (async () => {
      throw new Error("must not fetch");
    }) as typeof fetch;
    expect(isArweaveId("local:abc")).toBe(false);
    expect(await checkDelivery("local:abc", never)).toEqual({ stage: "unknown", readUrl: null });
  });
});
