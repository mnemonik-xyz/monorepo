// `mnemonic verify` — exit 0 verified, 3 tampered, 1 not_found.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runVerify } from "../src/commands/verify.js";
import { saveIdentityJson, saveToken } from "../src/config.js";
import { IntegrityError, UserError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("runVerify", () => {
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

  function reply(
    status: "verified" | "tampered" | "not_found",
    extra: Record<string, unknown> = {}
  ): Response {
    return new Response(
      JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        result: {
          content: [
            {
              type: "text",
              text: JSON.stringify({ status, ...extra }),
            },
          ],
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  }

  it("verified: returns without throwing (exit 0)", async () => {
    globalThis.fetch = (async () =>
      reply("verified", {
        signer: "abc",
        arweave_tx: "ar1",
        solana_tx: "sol1",
      })) as never;
    await expect(
      runVerify("att-1", { baseUrl: "http://t" })
    ).resolves.toBeUndefined();
  });

  it("tampered: throws IntegrityError (exit 3)", async () => {
    globalThis.fetch = (async () =>
      reply("tampered", { signer: "abc", reason: "hash drift" })) as never;
    await expect(
      runVerify("att-1", { baseUrl: "http://t" })
    ).rejects.toBeInstanceOf(IntegrityError);
  });

  it("not_found: throws UserError (exit 1)", async () => {
    globalThis.fetch = (async () => reply("not_found")) as never;
    await expect(
      runVerify("att-1", { baseUrl: "http://t" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("empty attestation_id throws UserError", async () => {
    await expect(runVerify("", { baseUrl: "http://t" })).rejects.toBeInstanceOf(
      UserError
    );
  });
});
