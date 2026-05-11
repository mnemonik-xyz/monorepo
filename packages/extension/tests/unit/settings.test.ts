// Settings module — round-trip + defaults + migration validator.
//
// The module is the single source of truth for `chrome.storage.local`
// reads/writes under `settings.v1`. Defaults must honour D7 (local
// tier), D11 (enumerated supported domains only), and D12 (auto-capture
// off). The migration helper today validates v1 and falls back to
// defaults on corrupt rows; future versions branch on `version`.

import { describe, it, expect, beforeEach } from "vitest";
import {
  loadSettings,
  saveSettings,
  updateSettings,
  migrateAndValidate,
  DEFAULT_SETTINGS_V1,
  SETTINGS_KEY_V1,
  SUPPORTED_DOMAINS,
  type SettingsV1,
} from "../../src/settings.js";

interface ChromeStorageStub {
  storage: {
    local: {
      get: (keys: string[]) => Promise<Record<string, unknown>>;
      set: (items: Record<string, unknown>) => Promise<void>;
    };
  };
}

let backingStore: Record<string, unknown>;

function installChromeStub(): void {
  backingStore = {};
  const chromeStub: ChromeStorageStub = {
    storage: {
      local: {
        get: async (keys: string[]) => {
          const out: Record<string, unknown> = {};
          for (const k of keys) {
            if (k in backingStore) out[k] = backingStore[k];
          }
          return out;
        },
        set: async (items: Record<string, unknown>) => {
          Object.assign(backingStore, items);
        },
      },
    },
  };
  (globalThis as { chrome?: unknown }).chrome = chromeStub;
}

describe("settings module", () => {
  beforeEach(() => {
    installChromeStub();
  });

  it("loadSettings returns defaults when storage is empty", async () => {
    const s = await loadSettings();
    expect(s.version).toBe(1);
    expect(s.storage_tier).toBe("local"); // D7 default
    expect(s.telemetry.enabled).toBe(false); // off by default
    for (const host of SUPPORTED_DOMAINS) {
      expect(s.per_domain[host]?.auto_capture).toBe(false); // D12
      expect(s.per_domain[host]?.enabled).toBe(true);
      expect(s.per_domain[host]?.fab_visible).toBe(true);
    }
  });

  it("save then load round-trips the value", async () => {
    const written: SettingsV1 = {
      ...DEFAULT_SETTINGS_V1,
      storage_tier: "cloud",
      telemetry: { enabled: true },
      per_domain: {
        ...DEFAULT_SETTINGS_V1.per_domain,
        "chatgpt.com": {
          enabled: false,
          fab_visible: false,
          auto_capture: true,
        },
      },
    };
    await saveSettings(written);
    const read = await loadSettings();
    expect(read.storage_tier).toBe("cloud");
    expect(read.telemetry.enabled).toBe(true);
    expect(read.per_domain["chatgpt.com"]).toEqual({
      enabled: false,
      fab_visible: false,
      auto_capture: true,
    });
    // Untouched domain keeps its default.
    expect(read.per_domain["claude.ai"]?.auto_capture).toBe(false);
  });

  it("updateSettings deep-merges per_domain (single host patch keeps siblings)", async () => {
    await saveSettings(DEFAULT_SETTINGS_V1);
    await updateSettings({
      per_domain: {
        "chatgpt.com": {
          enabled: true,
          fab_visible: true,
          auto_capture: true,
        },
      },
    });
    const after = await loadSettings();
    expect(after.per_domain["chatgpt.com"]?.auto_capture).toBe(true);
    expect(after.per_domain["claude.ai"]?.auto_capture).toBe(false);
    expect(after.per_domain["gemini.google.com"]?.auto_capture).toBe(false);
  });

  it("auto_capture defaults to false even when storage row omits the field (D12)", async () => {
    backingStore[SETTINGS_KEY_V1] = {
      version: 1,
      storage_tier: "local",
      per_domain: {
        "chatgpt.com": {
          enabled: true,
          fab_visible: true /* no auto_capture */,
        },
      },
      telemetry: { enabled: false },
    };
    const s = await loadSettings();
    expect(s.per_domain["chatgpt.com"]?.auto_capture).toBe(false);
  });

  it("migrateAndValidate falls back to defaults on unknown version", () => {
    const corrupt = { version: 999, storage_tier: "cloud" };
    const out = migrateAndValidate(corrupt);
    expect(out.version).toBe(1);
    expect(out.storage_tier).toBe("local"); // default, NOT carried from corrupt row
  });

  it("migrateAndValidate coerces storage_tier to 'local' when malformed", () => {
    const out = migrateAndValidate({ version: 1, storage_tier: "wat" });
    expect(out.storage_tier).toBe("local");
  });

  it("migrateAndValidate returns defaults on null / non-object", () => {
    expect(migrateAndValidate(null).version).toBe(1);
    expect(migrateAndValidate(42).storage_tier).toBe("local");
    expect(migrateAndValidate("nope").telemetry.enabled).toBe(false);
  });
});
