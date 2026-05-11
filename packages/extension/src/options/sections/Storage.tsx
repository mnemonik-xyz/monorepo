// Storage section. Displays the active tier (read-only) and gates the
// switch behind an EXPLICIT confirmation dialog (D7 — no silent dual-
// write). Local → Cloud requires a Google session (T16); without one
// the dialog opens in sign-in mode and routes the user through
// `auth.signIn()` first. Cloud → Local triggers a download of every
// attestation as a COSE bundle.
//
// The migration confirmation lists the row count so the user knows
// exactly what will move.

import { useCallback, useEffect, useRef, useState, type JSX } from "react";
import { getOptionsRuntime } from "../runtime.js";
import type { AuthSession, MigrationProgressEvent } from "../runtime.js";
import { Toast } from "../../popup/components/Toast.js";
import { MigrationProgress } from "../components/MigrationProgress.js";
import type { SettingsV1 } from "../../settings.js";

type Mode =
  | { kind: "closed" }
  | { kind: "signin" }
  | { kind: "confirm-to-cloud"; rows: number }
  | { kind: "confirm-to-local"; rows: number }
  | {
      kind: "migrating-to-cloud";
      progress: MigrationProgressEvent | null;
    };

interface ToastState {
  message: string;
  kind: "success" | "error" | "info";
  nonce: number;
}

