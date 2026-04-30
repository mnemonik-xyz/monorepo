// `mnemonic prove` — generates an Ed25519 signature locally via WASM.
// Output shape: {pubkey, did, challenge, signature}.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runProve } from "../src/commands/prove.js";
import { saveIdentityJson } from "../src/config.js";
import { clearWasmMock, installWasmMock, withTmpConfigDir } from "./helpers.js";

describe("runProve", () => {
  let cleanup = (): void => {};
  let mock: ReturnType<typeof installWasmMock>;

  beforeEach(() => {
    cleanup = withTmpConfigDir().cleanup;
    mock = installWasmMock();
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    cleanup();
  });

  it("--challenge=<hex> signs offline with WASM mock", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);

    let captured = "";
    vi.spyOn(process.stdout, "write").mockImplementation((c: unknown) => {
      captured += String(c);
      return true;
    });

    const challenge = "0011223344"; // 5 bytes — short, but valid.
    await runProve({ json: true, challenge });
    const obj = JSON.parse(captured);
    expect(obj.pubkey).toBe(kp.pubkey_base58);
    expect(obj.did).toBe(`did:sol:${kp.pubkey_base58}`);
    expect(obj.challenge).toBe(challenge);
    // Ed25519 signature: 64 bytes = 128 hex chars.
    expect(obj.signature).toMatch(/^[0-9a-f]{128}$/);

    // Verified: WASM mock recorded one sign_challenge call.
    expect(mock.__calls.sign_challenge.length).toBe(1);
  });

  it("default challenge: generates 32 random bytes", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    let captured = "";
    vi.spyOn(process.stdout, "write").mockImplementation((c: unknown) => {
      captured += String(c);
      return true;
    });
    await runProve({ json: true });
    const obj = JSON.parse(captured);
    expect(obj.challenge).toMatch(/^[0-9a-f]{64}$/); // 32 bytes hex
  });

  it("rejects malformed hex", async () => {
    const kp = mock.generate_keypair();
    saveIdentityJson(kp);
    await expect(runProve({ challenge: "xyz" })).rejects.toMatchObject({
      exitCode: 1,
    });
  });
});
