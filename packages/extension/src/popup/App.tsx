// Popup root. Owns:
//   - header (IdentityBadge + StorageTierPill + Settings cog)
//   - tab switcher (Capture / Recall / Verify)
//   - one-time bootstrap of identity + active-tab adapter + selection
//
// Identity + storage tier read from `chrome.storage.local`. Heavy
// dependencies (WASM, embedder worker, IndexedDB cosine search) sit
// behind dynamic imports in `runtime-impl.ts` so the popup's first-
// paint bundle stays under the 50KB size-limit budget.

import { useEffect, useState, type JSX } from "react";
import type { ChatAdapter } from "../runtime/chat/types.js";
import { getRuntime, type PopupIdentity, type StorageTier } from "./runtime.js";
import { Capture } from "./tabs/Capture.js";
import { Recall } from "./tabs/Recall.js";
import { Verify } from "./tabs/Verify.js";
import { IdentityBadge } from "./components/IdentityBadge.js";
import { StorageTierPill } from "./components/StorageTierPill.js";

type Tab = "capture" | "recall" | "verify";

const TAB_LABELS: Record<Tab, string> = {
  capture: "Capture",
  recall: "Recall",
  verify: "Verify",
};

export function App(): JSX.Element {
  const [tab, setTab] = useState<Tab>("capture");
  const [identity, setIdentity] = useState<PopupIdentity | null>(null);
  const [tier, setTier] = useState<StorageTier>("local");
  const [adapter, setAdapter] = useState<ChatAdapter | null>(null);
  const [selection, setSelection] = useState("");
  const [tierDialogOpen, setTierDialogOpen] = useState(false);

  useEffect(() => {
    const r = getRuntime();
    let cancelled = false;
    void (async () => {
      const [id, t, ad, sel] = await Promise.all([
        r.loadIdentity(),
        r.loadStorageTier(),
        r.getActiveTabAdapter(),
        r.getActiveTabSelection(),
      ]);
      if (cancelled) return;
      setIdentity(id);
      setTier(t);
      setAdapter(ad);
      setSelection(sel);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const openOptions = (): void => {
    try {
      chrome.runtime.openOptionsPage();
    } catch {
      // openOptionsPage requires a registered options_ui; the popup
      // can do nothing useful if it's missing.
    }
  };

  return (
    <main className="bg-background text-text-primary p-3 flex flex-col gap-3 min-h-full">
      <header className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h1 className="text-sm text-accent-primary font-semibold tracking-wide">
            Mnemonik
          </h1>
          <IdentityBadge identity={identity} onOpenOptions={openOptions} />
        </div>
        <div className="flex items-center gap-2">
          <StorageTierPill
            tier={tier}
            onClick={() => setTierDialogOpen(true)}
          />
          <button
            type="button"
            onClick={openOptions}
            aria-label="Settings"
            title="Open Settings"
            className="text-text-muted hover:text-accent-primary text-xs font-mono px-2 py-1 rounded border border-white/10 transition-colors"
          >
            {/* gear glyph — Unicode keeps the bundle small */}⚙
          </button>
        </div>
      </header>

      <nav
        aria-label="Tab switcher"
        className="flex items-center gap-1 border-b border-white/10"
      >
        {(Object.keys(TAB_LABELS) as Tab[]).map((t) => (
          <button
            key={t}
            type="button"
            aria-current={tab === t ? "page" : undefined}
            onClick={() => setTab(t)}
            className={`flex-1 text-xs font-mono uppercase tracking-wide px-2 py-1.5 border-b-2 transition-colors ${
              tab === t
                ? "border-accent-primary text-accent-primary"
                : "border-transparent text-text-muted hover:text-text-primary"
            }`}
          >
            {TAB_LABELS[t]}
          </button>
        ))}
      </nav>

      <section className="flex-1">
        {tab === "capture" ? (
          <Capture adapter={adapter} prefilledSelection={selection} />
        ) : null}
        {tab === "recall" ? <Recall adapter={adapter} /> : null}
        {tab === "verify" ? <Verify /> : null}
      </section>

      {tierDialogOpen ? (
        <TierDialog onClose={() => setTierDialogOpen(false)} />
      ) : null}
    </main>
  );
}

function TierDialog({ onClose }: { onClose: () => void }): JSX.Element {
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Switching storage tiers"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-xs flex flex-col gap-3">
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Switch storage tier
        </h2>
        <p className="text-xs text-text-muted">
          Switching tiers is handled in Settings. Local → Cloud uploads existing
          memories one by one; Cloud → Local exports and disconnects. Both flows
          live on the Settings page.
        </p>
        <div className="flex items-center gap-2 mt-1">
          <button
            type="button"
            onClick={() => {
              chrome.runtime.openOptionsPage?.();
              onClose();
            }}
            className="flex-1 bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-1.5 rounded transition-colors"
          >
            Open Settings
          </button>
          <button
            type="button"
            onClick={onClose}
            className="flex-1 bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-1.5 rounded transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