export function Storage(): JSX.Element {
  const [tier, setTier] = useState<SettingsV1["storage_tier"]>("local");
  const [session, setSession] = useState<AuthSession | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "closed" });
  const [toast, setToast] = useState<ToastState | null>(null);
  const unsubscribeRef = useRef<(() => void) | null>(null);

  const showToast = useCallback((message: string, kind: ToastState["kind"]) => {
    setToast({ message, kind, nonce: Date.now() });
  }, []);

  // Initial load — settings + cached session.
  useEffect(() => {
    let cancelled = false;
    const r = getOptionsRuntime();
    void (async () => {
      const [s, sess] = await Promise.all([
        r.settings.load(),
        r.auth.getSession(),
      ]);
      if (cancelled) return;
      setTier(s.storage_tier);
      setSession(sess);
    })();
    return () => {
      cancelled = true;
      unsubscribeRef.current?.();
    };
  }, []);

  const onSwitch = useCallback(async () => {
    const r = getOptionsRuntime();
    if (tier === "local") {
      // Local → Cloud requires a Google session; gate first.
      if (!session) {
        setMode({ kind: "signin" });
        return;
      }
      const rows = await r.cloudSync.countLocalAttestations();
      setMode({ kind: "confirm-to-cloud", rows });
      return;
    }
    // Cloud → Local: confirm the export-then-disconnect path.
    const rows = await r.cloudSync.countLocalAttestations();
    setMode({ kind: "confirm-to-local", rows });
  }, [tier, session]);

  const onSignIn = useCallback(async () => {
    const r = getOptionsRuntime();
    try {
      const fresh = await r.auth.signIn();
      setSession(fresh);
      const rows = await r.cloudSync.countLocalAttestations();
      setMode({ kind: "confirm-to-cloud", rows });
    } catch (err) {
      showToast(
        `Sign-in failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
      setMode({ kind: "closed" });
    }
  }, [showToast]);

  const onConfirmToCloud = useCallback(async () => {
    const r = getOptionsRuntime();
    setMode({ kind: "migrating-to-cloud", progress: null });
    try {
      await r.cloudSync.enqueueAll();
      const next = await r.settings.update({ storage_tier: "cloud" });
      setTier(next.storage_tier);
      const unsub = r.cloudSync.subscribeProgress((e) => {
        setMode({ kind: "migrating-to-cloud", progress: e });
        if (e.done) {
          unsub();
          unsubscribeRef.current = null;
          setMode({ kind: "closed" });
          showToast("Migrated to Cloud tier", "success");
        }
      });
      unsubscribeRef.current = unsub;
    } catch (err) {
      showToast(
        `Migration failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
      setMode({ kind: "closed" });
    }
  }, [showToast]);

  const onConfirmToLocal = useCallback(async () => {
    const r = getOptionsRuntime();
    try {
      const blob = await r.cloudSync.exportAll();
      triggerDownload(blob, "mnemonik-export.zip");
      const next = await r.settings.update({ storage_tier: "local" });
      setTier(next.storage_tier);
      await r.auth.clearSession();
      setSession(null);
      setMode({ kind: "closed" });
      showToast("Switched to Local tier — cloud session cleared", "success");
    } catch (err) {
      showToast(
        `Switch failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
      setMode({ kind: "closed" });
    }
  }, [showToast]);

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Storage
        </h2>
        <p className="text-xs text-text-muted mt-1">
          Local stores attestations only in this browser. Cloud syncs to managed
          Mnemonik infrastructure for cross-device recall.
        </p>
      </header>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <div>
            <div className="text-[10px] uppercase tracking-wide text-text-muted font-mono">
              Active tier
            </div>
            <div className="text-sm font-mono mt-0.5">
              {tier === "cloud" ? "Cloud" : "Local"}
            </div>
          </div>
          <button
            type="button"
            onClick={onSwitch}
            className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded transition-colors"
          >
            {tier === "local" ? "Switch to Cloud" : "Switch to Local"}
          </button>
        </div>
        {session ? (
          <div className="text-[11px] text-text-muted font-mono">
            Signed in as {session.email}
          </div>
        ) : null}
      </div>

      {mode.kind === "signin" ? (
        <SignInDialog
          onSignIn={onSignIn}
          onCancel={() => setMode({ kind: "closed" })}
        />
      ) : null}

      {mode.kind === "confirm-to-cloud" ? (
        <ConfirmToCloudDialog
          rows={mode.rows}
          onConfirm={onConfirmToCloud}
          onCancel={() => setMode({ kind: "closed" })}
        />
      ) : null}

      {mode.kind === "confirm-to-local" ? (
        <ConfirmToLocalDialog
          rows={mode.rows}
          onConfirm={onConfirmToLocal}
          onCancel={() => setMode({ kind: "closed" })}
        />
      ) : null}

      {mode.kind === "migrating-to-cloud" ? (
        <MigrationProgress progress={mode.progress} />
      ) : null}

      {toast ? (
        <Toast
          key={toast.nonce}
          message={toast.message}
          kind={toast.kind}
          onDismiss={() => setToast(null)}
        />
      ) : null}
    </div>
  );
}

function SignInDialog({
  onSignIn,
  onCancel,
}: {
  onSignIn: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="storage-signin-title"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4 z-10"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-sm flex flex-col gap-3">
        <h3
          id="storage-signin-title"
          className="text-sm font-mono uppercase tracking-wide text-accent-primary"
        >
          Sign in with Google
        </h3>
        <p className="text-xs text-text-muted">
          Cloud tier requires a Google account so attestations are bound to a
          stable identity for cross-device restore. The server stores only an
          encrypted blob — your passphrase never leaves this browser.
        </p>
        <div className="flex items-center gap-2 mt-1">
          <button
            type="button"
            onClick={onSignIn}
            className="flex-1 bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Sign in with Google
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="flex-1 bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function ConfirmToCloudDialog({
  rows,
  onConfirm,
  onCancel,
}: {
  rows: number;
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="storage-to-cloud-title"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4 z-10"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-sm flex flex-col gap-3">
        <h3
          id="storage-to-cloud-title"
          className="text-sm font-mono uppercase tracking-wide text-accent-primary"
        >
          Switch to Cloud
        </h3>
        <p className="text-xs text-text-muted">
          This will upload {rows} existing attestation
          {rows === 1 ? "" : "s"} to the managed Mnemonik server. New
          attestations will be anchored on Arweave + Solana. The flow is
          resumable — closing this page won&apos;t cancel uploads.
        </p>
        <div className="flex items-center gap-2 mt-1">
          <button
            type="button"
            onClick={onConfirm}
            className="flex-1 bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Confirm
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="flex-1 bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function ConfirmToLocalDialog({
  rows,
  onConfirm,
  onCancel,
}: {
  rows: number;
  onConfirm: () => void;
  onCancel: () => void;
}): JSX.Element {
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="storage-to-local-title"
      className="fixed inset-0 bg-black/70 flex items-center justify-center p-4 z-10"
    >
      <div className="bg-background border border-white/10 rounded p-4 max-w-sm flex flex-col gap-3">
        <h3
          id="storage-to-local-title"
          className="text-sm font-mono uppercase tracking-wide text-accent-primary"
        >
          Switch to Local
        </h3>
        <p className="text-xs text-text-muted">
          This will export all {rows} cloud attestation
          {rows === 1 ? "" : "s"} as a COSE bundle (.zip) and disconnect your
          cloud session. Future captures stay in this browser only.
        </p>
        <div className="flex items-center gap-2 mt-1">
          <button
            type="button"
            onClick={onConfirm}
            className="flex-1 bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Confirm
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="flex-1 bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide py-1.5 rounded"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function triggerDownload(bytes: Uint8Array, filename: string): void {
  const blob = new Blob([bytes], { type: "application/zip" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
