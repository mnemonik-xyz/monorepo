// Identity badge — top-left of the popup header. Renders a truncated
// pubkey with a copy-to-clipboard interaction and a tooltip-style full
// DID. Click also opens the options page (T12 owns the Settings UI).
//
// The pubkey is read from `chrome.storage.local` (populated by T16
// onboarding). When no identity is yet present we render a neutral
// "(not signed in)" affordance — the popup is still useful for the
// Verify tab in that state.

import type { JSX } from "react";
import type { PopupIdentity } from "../runtime.js";

export interface IdentityBadgeProps {
  identity: PopupIdentity | null;
  onOpenOptions: () => void;
}

export function IdentityBadge(props: IdentityBadgeProps): JSX.Element {
  const { identity, onOpenOptions } = props;
  if (!identity) {
    return (
      <button
        type="button"
        onClick={onOpenOptions}
        className="text-text-muted hover:text-accent-primary text-xs font-mono px-2 py-1 rounded border border-white/10 transition-colors"
        aria-label="Not signed in — open settings"
        title="No identity configured — click to open Settings"
      >
        (not signed in)
      </button>
    );
  }
  const short = truncateMiddle(identity.pubkey_base58, 4, 4);
  const did = `did:sol:${identity.pubkey_base58}`;
  return (
    <button
      type="button"
      onClick={onOpenOptions}
      title={did}
      aria-label={`Identity ${did}`}
      className="text-accent-primary hover:text-white text-xs font-mono px-2 py-1 rounded border border-accent-primary/40 hover:border-accent-primary transition-colors"
    >
      {short}
    </button>
  );
}

function truncateMiddle(value: string, head: number, tail: number): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}
