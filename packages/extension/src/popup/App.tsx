// Popup root. Owns:
//   - header (IdentityBadge + StorageTierPill + Settings cog)
//   - tab switcher (Capture / Recall / Verify)
//   - one-time bootstrap of identity + active-tab adapter + selection
//
// Identity + storage tier read from `chrome.storage.local`. Heavy
// dependencies (WASM, embedder worker, IndexedDB cosine search) sit
// behind dynamic imports in `runtime-impl.ts` so the popup's first-
// paint bundle stays under the 50KB size-limit budget.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type JSX,
  type KeyboardEvent,
} from "react";
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

const TAB_ORDER: Tab[] = ["capture", "recall", "verify"];

export function App(): JSX.Element {
  const [tab, setTab] = useState<Tab>("capture");
  const [identity, setIdentity] = useState<PopupIdentity | null>(null);
  const [tier, setTier] = useState<StorageTier>("local");
  const [adapter, setAdapter] = useState<ChatAdapter | null>(null);
  const [selection, setSelection] = useState("");
  const [tierDialogOpen, setTierDialogOpen] = useState(false);
  const tabRefs = useRef<Partial<Record<Tab, HTMLButtonElement | null>>>({});

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

  // APG tabs pattern: left/right arrows cycle through tabs and move
  // focus to the newly-selected tab. Home/End jump to first/last.
  const handleTabKeyDown = (
    e: KeyboardEvent<HTMLButtonElement>,
    current: Tab
  ): void => {
    const idx = TAB_ORDER.indexOf(current);
    let next: Tab | null = null;
    if (e.key === "ArrowRight") {
      next = TAB_ORDER[(idx + 1) % TAB_ORDER.length] ?? null;
    } else if (e.key === "ArrowLeft") {
      next = TAB_ORDER[(idx - 1 + TAB_ORDER.length) % TAB_ORDER.length] ?? null;
    } else if (e.key === "Home") {
      next = TAB_ORDER[0] ?? null;
    } else if (e.key === "End") {
      next = TAB_ORDER[TAB_ORDER.length - 1] ?? null;
    }
    if (next) {
      e.preventDefault();
      setTab(next);
      tabRefs.current[next]?.focus();
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
            className="text-text-muted hover:text-accent-primary text-xs font-mono px-2 py-1 rounded border border-white/10 transition-colors"
          >
            {/* Unicode gear glyph — aria-hidden so the aria-label is
                the single source of the accessible name. */}
            <span aria-hidden="true">⚙</span>
          </button>
        </div>
      </header>

      <div
        role="tablist"
        aria-label="Mnemonik popup tabs"
        className="flex items-center gap-1 border-b border-white/10"
      >
        {TAB_ORDER.map((t) => {
          const selected = tab === t;
          return (
            <button
              key={t}
              ref={(el) => {
                tabRefs.current[t] = el;
              }}
              type="button"
              role="tab"
              id={`tab-${t}`}
              aria-selected={selected}
              aria-controls={`panel-${t}`}
              tabIndex={selected ? 0 : -1}
              onClick={() => setTab(t)}
              onKeyDown={(e) => handleTabKeyDown(e, t)}
              className={`flex-1 text-xs font-mono uppercase tracking-wide px-2 py-1.5 border-b-2 transition-colors ${
                selected
                  ? "border-accent-primary text-accent-primary"
                  : "border-transparent text-text-muted hover:text-text-primary"
              }`}
            >
              {TAB_LABELS[t]}
            </button>
          );
        })}
      </div>

      <section className="flex-1">
        <div
          role="tabpanel"
          id="panel-capture"
          aria-labelledby="tab-capture"
          tabIndex={0}
          hidden={tab !== "capture"}
        >
          {tab === "capture" ? (
            <Capture adapter={adapter} prefilledSelection={selection} />
          ) : null}
        </div>
        <div
          role="tabpanel"
          id="panel-recall"
          aria-labelledby="tab-recall"
          tabIndex={0}
          hidden={tab !== "recall"}
        >
          {tab === "recall" ? <Recall adapter={adapter} /> : null}
        </div>
        <div
          role="tabpanel"
          id="panel-verify"
          aria-labelledby="tab-verify"
          tabIndex={0}
          hidden={tab !== "verify"}
        >
          {tab === "verify" ? <Verify /> : null}
        </div>
      </section>

      {tierDialogOpen ? (
        <TierDialog onClose={() => setTierDialogOpen(false)} />
      ) : null}
    </main>
  );
}

function TierDialog({ onClose }: { onClose: () => void }): JSX.Element {
  const dialogRef = useRef<HTMLDivElement>(null);
  const firstButtonRef = useRef<HTMLButtonElement>(null);

  // Escape-to-dismiss + initial focus on the primary action. A full
  // focus-trap implementation can land with the dedicated dialog
  // primitive in T12; the keyboard-driven close path is the most
  // user-visible piece and ships here.
  const handleKeyDown = useCallback(
    (e: globalThis.KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    },
    [onClose]
  );

  useEffect(() => {
    firstButtonRef.current?.focus();
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  return (
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="tier-dialog-title"
      aria-describedby="tier-dialog-desc"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-xs flex flex-col gap-3">
        <h2
          id="tier-dialog-title"
          className="text-sm font-mono uppercase tracking-wide text-accent-primary"
        >
          Switch storage tier
        </h2>
        <p id="tier-dialog-desc" className="text-xs text-text-muted">
          Switching tiers is handled in Settings. Local → Cloud uploads existing
          memories one by one; Cloud → Local exports and disconnects. Both flows
          live on the Settings page.
        </p>
        <div className="flex items-center gap-2 mt-1">
          <button
            ref={firstButtonRef}
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
