// Integration: cli-bootstrap ticket flow.
//
// Server contract: issue → returns UUID. Redeem → returns the keypair payload
// once; second redeem → 404 (single-use). Mirrors mcp/src/api.rs.

import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";

import { startMockServer, type MockServer } from "../mock-server.js";

let server: MockServer;

beforeAll(async () => {
  server = await startMockServer();
});

afterAll(async () => {
  await server.close();
});

beforeEach(() => {
  server.reset();
});

describe("cli-bootstrap ticket", () => {
  it("issue → redeem returns the payload (200)", async () => {
    const payload = {
      secret: Array.from({ length: 64 }, (_, i) => i % 256),
      pubkey_base58: "TestPubkey1",
    };
    const issueRes = await fetch(`${server.url}/api/cli-bootstrap/issue`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    expect(issueRes.status).toBe(200);
    // Production server returns `{ticket_id}` (mcp/src/api.rs::BootstrapIssueResponse).
    // The webapp IdentityPanel consumes `body.ticket_id`; pin the mock to the
    // same shape so a regression that drops the field fails the integration suite.
    const { ticket_id: ticketId } = (await issueRes.json()) as {
      ticket_id: string;
    };
    expect(ticketId).toMatch(/^[0-9a-f-]+$/);

    const redeemRes = await fetch(
      `${server.url}/api/cli-bootstrap/redeem/${encodeURIComponent(ticketId)}`
    );
    expect(redeemRes.status).toBe(200);
    const got = (await redeemRes.json()) as typeof payload;
    expect(got.pubkey_base58).toBe("TestPubkey1");
    expect(got.secret).toEqual(payload.secret);
  });

  it("second redeem returns 404 (single-use)", async () => {
    const ticket = server.issueTicketSync({
      secret: Array(64).fill(0),
      pubkey_base58: "TestPubkey2",
    });
    const first = await fetch(
      `${server.url}/api/cli-bootstrap/redeem/${ticket}`
    );
    expect(first.status).toBe(200);
    const second = await fetch(
      `${server.url}/api/cli-bootstrap/redeem/${ticket}`
    );
    expect(second.status).toBe(404);
  });

  it("unknown ticket returns 404", async () => {
    const res = await fetch(
      `${server.url}/api/cli-bootstrap/redeem/00000000-0000-0000-0000-000000000000`
    );
    expect(res.status).toBe(404);
  });
});
