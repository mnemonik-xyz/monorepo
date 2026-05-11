// Identity section — basic render + export-keypair flow.
//
// The section reuses the popup `IdentityBadge` for the truncated pubkey
// and adds DID + export/import controls. Export calls
// `identity.exportEncrypted(passphrase)` with the user-typed passphrase.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Identity } from "../../../src/options/sections/Identity.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

function makeRuntime(
  overrides: {
    pubkey?: string | null;
    exportEncrypted?: OptionsRuntime["identity"]["exportEncrypted"];
  } = {},
): OptionsRuntime {
  return {
    settings: {
      load: async () => DEFAULT_SETTINGS_V1,
      update: async () => DEFAULT_SETTINGS_V1,
    },
    auth: {
      signIn: async () => ({ google_sub: "g", email: "u", jwt: "j" }),
      getSession: async () => null,
      clearSession: async () => undefined,
    },
    identity: {
      load: async () =>
        overrides.pubkey
          ? {
              pubkey_base58: overrides.pubkey,
              did: `did:sol:${overrides.pubkey}`,
            }
          : null,
      exportEncrypted:
        overrides.exportEncrypted ?? (async () => new Uint8Array([1, 2, 3, 4])),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
    },
    keyEscrow: {
      rotate: async () => undefined,
      delete: async () => undefined,
      hasBlob: async () => false,
    },
    cloudSync: {
      countLocalAttestations: async () => 0,
      countCloudAttestations: async () => 0,
      enqueueAll: async () => undefined,
      subscribeProgress: () => () => undefined,
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Identity section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("renders the empty state when no identity is configured", async () => {
    setOptionsRuntime(makeRuntime({ pubkey: null }));
    render(<Identity />);
    expect(
      await screen.findByText(/No agent identity found/i),
    ).toBeInTheDocument();
  });

  it("renders pubkey + DID when an identity is loaded", async () => {
    setOptionsRuntime(
      makeRuntime({ pubkey: "H8xYz123ABCDEFGabcdefg9876543210" }),
    );
    render(<Identity />);
    expect(
      await screen.findByText("did:sol:H8xYz123ABCDEFGabcdefg9876543210"),
    ).toBeInTheDocument();
  });

  it("export keypair calls exportEncrypted with the user passphrase", async () => {
    const exportEncrypted = vi
      .fn<[string], Promise<Uint8Array>>()
      .mockResolvedValue(new Uint8Array([0xde, 0xad]));
    setOptionsRuntime(
      makeRuntime({
        pubkey: "abcDEFghi",
        exportEncrypted,
      }),
    );
    render(<Identity />);
    const passInput = await screen.findByLabelText(/Export passphrase/i);
    // Long, non-dictionary passphrase clears the zxcvbn gate (>=12 chars
    // AND score >= 3) — required to enable the Export button.
    const strongPp = "Tr0ub4dor&3-Mnemonik-lazy-bear";
    fireEvent.change(passInput, { target: { value: strongPp } });
    fireEvent.click(screen.getByRole("button", { name: /^export keypair$/i }));
    await waitFor(() => expect(exportEncrypted).toHaveBeenCalledWith(strongPp));
  });
});
