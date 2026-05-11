// Security section. Cloud-only. Three flows:
//   1. Rotate recovery passphrase (re-derives Argon2id wrap-key,
//      re-encrypts the Ed25519 secret, PUTs to /api/key-escrow). Calls
//      `keyEscrow.rotate(old, new)` — TDD anchor binds this contract.
//   2. Delete cloud escrow (advanced; warns + double-confirm).
//   3. Sign out of Google (clears session via `auth.clearSession()`).
//
// The page renders a Local-tier banner instead when no Google session
// exists — none of these flows are meaningful without escrow access.

import {
  useCallback,
  useEffect,
  useState,
  type FormEvent,
  type JSX,
} from "react";
import { getOptionsRuntime, type AuthSession } from "../runtime.js";
import { Toast } from "../../popup/components/Toast.js";

interface ToastState {
  message: string;
  kind: "success" | "error" | "info";
  nonce: number;
}

export function Security(): JSX.Element {
  const [session, setSession] = useState<AuthSession | null>(null);
  const [hasBlob, setHasBlob] = useState(false);
  const [oldPp, setOldPp] = useState("");
  const [newPp, setNewPp] = useState("");
  const [toast, setToast] = useState<ToastState | null>(null);
  const [rotating, setRotating] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const showToast = useCallback((message: string, kind: ToastState["kind"]) => {
    setToast({ message, kind, nonce: Date.now() });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const r = getOptionsRuntime();
    void (async () => {
      const [sess, blob] = await Promise.all([
        r.auth.getSession(),
        r.keyEscrow.hasBlob(),
      ]);
      if (cancelled) return;
      setSession(sess);
      setHasBlob(blob);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const onRotate = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      const trimmedOld = oldPp.trim();
      const trimmedNew = newPp.trim();
      if (!trimmedOld || !trimmedNew) {
        showToast("Both current and new passphrase are required.", "error");
        return;
      }
      if (trimmedNew.length < 10) {
        showToast("New passphrase must be at least 10 characters.", "error");
        return;
      }
      setRotating(true);
      try {
        await getOptionsRuntime().keyEscrow.rotate(trimmedOld, trimmedNew);
        setOldPp("");
        setNewPp("");
        showToast("Passphrase rotated and re-uploaded.", "success");
      } catch (err) {
        showToast(
          `Rotate failed: ${err instanceof Error ? err.message : String(err)}`,
          "error",
        );
      } finally {
        setRotating(false);
      }
    },
    [oldPp, newPp, showToast],
  );

  const onDelete = useCallback(async () => {
    try {
      await getOptionsRuntime().keyEscrow.delete();
      setHasBlob(false);
      setConfirmDelete(false);
      showToast("Cloud escrow deleted.", "success");
    } catch (err) {
      showToast(
        `Delete failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
    }
  }, [showToast]);

  const onSignOut = useCallback(async () => {
    try {
      await getOptionsRuntime().auth.clearSession();
      setSession(null);
      showToast("Signed out of Google.", "success");
    } catch (err) {
      showToast(
        `Sign-out failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
    }
  }, [showToast]);

  if (!session) {
    return (
      <div className="flex flex-col gap-4">
        <header>
          <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
            Security
          </h2>
          <p className="text-xs text-text-muted mt-1">
            Cloud tier required — passphrase rotation, escrow management, and
            Google sign-out only apply when a cloud session is active.
          </p>
        </header>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Security
        </h2>
        <p className="text-xs text-text-muted mt-1">
          Manage your recovery passphrase, the encrypted server-side escrow
          blob, and your Google session.
        </p>
      </header>

      <form
        onSubmit={onRotate}
        className="border border-white/10 rounded p-3 flex flex-col gap-2"
      >
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Rotate recovery passphrase
        </h3>
        <label className="flex flex-col gap-1 text-[11px] text-text-muted font-mono">
          Current passphrase
          <input
            type="password"
            autoComplete="current-password"
            value={oldPp}
            onChange={(e) => setOldPp(e.target.value)}
            className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
          />
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-text-muted font-mono">
          New passphrase
          <input
            type="password"
            autoComplete="new-password"
            value={newPp}
            onChange={(e) => setNewPp(e.target.value)}
            className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
          />
        </label>
        <button
          type="submit"
          disabled={rotating}
          className="self-start bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
        >
          {rotating ? "Rotating…" : "Rotate passphrase"}
        </button>
      </form>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Delete cloud escrow
        </h3>
        <p className="text-[11px] text-text-muted">
          Advanced — removes the encrypted blob from the server. New devices
          will not be able to restore until you re-enrol with a fresh
          passphrase. {hasBlob ? "" : "(no blob currently stored)"}
        </p>
        {confirmDelete ? (
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onDelete}
              className="bg-error/20 hover:bg-error/30 text-error border border-error/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded"
            >
              Confirm delete
            </button>
            <button
              type="button"
              onClick={() => setConfirmDelete(false)}
              className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded"
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            type="button"
            disabled={!hasBlob}
            onClick={() => setConfirmDelete(true)}
            className="self-start bg-white/5 hover:bg-white/10 text-text-muted border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
          >
            Delete cloud escrow
          </button>
        )}
      </div>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Google session
        </h3>
        <p className="text-[11px] text-text-muted">
          Signed in as {session.email}. Sign-out clears the cached JWT; your
          local keypair remains untouched.
        </p>
        <button
          type="button"
          onClick={onSignOut}
          className="self-start bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded"
        >
          Sign out of Google
        </button>
      </div>

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
