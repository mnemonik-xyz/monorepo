import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// The consent page POSTs to the operator named in `mcp_base`; the browser
// blocks any origin missing from connect-src with "Failed to fetch".
const indexHtml = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../../index.html"),
  "utf8",
);

function connectSrc(html: string): string[] {
  const policy = /http-equiv="Content-Security-Policy"\s+content="([^"]*)"/.exec(html)?.[1] ?? "";
  const directive = policy.split(";").map((d) => d.trim()).find((d) => d.startsWith("connect-src "));
  return directive?.split(/\s+/).slice(1) ?? [];
}

describe("index.html CSP", () => {
  it("allows the consent page to reach every hosted MCP operator", () => {
    const allowed = connectSrc(indexHtml);
    expect(allowed).toContain("https://mcp.mnemonik.xyz");
    // Additional operators (mcp2, ...) are subdomains; no rebuild per server.
    expect(allowed).toContain("https://*.mnemonik.xyz");
  });
});
