// Telemetry section — off-by-default + opt-in toggle persists.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { Telemetry } from "../../../src/options/sections/Telemetry.js";
import {
  setOptionsRuntime,
  type OptionsRuntime,
} from "../../../src/options/runtime.js";
import { DEFAULT_SETTINGS_V1, type SettingsV1 } from "../../../src/settings.js";

function makeRuntime(
  overrides: {
    initial?: SettingsV1;
    update?: OptionsRuntime["settings"]["update"];
  } = {},
): OptionsRuntime {
  let settings: SettingsV1 = overrides.initial ?? DEFAULT_SETTINGS_V1;
  return {
    settings: {
      load: async () => settings,
      update:
        overrides.update ??
        (async (patch) => {
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
        }),
    },
    auth: {
      signIn: async () => ({ google_sub: "g", email: "u", jwt: "j" }),
      getSession: async () => null,
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
      countLocalAttestations: async () => 0,
      enqueueAll: async () => undefined,
      subscribeProgress: () => () => undefined,
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Telemetry section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("telemetry is off by default", async () => {
    setOptionsRuntime(makeRuntime());
    render(<Telemetry />);
    const toggle = (await screen.findByLabelText(
      /enable telemetry/i,
    )) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
  });

  it("opt-in toggle persists via settings.update", async () => {
    const update = vi
      .fn<[Partial<Omit<SettingsV1, "version">>], Promise<SettingsV1>>()
      .mockResolvedValue({
        ...DEFAULT_SETTINGS_V1,
        telemetry: { enabled: true },
      });
    setOptionsRuntime(makeRuntime({ update }));
    render(<Telemetry />);
    const toggle = await screen.findByLabelText(/enable telemetry/i);
    fireEvent.click(toggle);
    await waitFor(() => expect(update).toHaveBeenCalled());
    expect(update.mock.calls[0]?.[0].telemetry?.enabled).toBe(true);
  });
});
