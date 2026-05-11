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
      generate: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      clear: async () => undefined,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
      exportPlain: async () => ({
        version: 1,
        pubkey_base58: "x",
        secret: Array.from({ length: 64 }, () => 0),
        exported_at: "2026-05-11T00:00:00.000Z",
        warning: "",
      }),
      importPlain: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
    },
    keyEscrow: {
      rotate: overrides.rotate ?? (async () => undefined),
      delete: overrides.del ?? (async () => undefined),
      hasBlob: async () => overrides.hasBlob ?? false,
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
    // The new passphrase must clear the zxcvbn gate (>=12 chars AND
    // score >= 3) — pick a long, non-dictionary value.
    const strongPp = "Tr0ub4dor&3-Mnemonik-lazy-bear";
    fireEvent.change(newInput, { target: { value: strongPp } });

    fireEvent.click(screen.getByRole("button", { name: /rotate passphrase/i }));
    await waitFor(() => expect(rotate).toHaveBeenCalledTimes(1));
    expect(rotate).toHaveBeenCalledWith("old-pp", strongPp);
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
    const submit = await screen.findByRole("button", {
      name: /rotate passphrase/i,
    });
    // The submit button stays disabled until BOTH fields are filled and
    // the new passphrase clears the zxcvbn gate. Clicking a disabled
    // button does not fire onSubmit.
    expect(submit).toBeDisabled();
    fireEvent.click(submit);
    expect(rotate).not.toHaveBeenCalled();
  });

  it("blocks rotation when new passphrase is too weak", async () => {
    const rotate = vi.fn<[string, string], Promise<void>>();
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
    // 12 chars but trivial sequence — zxcvbn rates this < 3.
    fireEvent.change(newInput, { target: { value: "password1234" } });
    const submit = await screen.findByRole("button", {
      name: /rotate passphrase/i,
    });
    expect(submit).toBeDisabled();
    expect(rotate).not.toHaveBeenCalled();
  });

  it("rotate form shows the 'cannot recover' explainer", async () => {
    setOptionsRuntime(
      makeRuntime({
        session: { google_sub: "g", email: "u@x", jwt: "j" },
        hasBlob: true,
      }),
    );
    render(<Security />);
    expect(
      await screen.findByText(/Mnemonik cannot recover this passphrase/i),
    ).toBeInTheDocument();
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
