// Typed settings module for the extension. All user-tunable preferences
// live here behind a single versioned key in `chrome.storage.local`
// (`settings.v1`). The shape and the storage key are intentionally
// versioned: a future schema bump (`settings.v2`) ships a migration
// helper alongside without ever rewriting the v1 row in place.
//
// Per D7 (explicit tier switch), D11 (enumerated host_permissions), and
// D12 (auto-capture opt-in by default), the defaults below MUST stay:
//   - `storage_tier: "local"`
//   - per-domain `auto_capture: false`
//   - `telemetry.enabled: false`
//
// The list of supported domains mirrors the enumerated host_permissions
// in `manifest.json` (D11). Adding a new domain requires both a manifest
// change and an entry here.
//
// All getters are defensive: malformed or missing storage rows fall back
// to {@link DEFAULT_SETTINGS_V1} so the options page stays renderable
// even on a fresh profile.

export const SETTINGS_KEY_V1 = "settings.v1";
export const SETTINGS_VERSION_LATEST = 1;

/** Per-domain feature toggles. Domains are matched against
 *  `location.hostname` by the content scripts. */
export interface PerDomainSettings {
  /** Content script + adapter active on this domain. Default true; user
   *  can disable per-domain without changing host_permissions (D11). */
  enabled: boolean;
  /** FAB injected on this domain. Default true. */
  fab_visible: boolean;
  /** Auto-capture every assistant turn. Default FALSE per D12 — privacy
   *  default; user opts in per-domain in the Options page. */
  auto_capture: boolean;
}

export interface TelemetrySettings {
  /** Off by default. Tech-spec ships only broken-adapter / cold-start /
   *  sync-error counters when on; no payload, no PII. */
  enabled: boolean;
}

export interface SettingsV1 {
  version: 1;
  /** Active storage tier. Switching is explicit and gated by a
   *  confirmation dialog (D7). */
  storage_tier: "local" | "cloud";
  /** Per-host overrides keyed by `location.hostname`. */
  per_domain: Record<string, PerDomainSettings>;
  telemetry: TelemetrySettings;
}

/** Phase-1 supported AI-chat domains. Mirrors enumerated
 *  host_permissions in `manifest.json` per D11. */
export const SUPPORTED_DOMAINS = [
  "chatgpt.com",
  "claude.ai",
  "gemini.google.com",
] as const;

export type SupportedDomain = (typeof SUPPORTED_DOMAINS)[number];

const DEFAULT_PER_DOMAIN: PerDomainSettings = {
  enabled: true,
  fab_visible: true,
  auto_capture: false, // D12 — opt-in per domain
};

function defaultPerDomain(): Record<string, PerDomainSettings> {
  const out: Record<string, PerDomainSettings> = {};
  for (const host of SUPPORTED_DOMAINS) {
    out[host] = { ...DEFAULT_PER_DOMAIN };
  }
  return out;
}

export const DEFAULT_SETTINGS_V1: SettingsV1 = {
  version: 1,
  storage_tier: "local",
  per_domain: defaultPerDomain(),
  telemetry: { enabled: false },
};

/** Defensive read: missing key OR malformed payload both yield defaults.
 *  Never throws — the options page must always render. */
export async function loadSettings(): Promise<SettingsV1> {
  let raw: unknown;
  try {
    const stored = (await chrome.storage.local.get([
      SETTINGS_KEY_V1,
    ])) as Record<string, unknown>;
    raw = stored[SETTINGS_KEY_V1];
  } catch {
    return cloneDefaults();
  }
  if (raw === undefined || raw === null) {
    return cloneDefaults();
  }
  return migrateAndValidate(raw);
}

export async function saveSettings(next: SettingsV1): Promise<void> {
  await chrome.storage.local.set({ [SETTINGS_KEY_V1]: next });
}

/** Narrow patch update — merges into the existing row and persists.
 *  Returns the post-write snapshot so callers can re-render. */
export async function updateSettings(
  patch: Partial<Omit<SettingsV1, "version">>,
): Promise<SettingsV1> {
  const current = await loadSettings();
  const next: SettingsV1 = {
    ...current,
    ...patch,
    version: 1,
    // Per-domain merge must be deep (per-host); a shallow spread would
    // wipe other hosts' toggles when the caller patches a single one.
    per_domain: patch.per_domain
      ? { ...current.per_domain, ...patch.per_domain }
      : current.per_domain,
    telemetry: patch.telemetry
      ? { ...current.telemetry, ...patch.telemetry }
      : current.telemetry,
  };
  await saveSettings(next);
  return next;
}

/** Schema migration entry point. Today only v1 exists, so the helper is
 *  effectively a validator. When v2 lands, it routes by `version` and
 *  rewrites the row under `settings.v2`. */
export function migrateAndValidate(raw: unknown): SettingsV1 {
  if (typeof raw !== "object" || raw === null) return cloneDefaults();
  const obj = raw as Record<string, unknown>;
  const version = obj.version;
  if (version !== 1) {
    // Future: branch on version === 2, etc. For now any unknown version
    // is treated as corrupt and falls back to defaults so the options
    // page never blocks on a malformed row.
    return cloneDefaults();
  }
  const tier = obj.storage_tier === "cloud" ? "cloud" : "local";
  const perDomain = sanitizePerDomain(obj.per_domain);
  const telemetry = sanitizeTelemetry(obj.telemetry);
  return {
    version: 1,
    storage_tier: tier,
    per_domain: perDomain,
    telemetry,
  };
}

function sanitizePerDomain(raw: unknown): Record<string, PerDomainSettings> {
  const base = defaultPerDomain();
  if (typeof raw !== "object" || raw === null) return base;
  const src = raw as Record<string, unknown>;
  for (const host of Object.keys(src)) {
    const entry = src[host];
    if (typeof entry !== "object" || entry === null) continue;
    const e = entry as Record<string, unknown>;
    base[host] = {
      enabled: typeof e.enabled === "boolean" ? e.enabled : true,
      fab_visible: typeof e.fab_visible === "boolean" ? e.fab_visible : true,
      // D12: only honour explicit `true`. Anything else (missing,
      // malformed) falls back to false to preserve the privacy default.
      auto_capture: e.auto_capture === true,
    };
  }
  return base;
}

function sanitizeTelemetry(raw: unknown): TelemetrySettings {
  if (typeof raw !== "object" || raw === null) return { enabled: false };
  const e = raw as Record<string, unknown>;
  return { enabled: e.enabled === true };
}

function cloneDefaults(): SettingsV1 {
  return {
    version: 1,
    storage_tier: DEFAULT_SETTINGS_V1.storage_tier,
    per_domain: defaultPerDomain(),
    telemetry: { ...DEFAULT_SETTINGS_V1.telemetry },
  };
}
