// Storage-tier pill — reads `storage_tier` out of `chrome.storage.local`
// and renders "Local" (gray) or "Cloud" (teal). T11 ships this as a
// READ-ONLY indicator: clicking opens a stub dialog that points the
// user to Settings (T12 owns the actual tier-switch logic per the
// tech-spec wave 4 split). The pill is still clickable here so users
// have a clear path to the place that does the switch.

import type { JSX } from "react";
import type { StorageTier } from "../runtime.js";

export interface StorageTierPillProps {
  tier: StorageTier;
  onClick: () => void;
}

const LABEL: Record<StorageTier, string> = {
  local: "Local",
  cloud: "Cloud",
};

export function StorageTierPill(props: StorageTierPillProps): JSX.Element {
  const { tier, onClick } = props;
  const classes =
    tier === "cloud"
      ? "bg-accent-primary/15 text-accent-primary border-accent-primary/40"
      : "bg-white/5 text-text-muted border-white/10";
  return (
    <button
      type="button"
      onClick={onClick}
      title={
        tier === "cloud"
          ? "Cloud tier — synced to Mnemonik infra. Click to manage."
          : "Local tier — stored only in this browser. Click to manage."
      }
      aria-label={`Storage tier: ${LABEL[tier]}`}
      className={`text-[10px] uppercase tracking-wide font-mono px-2 py-1 rounded-full border transition-colors hover:brightness-125 ${classes}`}
    >
      {LABEL[tier]}
    </button>
  );
}
