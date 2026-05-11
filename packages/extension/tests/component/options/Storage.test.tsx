// Storage section — D7 explicit-tier-switch + T16 sign-in gating.
//
// TDD anchor (D13): `switching_to_cloud_requires_google_signin` —
// clicking "Switch to Cloud" with no Google session must surface a
// sign-in prompt instead of confirming the migration. The runtime
// facade is the only seam mocked; production code goes through the
// same `getOptionsRuntime()` lookup.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Storage } from "../../../src/options/sections/Storage.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
  type AuthSession,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1, type SettingsV1 } from "../../../src/settings.js";

function makeRuntime(
  overrides: {
    settings?: Partial<SettingsV1>;
    session?: AuthSession | null;
    signIn?: OptionsRuntime["auth"]["signIn"];
    enqueueAll?: OptionsRuntime["cloudSync"]["enqueueAll"];
    countLocal?: OptionsRuntime["cloudSync"]["countLocalAttestations"];
  } = {},
): OptionsRuntime {
  let settings: SettingsV1 = {
    ...DEFAULT_SETTINGS_V1,
    ...overrides.settings,
  };
  return {
    settings: {
      load: async () => settings,
      update: async (patch) => {
        settings = {
          ...settings,
          ...patch,
          version: 1,
          per_domain: patch.per_domain
            ? { ...settings.per_domain, ...patch.per_domain }
            : settings.per_domain,
          telemetry: patch.telemetry
            ? { ...settings.telemetry, ...patch.telemetry }
            : settings.telemetry,
        };
        return settings;
      },
    },
    auth: {
      signIn:
        overrides.signIn ??
        (async () => {
          throw new Error("signIn not configured by test");
        }),
      getSession: async () => overrides.session ?? null,
      clearSession: async () => undefined,
    },
    identity: {
      load: async () => null,
      exportEncrypted: async () => new Uint8Array(),
      importEncrypted: async () => ({ pubkey_base58: "x", did: "did:sol:x" }),
    },
    keyEscrow: {
      rotate: async () => undefined,
      delete: async () => undefined,
      hasBlob: async () => false,
    },
    cloudSync: {
      countLocalAttestations: overrides.countLocal ?? (async () => 0),
      enqueueAll: overrides.enqueueAll ?? (async () => undefined),
      subscribeProgress: (cb) => {
        const t = setTimeout(
          () => cb({ attempted: 0, flushed: 0, total: 0, done: true }),
          0,
        );
        return () => clearTimeout(t);
      },
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Storage section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("renders the active tier read-only", async () => {
    setOptionsRuntime(makeRuntime({ settings: { storage_tier: "local" } }));
    render(<Storage />);
    expect(await screen.findByText(/Active tier/i)).toBeInTheDocument();
    expect(await screen.findByText(/^Local$/i)).toBeInTheDocument();
  });

  it("switching_to_cloud_requires_google_signin", async () => {
    // TDD anchor: with no Google session, clicking "Switch to Cloud"
    // routes through a sign-in prompt — the migration confirmation
    // dialog must NOT be shown until a session exists.
    const signIn = vi.fn<[], Promise<AuthSession>>();
    const enqueueAll = vi.fn<[], Promise<void>>();
    setOptionsRuntime(
      makeRuntime({
        settings: { storage_tier: "local" },
        session: null,
        signIn,
        enqueueAll,
      }),
    );

    render(<Storage />);
    const switchBtn = await screen.findByRole("button", {
      name: /switch to cloud/i,
    });
    fireEvent.click(switchBtn);

    // The dialog opens in sign-in mode, not confirm-migration mode.
    // The dialog renders both a title and a button labelled "Sign in
    // with Google" — at least one match is enough to assert the prompt.
    const signInPrompts = await screen.findAllByText(/Sign in with Google/i);
    expect(signInPrompts.length).toBeGreaterThan(0);
    // Confirm copy must NOT appear yet.
    expect(screen.queryByText(/will upload/i)).not.toBeInTheDocument();
    expect(enqueueAll).not.toHaveBeenCalled();
  });

  it("with a live session, opens the migration confirmation dialog", async () => {
    const enqueueAll = vi.fn<[], Promise<void>>();
    const session: AuthSession = {
      google_sub: "g-1",
      email: "u@gmail.com",
      jwt: "jwt",
    };
    setOptionsRuntime(
      makeRuntime({
        settings: { storage_tier: "local" },
        session,
        countLocal: async () => 7,
        enqueueAll,
      }),
    );

    render(<Storage />);
    const switchBtn = await screen.findByRole("button", {
      name: /switch to cloud/i,
    });
    fireEvent.click(switchBtn);

    // Confirmation dialog cites the row count.
    await waitFor(() =>
      expect(screen.getByText(/will upload 7/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /^confirm/i }));
    await waitFor(() => expect(enqueueAll).toHaveBeenCalledTimes(1));
  });
});
