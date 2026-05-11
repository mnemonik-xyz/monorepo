// Security section — D13 TDD anchor `rotate_passphrase_re_encrypts_blob`.
//
// The test mounts <Security />, types an old + new passphrase, submits,
// and asserts the key-escrow facade's `rotate(old, new)` is called.
// T17 will replace the facade stub with the real client; this test
// fixes the contract.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Security } from "../../../src/options/sections/Security.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
  type AuthSession,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1 } from "../../../src/settings.js";

function makeRuntime(
  overrides: {
    session?: AuthSession | null;
    hasBlob?: boolean;
    rotate?: OptionsRuntime["keyEscrow"]["rotate"];
    del?: OptionsRuntime["keyEscrow"]["delete"];
    clearSession?: OptionsRuntime["auth"]["clearSession"];
  } = {},
): OptionsRuntime {
  return {
    settings: {
      load: async () => DEFAULT_SETTINGS_V1,
      update: async () => DEFAULT_SETTINGS_V1,
    },
    auth: {
      signIn: async () => ({ google_sub: "g", email: "u", jwt: "j" }),
      getSession: async () => overrides.session ?? null,
      clearSession: overrides.clearSession ?? (async () => undefined),
    },
    identity: {
      load: async () => null,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
    },
    keyEscrow: {
      rotate: overrides.rotate ?? (async () => undefined),
      delete: overrides.del ?? (async () => undefined),
      hasBlob: async () => overrides.hasBlob ?? false,
    },
    cloudSync: {
      countLocalAttestations: async () => 0,
      enqueueAll: async () => undefined,
      subscribeProgress: () => () => undefined,
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Security section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("renders Local-tier banner when no Google session is active", async () => {
    setOptionsRuntime(makeRuntime({ session: null }));
    render(<Security />);
    expect(await screen.findByText(/Cloud tier required/i)).toBeInTheDocument();
  });

  it("rotate_passphrase_re_encrypts_blob", async () => {
    // TDD anchor: submitting the rotate form calls
    // keyEscrow.rotate(old, new) exactly once with the typed values.
    const rotate = vi.fn<[string, string], Promise<void>>().mockResolvedValue();
    setOptionsRuntime(
      makeRuntime({
        session: { google_sub: "g", email: "u@x", jwt: "j" },
        hasBlob: true,
        rotate,
      }),
    );

    render(<Security />);
    const oldInput = await screen.findByLabelText(/Current passphrase/i);
    const newInput = await screen.findByLabelText(/New passphrase/i);
    fireEvent.change(oldInput, { target: { value: "old-pp" } });
    fireEvent.change(newInput, { target: { value: "new-pp-1234" } });

    fireEvent.click(screen.getByRole("button", { name: /rotate passphrase/i }));
    await waitFor(() => expect(rotate).toHaveBeenCalledTimes(1));
    expect(rotate).toHaveBeenCalledWith("old-pp", "new-pp-1234");
  });

  it("blocks rotation when fields are empty", async () => {
    const rotate = vi.fn<[string, string], Promise<void>>();
    setOptionsRuntime(
      makeRuntime({
        session: { google_sub: "g", email: "u@x", jwt: "j" },
        hasBlob: true,
        rotate,
      }),
    );
    render(<Security />);
    fireEvent.click(
      await screen.findByRole("button", { name: /rotate passphrase/i }),
    );
    // Error toast renders; rotate is NOT called.
    expect(await screen.findByRole("status")).toBeInTheDocument();
    expect(rotate).not.toHaveBeenCalled();
  });

  it("Sign out of Google clears the session", async () => {
    const clearSession = vi.fn<[], Promise<void>>().mockResolvedValue();
    setOptionsRuntime(
      makeRuntime({
        session: { google_sub: "g", email: "u@x", jwt: "j" },
        clearSession,
      }),
    );
    render(<Security />);
    fireEvent.click(
      await screen.findByRole("button", { name: /sign out of google/i }),
    );
    await waitFor(() => expect(clearSession).toHaveBeenCalledTimes(1));
  });
});
