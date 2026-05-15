// E2E coverage for the mcp_auth bridge tool + WWW-Authenticate header +
// path-specific protected-resource metadata. Tier 1 of the cursor-vscode-
// e2e-tests feature (work/cursor-vscode-e2e-tests/tech-spec.md).
//
// Hits the live mcp.mnemonik.xyz backend (or VITE_MCP_BASE override). All
// tests are network-only — no browser or webapp UI involvement.

import { test, expect } from "@playwright/test";
import { mcpBase } from "./_helpers";

// /oauth/* is rate-limited at 5/min/IP (tower_governor + Decision 9).
// These tests don't hit /oauth/* directly but stay serial-no-retries to
// match house style for live-server tests.
test.describe.configure({ mode: "serial", retries: 0 });

test("WWW-Authenticate header is present on 401 from /mcp", async ({
  request,
}) => {
  // Per MCP authorization spec + RFC 6750 §3, every 401 from a Bearer-
  // protected resource MUST include a WWW-Authenticate header so the
  // client can discover where to authenticate. Cursor / Claude.ai recent
  // MCP OAuth providers fail silently when this header is missing.
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: { "content-type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "mnemonic_recall", arguments: { query: "x" } },
    },
  });
  expect(r.status(), "tools/call without JWT must 401").toBe(401);
  const wwwAuth = r.headers()["www-authenticate"];
  expect(wwwAuth, "WWW-Authenticate header must be present").toBeTruthy();
  expect(wwwAuth.toLowerCase()).toContain("bearer");
  expect(wwwAuth).toContain("resource_metadata=");
  expect(wwwAuth).toContain('error="invalid_token"');
});

test("/.well-known/oauth-protected-resource (root) returns origin-shaped metadata", async ({
  request,
}) => {
  const r = await request.get(
    `${mcpBase()}/.well-known/oauth-protected-resource`
  );
  expect(r.status()).toBe(200);
  const body = await r.json();
  expect(body.resource).toBe(mcpBase());
  expect(body.authorization_servers).toEqual([mcpBase()]);
  expect(body.scopes_supported).toContain("mcp");
  expect(body.bearer_methods_supported).toContain("header");
});

test("/.well-known/oauth-protected-resource/mcp returns path-specific metadata (RFC 9728 §3.1)", async ({
  request,
}) => {
  // Cursor's MCP OAuth provider (3.2+) probes this path-specific endpoint
  // BEFORE the root variant. Without it returning the path-qualified
  // resource value, Cursor silently aborts the OAuth flow.
  const r = await request.get(
    `${mcpBase()}/.well-known/oauth-protected-resource/mcp`
  );
  expect(r.status()).toBe(200);
  const body = await r.json();
  expect(
    body.resource,
    "path-specific resource MUST equal the URL the MCP client connects to"
  ).toBe(`${mcpBase()}/mcp`);
  expect(body.authorization_servers).toEqual([mcpBase()]);
});

test("tools/list (no JWT) advertises all 7 tools including mcp_auth", async ({
  request,
}) => {
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: { "content-type": "application/json" },
    data: { jsonrpc: "2.0", id: 1, method: "tools/list" },
  });
  expect(r.status()).toBe(200);
  const body = await r.json();
  const names: string[] = body.result.tools.map(
    (t: { name: string }) => t.name
  );
  expect(names).toEqual(
    expect.arrayContaining([
      "mnemonic_whoami",
      "mnemonic_sign_memory",
      "mnemonic_verify",
      "mnemonic_prove_identity",
      "mnemonic_recall",
      "mcp_auth",
      "mnemonic_check_pending",
    ])
  );
  expect(names.length).toBe(7);
});

test("mcp_auth callable WITHOUT JWT — returns unauthorized + install_url", async ({
  request,
}) => {
  // The whole point of mcp_auth: callable without a token, returns the URL
  // an unauthenticated client should send the user to.
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: { "content-type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "mcp_auth", arguments: {} },
    },
  });
  expect(r.status(), "mcp_auth must succeed without a JWT").toBe(200);
  const env = await r.json();
  // tools/call response wraps the result as {content: [{type, text}]}.
  // The text field is JSON-encoded; parse it to inspect the payload.
  const text = env.result.content[0].text;
  const payload = JSON.parse(text);
  expect(payload.status).toBe("unauthorized");
  expect(payload.install_url).toBe("https://mnemonik.xyz/install");
  expect(typeof payload.instructions).toBe("string");
  expect(payload.instructions.length).toBeGreaterThan(50);
});

test("mcp_auth unauthorized response includes per-client reconnect paths", async ({
  request,
}) => {
  // Regression guard: the single `install_url` is misleading for Claude.ai
  // users, whose connector OAuth only re-fires on Settings → Connectors →
  // Disconnect/Reconnect (not on the install page). The response must carry
  // a structured `client_paths` map so the agent can render the right path
  // for the chat / IDE it is running in.
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: { "content-type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "mcp_auth", arguments: {} },
    },
  });
  expect(r.status()).toBe(200);
  const payload = JSON.parse((await r.json()).result.content[0].text);

  expect(payload.client_paths).toBeTruthy();
  for (const key of ["claude_ai", "cursor", "vscode", "chatgpt", "other"]) {
    expect(payload.client_paths[key], `missing client_paths.${key}`).toBeTruthy();
    expect(typeof payload.client_paths[key].title).toBe("string");
    expect(Array.isArray(payload.client_paths[key].steps)).toBe(true);
    expect(payload.client_paths[key].steps.length).toBeGreaterThan(0);
  }

  // Claude.ai branch must point at the connectors UI, NOT the install page —
  // that's the whole bug this regression test exists to prevent.
  expect(payload.client_paths.claude_ai.reconnect_url).toBe(
    "https://claude.ai/settings/connectors"
  );
  const claudeJoined =
    payload.client_paths.claude_ai.steps.join(" ").toLowerCase();
  expect(claudeJoined).toContain("disconnect");
  expect(claudeJoined).toContain("connect");
});

test("non-allowlisted tools/call still 401s without JWT", async ({
  request,
}) => {
  // The unauth allowlist is narrow: only mcp_auth. Every other tools/call
  // must still gate on JWT. Regression guard against accidental over-
  // allowlisting.
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: { "content-type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "mnemonic_whoami", arguments: {} },
    },
  });
  expect(r.status()).toBe(401);
});
