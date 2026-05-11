// Per-domain section. One row per supported AI-chat host (D11). Each
// row exposes three toggles:
//   - enabled       — content script + adapter active on this host
//   - fab_visible   — floating action button injected on this host
//   - auto_capture  — capture every assistant turn (D12: default OFF)
//
// The toggles only enable / disable functionality on already-permitted
// domains — they MUST NOT change `host_permissions` (D11). Toggles
// persist immediately via `settings.update`.

import { useCallback, useEffect, useState, type JSX } from "react";
import { getOptionsRuntime } from "../runtime.js";
import {
  SUPPORTED_DOMAINS,
  type SettingsV1,
  type PerDomainSettings,
} from "../../settings.js";

export function PerDomain(): JSX.Element {
  const [settings, setSettings] = useState<SettingsV1 | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const s = await getOptionsRuntime().settings.load();
      if (!cancelled) setSettings(s);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const onToggle = useCallback(
    async (host: string, field: keyof PerDomainSettings) => {
      if (!settings) return;
      const current: PerDomainSettings = settings.per_domain[host] ?? {
        enabled: true,
        fab_visible: true,
        auto_capture: false,
      };
      const next: PerDomainSettings = {
        ...current,
        [field]: !current[field],
      };
      const updated = await getOptionsRuntime().settings.update({
        per_domain: { [host]: next },
      });
      setSettings(updated);
    },
    [settings],
  );

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Per-domain
        </h2>
        <p className="text-xs text-text-muted mt-1">
          Per-host functionality toggles. Auto-capture is off by default for
          privacy — turn it on per domain to capture every assistant turn
          automatically.
        </p>
      </header>

      <div className="flex flex-col gap-2">
        {SUPPORTED_DOMAINS.map((host) => {
          const row: PerDomainSettings = settings?.per_domain[host] ?? {
            enabled: true,
            fab_visible: true,
            auto_capture: false,
          };
          return (
            <div
              key={host}
              className="border border-white/10 rounded p-3 flex flex-col gap-2"
            >
              <h3 className="text-xs font-mono text-text-primary">{host}</h3>
              <Toggle
                host={host}
                label="Enabled"
                field="enabled"
                value={row.enabled}
                onToggle={onToggle}
                hint="Content script + adapter active on this host."
              />
              <Toggle
                host={host}
                label="FAB visible"
                field="fab_visible"
                value={row.fab_visible}
                onToggle={onToggle}
                hint="Show the floating action button on this host."
              />
              <Toggle
                host={host}
                label="Auto-capture"
                field="auto_capture"
                value={row.auto_capture}
                onToggle={onToggle}
                hint="Sign every new assistant turn automatically. Off by default."
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Toggle({
  host,
  label,
  field,
  value,
  onToggle,
  hint,
}: {
  host: string;
  label: string;
  field: keyof PerDomainSettings;
  value: boolean;
  onToggle: (host: string, field: keyof PerDomainSettings) => void;
  hint?: string;
}): JSX.Element {
  // The accessible name embeds the host so tests + screen-reader users
  // can disambiguate the same toggle across rows.
  const ariaLabel = `${host} ${label}`;
  return (
    <label className="flex items-start justify-between gap-3 text-xs">
      <span className="flex flex-col">
        <span className="text-text-primary font-mono">{label}</span>
        {hint ? (
          <span className="text-[10px] text-text-muted">{hint}</span>
        ) : null}
      </span>
      <input
        type="checkbox"
        checked={value}
        aria-label={ariaLabel}
        onChange={() => onToggle(host, field)}
        className="mt-1 accent-accent-primary"
      />
    </label>
  );
}
