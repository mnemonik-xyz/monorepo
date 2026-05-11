// Telemetry section. Off by default. The opt-in toggle ships only the
// counters listed in the tech-spec — broken-adapter / cold-start /
// sync-error — with no payload and no PII. The actual emit pipeline is
// owned by the SW; this section just gates the boolean.

import { useCallback, useEffect, useState, type JSX } from "react";
import { getOptionsRuntime } from "../runtime.js";
import type { SettingsV1 } from "../../settings.js";

export function Telemetry(): JSX.Element {
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

  const onToggle = useCallback(async () => {
    if (!settings) return;
    const updated = await getOptionsRuntime().settings.update({
      telemetry: { enabled: !settings.telemetry.enabled },
    });
    setSettings(updated);
  }, [settings]);

  const enabled = settings?.telemetry.enabled ?? false;

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Telemetry
        </h2>
        <p className="text-xs text-text-muted mt-1">
          Off by default. When enabled, Mnemonik reports anonymous counters:
          broken-adapter detections, cold-start latency, and sync errors. No
          content, URLs, or identifiers are sent.
        </p>
      </header>

      <div className="border border-white/10 rounded p-3 flex items-start justify-between gap-3">
        <div className="flex flex-col">
          <span className="text-xs text-text-primary font-mono">
            Enable telemetry
          </span>
          <span className="text-[10px] text-text-muted">
            Help improve adapter coverage. You can turn this off anytime.
          </span>
        </div>
        <input
          type="checkbox"
          aria-label="Enable telemetry"
          checked={enabled}
          onChange={onToggle}
          className="mt-1 accent-accent-primary"
        />
      </div>
    </div>
  );
}
