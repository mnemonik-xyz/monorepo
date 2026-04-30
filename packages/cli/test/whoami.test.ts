// `mnemonic whoami` — TDD anchor: by default, no fetch is called.
// Drives Decision 14 (whoami is client-side; server's whoami returns the
// SERVER's identity, not the user's).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runWhoami } from "../src/commands/whoami.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("runWhoami", () => {
  let cleanup = (): void => {};
  let realFetch: typeof globalThis.fetch | undefined;
  let fetchSpy: ReturnType<typeof vi.fn>;
  let mock: ReturnType<typeof installWasmMock>;

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    mock = installWasmMock();
    realFetch = globalThis.fetch;
    fetchSpy = vi.fn(async () => new Response("{}", { status: 200 }));
    globalThis.fetch = fetchSpy as never;
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    if (realFetch) globalThis.fetch = realFetch;
    cleanup();
  });

  it("TDD anchor: default whoami calls fetch ZERO times", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    saveToken({
      jwt: makeJwt(kp.pubkey_base58),
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: kp.pubkey_base58,
    });
    await runWhoami({});
    expect(fetchSpy).toHaveBeenCalledTimes(0);
  });

  it("works without identity (prints null fields)", async () => {
    await runWhoami({ json: true });
    expect(fetchSpy).toHaveBeenCalledTimes(0);
  });

  it("signer_match=true when identity.pubkey === jwt.sub", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    saveToken({
      jwt: makeJwt(kp.pubkey_base58),
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
      sub: kp.pubkey_base58,
    });
    let captured = "";
    vi.spyOn(process.stdout, "write").mockImplementation((c: unknown) => {
      captured += String(c);
      return true;
    });
    await runWhoami({ json: true });
    const obj = JSON.parse(captured);
    expect(obj.signer_match).toBe(true);
    expect(obj.pubkey).toBe(kp.pubkey_base58);
  });
});
