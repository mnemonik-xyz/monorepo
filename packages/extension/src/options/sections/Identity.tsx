// Identity section. Shows the active pubkey + DID, offers a click-to-
// copy on each, and surfaces "Export keypair" / "Import keypair"
// (passphrase-encrypted JSON blob). The QR code is a placeholder until
// a small QR module lands; the pubkey copy + DID copy cover the
// Phase-1 sharing UX. Export passphrase must clear the same zxcvbn
// strength gate as Security/rotate (length >= 12 AND score >= 3).

import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useRef,
  useState,
  type JSX,
} from "react";
import { getOptionsRuntime, type IdentitySnapshot } from "../runtime.js";
import { Toast } from "../../popup/components/Toast.js";
import type { ToastState } from "../types.js";
import { triggerDownload } from "../utils/download.js";
import {
  MIN_PASSPHRASE_LENGTH,
  isPassphraseAcceptable,
} from "../components/PassphraseStrength.js";

const PassphraseStrength = lazy(
  () => import("../components/PassphraseStrength.js"),
);

export function Identity(): JSX.Element {
  const [identity, setIdentity] = useState<IdentitySnapshot | null>(null);
  const [exportPassphrase, setExportPassphrase] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [toast, setToast] = useState<ToastState | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const showToast = useCallback((message: string, kind: ToastState["kind"]) => {
    setToast({ message, kind, nonce: Date.now() });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const r = getOptionsRuntime();
    void (async () => {
      const id = await r.identity.load();
      if (!cancelled) setIdentity(id);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const onCopy = useCallback(
    async (value: string, label: string) => {
      try {
        await navigator.clipboard?.writeText(value);
        showToast(`${label} copied`, "success");
      } catch {
        showToast(`Copy ${label} failed`, "error");
      }
    },
    [showToast],
  );

  const onExport = useCallback(async () => {
    const pp = exportPassphrase.trim();
    if (!pp || pp.length < MIN_PASSPHRASE_LENGTH) {
      showToast(
        `Export passphrase must be at least ${MIN_PASSPHRASE_LENGTH} characters.`,
        "error",
      );
      return;
    }
    if (!isPassphraseAcceptable(pp)) {
      showToast(
        "Export passphrase is too weak — aim for a Strong rating.",
        "error",
      );
      return;
    }
    try {
      const blob = await getOptionsRuntime().identity.exportEncrypted(pp);
      triggerDownload(blob, "mnemonik-keypair.enc.json", "application/json");
      setExportPassphrase("");
      showToast("Encrypted keypair downloaded.", "success");
    } catch (err) {
      showToast(
        `Export failed: ${err instanceof Error ? err.message : String(err)}`,
        "error",
      );
    }
  }, [exportPassphrase, showToast]);

  const onImportFile = useCallback(
    async (file: File) => {
      const pp = importPassphrase.trim();
      if (!pp) {
        showToast("Import passphrase is required.", "error");
        return;
      }
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        const id = await getOptionsRuntime().identity.importEncrypted(
          bytes,
          pp,
        );
        setIdentity(id);
        setImportPassphrase("");
        showToast("Keypair imported.", "success");
      } catch (err) {
        showToast(
          `Import failed: ${err instanceof Error ? err.message : String(err)}`,
          "error",
        );
      }
    },
    [importPassphrase, showToast],
  );

  const exportAcceptable = isPassphraseAcceptable(exportPassphrase);
  const exportDisabled = !identity || !exportAcceptable;

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          Identity
        </h2>
        <p className="text-xs text-text-muted mt-1">
          Your Ed25519 keypair signs every attestation. Export the keypair as an
          encrypted file to back it up or move to another browser.
        </p>
      </header>

      {identity ? (
        <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
          <Row
            label="Public key"
            value={identity.pubkey_base58}
            onCopy={() => onCopy(identity.pubkey_base58, "Public key")}
          />
          <Row
            label="DID"
            value={identity.did}
            onCopy={() => onCopy(identity.did, "DID")}
          />
        </div>
      ) : (
        <div className="border border-white/10 rounded p-3">
          <p className="text-xs text-text-muted">
            No agent identity found. Generate or import a keypair via the popup.
          </p>
        </div>
      )}

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Export keypair
        </h3>
        <p className="text-[11px] text-text-muted">
          Encrypts the keypair with your passphrase (AES-GCM-256 + Argon2id) and
          downloads it as a JSON file. Keep this file in a safe place — anyone
          with the file AND the passphrase can sign attestations as you.
        </p>
        <label className="flex flex-col gap-1 text-[11px] text-text-muted font-mono">
          Export passphrase
          <input
            type="password"
            autoComplete="new-password"
            value={exportPassphrase}
            onChange={(e) => setExportPassphrase(e.target.value)}
            className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
          />
        </label>
        <Suspense
          fallback={
            <div className="text-[10px] text-text-muted font-mono">
              Loading strength meter…
            </div>
          }
        >
          <PassphraseStrength value={exportPassphrase} />
        </Suspense>
        <button
          type="button"
          onClick={onExport}
          disabled={exportDisabled}
          className="self-start bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
        >
          Export keypair
        </button>
      </div>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Import keypair
        </h3>
        <p className="text-[11px] text-text-muted">
          Restore a previously-exported keypair. Replaces the current identity
          in this browser.
        </p>
        <label className="flex flex-col gap-1 text-[11px] text-text-muted font-mono">
          Import passphrase
          <input
            type="password"
            autoComplete="current-password"
            value={importPassphrase}
            onChange={(e) => setImportPassphrase(e.target.value)}
            className="bg-black/30 border border-white/10 rounded px-2 py-1 text-text-primary text-xs font-mono"
          />
        </label>
        <input
          ref={fileInputRef}
          type="file"
          accept="application/json"
          aria-label="Encrypted keypair file"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) void onImportFile(f);
            if (fileInputRef.current) fileInputRef.current.value = "";
          }}
          className="text-xs text-text-muted file:mr-2 file:bg-white/5 file:hover:bg-white/10 file:text-text-primary file:border file:border-white/10 file:text-xs file:font-mono file:uppercase file:tracking-wide file:px-2 file:py-1 file:rounded"
        />
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

function Row({
  label,
  value,
  onCopy,
}: {
  label: string;
  value: string;
  onCopy: () => void;
}): JSX.Element {
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="flex flex-col min-w-0">
        <span className="text-[10px] uppercase tracking-wide text-text-muted font-mono">
          {label}
        </span>
        <span className="text-xs font-mono break-all text-text-primary">
          {value}
        </span>
      </div>
      <button
        type="button"
        onClick={onCopy}
        aria-label={`Copy ${label}`}
        className="shrink-0 text-[10px] uppercase tracking-wide font-mono px-2 py-1 rounded border border-white/20 text-text-muted hover:text-accent-primary hover:border-accent-primary"
      >
        Copy
      </button>
    </div>
  );
}
