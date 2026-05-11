// Per-domain section — toggle persistence round-trip.
//
// Asserts:
//   1. auto-capture defaults to OFF for every supported domain (D12).
//   2. Toggling auto-capture for one domain calls
//      `settings.update({ per_domain: { <host>: { … auto_capture: true } } })`.

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { PerDomain } from "../../../src/options/sections/PerDomain.js";
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
      countCloudAttestations: async () => 0,
      enqueueAll: async () => undefined,
      subscribeProgress: () => () => undefined,
      exportAll: async () => new Uint8Array(),
    },
    about: { version: "0.1.0" },
  };
}

describe("Per-domain section", () => {
  beforeEach(() => setOptionsRuntime(null));
  afterEach(() => setOptionsRuntime(null));

  it("auto-capture defaults to OFF for every supported domain (D12)", async () => {
    setOptionsRuntime(makeRuntime());
    render(<PerDomain />);
    // One auto-capture toggle per domain — all start unchecked.
    const toggles = await screen.findAllByLabelText(/auto-capture/i);
    expect(toggles.length).toBeGreaterThanOrEqual(3);
    for (const t of toggles) {
      expect((t as HTMLInputElement).checked).toBe(false);
    }
  });

  it("toggling auto-capture persists via settings.update (round-trip)", async () => {
    const update = vi
      .fn<[Partial<Omit<SettingsV1, "version">>], Promise<SettingsV1>>()
      .mockImplementation(async () => DEFAULT_SETTINGS_V1);
    setOptionsRuntime(makeRuntime({ update }));
    render(<PerDomain />);

    // Find the auto-capture toggle for chatgpt.com specifically.
    const toggle = await screen.findByLabelText(/chatgpt\.com.*auto-capture/i);
    fireEvent.click(toggle);
    await waitFor(() => expect(update).toHaveBeenCalled());
    const patch = update.mock.calls[0]?.[0];
    expect(patch?.per_domain?.["chatgpt.com"]?.auto_capture).toBe(true);
    // Other fields preserved.
    expect(patch?.per_domain?.["chatgpt.com"]?.enabled).toBe(true);
    expect(patch?.per_domain?.["chatgpt.com"]?.fab_visible).toBe(true);
  });
});
