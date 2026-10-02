// Shared test helpers: temp config dir + JWT fixture builders.

import { mkdtempSync, rmSync, readdirSync, readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Reach into the SDK's internal WASM injection hook + test helpers via
// relative paths because the SDK's package.json exports only the public
// barrel (`.`), and CLI tests need to mock crypto without forcing a real
// WASM init.
import { __setWasmForTesting } from "../../sdk/dist/wasm.js";
import { buildWasmMock } from "../../sdk/test/helpers/wasm-mock.js";

/** Mint a fresh tmpdir, point MNEMONIC_CONFIG_DIR at it, return cleanup. */
export function withTmpConfigDir(): { dir: string; cleanup: () => void } {
  const dir = mkdtempSync(join(tmpdir(), "mnemonic-cli-test-"));
  const prev = process.env.MNEMONIC_CONFIG_DIR;
  process.env.MNEMONIC_CONFIG_DIR = dir;
  return {
    dir,
    cleanup: () => {
      try {
        rmSync(dir, { recursive: true, force: true });
      } catch {
        /* ignore */
      }
      if (prev === undefined) delete process.env.MNEMONIC_CONFIG_DIR;
      else process.env.MNEMONIC_CONFIG_DIR = prev;
    },
  };
}

/** Build a fake HS256 JWT for sub `sub` with the given exp seconds. */
export function makeJwt(sub: string, expSecondsFromNow = 3600): string {
  const header = { alg: "HS256", typ: "JWT" };
  const payload = {
    sub,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + expSecondsFromNow,
  };
  const b64 = (o: unknown) =>
    Buffer.from(JSON.stringify(o), "utf8")
      .toString("base64")
      .replace(/=+$/g, "")
      .replace(/\+/g, "-")
      .replace(/\//g, "_");
  return `${b64(header)}.${b64(payload)}.signature`;
}

/** Install the WASM mock from the SDK's test helpers. Returns spy handle. */
export function installWasmMock(): ReturnType<typeof buildWasmMock> {
  const mock = buildWasmMock();
  // Deterministic test hash only; real BLAKE3 round trips have separate coverage.
  const hash = (bytes:Uint8Array) => createHash("sha256").update(bytes).digest();
  const seal = mock.seal_memory;
  mock.seal_memory = (...args) => {const row=seal(...args);return {...row,content_hash:new Uint8Array(hash(row.outer_cbor))};};
  __setWasmForTesting({...mock,blake3_hash_hex:(bytes:Uint8Array)=>hash(bytes).toString("hex")} as never);
  return mock;
}

export function clearWasmMock(): void {
  __setWasmForTesting(null);
}

/** Mock gateway reads of exactly the signed bytes persisted before upload. */
export function signedDeliveryResponse(url: string): Response | undefined {
  const root = join(process.env.MNEMONIC_CONFIG_DIR!, "sealed");
  if (!url.endsWith("/api/ingest-artifact") && !url.startsWith("https://gateway.irys.xyz/")) return undefined;
  const author = readdirSync(root)[0]!;
  const row = JSON.parse(readFileSync(join(root,author,readdirSync(join(root,author))[0]!),"utf8"));
  if (url.endsWith("/api/ingest-artifact")) return new Response(JSON.stringify({delivery_status:"verified",locator:`ar://${"a".repeat(43)}`,content_hash:row.memoryHash,author}),{headers:{"content-type":"application/json"}});
  return new Response(Buffer.from(row.signedBytes,"base64"));
}
