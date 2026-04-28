/**
 * Playwright e2e shared helpers — Mnemonic Protocol.
 *
 * The webapp dev server runs at http://localhost:5173 (Vite). Tests can
 * point the webapp's MCP fetch base at either the LIVE deployment
 * (`https://mcp.mnemonik.xyz`) or a LOCAL release-build mnemonic-mcp
 * binary at `http://localhost:3000` via `VITE_MCP_BASE` env (Vite reads
 * it during the dev server bootstrap).
 *
 * The CORS policy on the server allows `http://localhost:*` and
 * `http://127.0.0.1:*` for dev convenience (see
 * `mcp/src/cors_policy.rs`), so cross-origin fetches from the webapp's
 * dev server to either backend variant Just Work.
 */

const DEFAULT_MCP_BASE = "https://mcp.mnemonik.xyz";

/**
 * Resolve the backend the test should hit. Mirrors the resolution logic
 * inside the webapp (Sign.tsx, Consent.tsx) so the test sees the same
 * server the React code talks to.
 */
export function mcpBase(): string {
  // Allow overriding via PLAYWRIGHT_MCP_BASE so CI can point at staging.
  // `process` typing comes from @types/node, which Playwright's project
  // pulls in transitively; cast to defeat strict-mode `process` lookup
  // in IDEs that scope-restrict d.ts resolution to `src/`.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const env = (globalThis as any).process?.env ?? {};
  return env.PLAYWRIGHT_MCP_BASE || env.VITE_MCP_BASE || DEFAULT_MCP_BASE;
}

/** Random hex string of `byteLen` bytes (state, code_verifier seed, etc). */
export function randomHex(byteLen: number): string {
  const buf = new Uint8Array(byteLen);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** RFC 7636 PKCE: SHA-256 of code_verifier, base64url no-pad. */
export async function pkceChallenge(verifier: string): Promise<string> {
  const data = new TextEncoder().encode(verifier);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return base64url(new Uint8Array(digest));
}

/** base64url no-pad. */
export function base64url(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s).replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
}

/** Standard base64 (with `=` padding) of a byte array. */
export function base64Std(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

/** Base64 (standard) → Uint8Array. */
export function base64StdToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
