// Identity section. Shows the active pubkey + DID, offers a click-to-
// copy on each, and surfaces the T25 identity-management surface:
//
//   - Generate identity (only when no identity is configured) /
//     Re-generate (with a destructive-confirm dialog) when one exists.
//   - Export keypair (UNENCRYPTED JSON, with a non-removable warning).
//   - Import keypair (UNENCRYPTED JSON; replace-confirm when an
//     identity already lives in chrome.storage.local).
//
// The pre-T25 "Export encrypted" / "Import encrypted" controls are kept
// below for the passphrase-protected backup flow (T17/T18 follow-up):
// they round-trip the same on-disk shape via the WASM `export_keypair_
// json` export and the existing key-escrow Argon2id wrap. Surfacing
// both side-by-side lets the user pick the right tool — the plain
// export prints a loud warning, the encrypted export prints the
// passphrase requirement.

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

/** Confirmation hook surface. Defaults to `window.confirm`; tests
 *  inject a deterministic stub so the destructive paths (regenerate,
 *  overwrite-on-import) are observable without driving a real dialog. */
type ConfirmFn = (message: string) => boolean;

const DEFAULT_CONFIRM: ConfirmFn = (msg) => {
  try {
    return window.confirm(msg);
  } catch {
    // jsdom (or a non-DOM environment) → behave as if the user cancelled.
    return false;
  }
};

export interface IdentityProps {
  /** Test-only seam. Production omits this and we fall back to
   *  `window.confirm`. The Onboarding-orchestrator tests pass a stub
   *  to drive both the accept and the cancel paths. */
  confirm?: ConfirmFn;
}

export function Identity(props: IdentityProps = {}): JSX.Element {
  const confirm = props.confirm ?? DEFAULT_CONFIRM;
  const [identity, setIdentity] = useState<IdentitySnapshot | null>(null);
  const [exportPassphrase, setExportPassphrase] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [toast, setToast] = useState<ToastState | null>(null);
  const encryptedFileInputRef = useRef<HTMLInputElement>(null);
  const plainFileInputRef = useRef<HTMLInputElement>(null);

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

  // ── T25: Generate (and re-generate) ────────────────────────────────────
  const onGenerate = useCallback(async () => {
    const r = getOptionsRuntime();
    try {
      if (identity) {
        const ok = confirm(
          `Replacing existing identity ${shortPubkey(identity.pubkey_base58)}. The old keypair will be lost forever — your existing memories will become unverifiable on this device. Continue?`,
        );
        if (!ok) return;
        await r.identity.clear();
      }
      const id = await r.identity.generate();
      setIdentity(id);
      showToast("New identity generated.", "success");
    } catch (err) {
      // Never include the secret in the error path.
      showToast(
        `Generate failed: ${err instanceof Error ? err.message : "unknown error"}`,
        "error",
      );
    }
  }, [identity, confirm, showToast]);

  // ── T25: Plain export ─────────────────────────────────────────────────
  const onExportPlain = useCallback(async () => {
    if (!identity) {
      showToast("No identity to export.", "error");
      return;
    }
    try {
      const payload = await getOptionsRuntime().identity.exportPlain();
      const json = JSON.stringify(payload, null, 2);
      const bytes = new TextEncoder().encode(json);
      const filename = `mnemonik-keypair-${shortPubkey(payload.pubkey_base58)}.json`;
      triggerDownload(bytes, filename, "application/json");
      showToast(
        "Keypair downloaded. Store in your password manager.",
        "success",
      );
    } catch (err) {
      showToast(
        `Export failed: ${err instanceof Error ? err.message : "unknown error"}`,
        "error",
      );
    }
  }, [identity, showToast]);

  // ── T25: Plain import ─────────────────────────────────────────────────
  const onImportPlainFile = useCallback(
    async (file: File) => {
      try {
        const text = await file.text();
        let parsed: unknown;
        try {
          parsed = JSON.parse(text);
        } catch {
          showToast("Wrong file format: not valid JSON.", "error");
          return;
        }
        // Replace-confirm when an identity is already live in this
        // browser. Done BEFORE the runtime call so we don't shred the
        // existing secret on user-cancel.
        if (identity) {
          const ok = confirm(
            `Replacing existing identity ${shortPubkey(identity.pubkey_base58)}. New memories will be signed with the imported key. Continue?`,
          );
          if (!ok) return;
        }
        const id = await getOptionsRuntime().identity.importPlain(parsed);
        setIdentity(id);
        showToast("Keypair imported.", "success");
      } catch (err) {
        showToast(
          `Import failed: ${err instanceof Error ? err.message : "unknown error"}`,
          "error",
        );
      }
    },
    [identity, confirm, showToast],
  );

  // ── Existing: encrypted export ────────────────────────────────────────
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

  // ── Existing: encrypted import ────────────────────────────────────────
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
          Your Ed25519 keypair signs every attestation. Generate one if you have
          none, export it for backup, or import a previous export.
        </p>
      </header>

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void onGenerate()}
          className="bg-accent-primary/20 hover:bg-accent-primary/30 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded transition-colors"
        >
          {identity ? "Re-generate identity" : "Generate identity"}
        </button>
        <button
          type="button"
          onClick={() => void onExportPlain()}
          disabled={!identity}
          title={identity ? undefined : "No identity to export."}
          className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded disabled:opacity-50"
        >
          Export
        </button>
        <button
          type="button"
          onClick={() => plainFileInputRef.current?.click()}
          className="bg-white/5 hover:bg-white/10 text-text-primary border border-white/10 text-xs font-mono uppercase tracking-wide px-3 py-1.5 rounded"
        >
          Import
        </button>
        <input
          ref={plainFileInputRef}
          type="file"
          accept="application/json,.json"
          aria-label="Plain keypair file"
          className="hidden"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) void onImportPlainFile(f);
            if (plainFileInputRef.current) plainFileInputRef.current.value = "";
          }}
        />
      </div>

      {identity ? (
        <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
          <p className="text-[11px] text-text-muted font-mono">
            Active identity:{" "}
            <span className="text-accent-primary">
              {shortPubkey(identity.pubkey_base58)}
            </span>{" "}
            ·{" "}
            <span>
              Created{" "}
              {identity.created_at !== undefined
                ? new Date(identity.created_at).toLocaleString()
                : "—"}
            </span>
          </p>
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
            No agent identity found. Click <em>Generate identity</em> above to
            mint a fresh Ed25519 keypair, or sign in via the popup to let
            onboarding generate one for you.
          </p>
        </div>
      )}

      <div className="border border-yellow-500/30 bg-yellow-500/5 rounded p-3 flex flex-col gap-1">
        <h3 className="text-xs font-mono uppercase tracking-wide text-yellow-300">
          Plain export warning
        </h3>
        <p className="text-[11px] text-text-muted">
          The <em>Export</em> button downloads your keypair in CLEARTEXT. Never
          share the file — anyone with it controls your identity. Store it in a
          password manager.
        </p>
      </div>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2">
        <h3 className="text-xs font-mono uppercase tracking-wide text-text-primary">
          Export keypair (encrypted)
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
          Import keypair (encrypted)
        </h3>
        <p className="text-[11px] text-text-muted">
          Restore a previously-exported encrypted keypair. Replaces the current
          identity in this browser.
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
          ref={encryptedFileInputRef}
          type="file"
          accept="application/json"
          aria-label="Encrypted keypair file"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) void onImportFile(f);
            if (encryptedFileInputRef.current)
              encryptedFileInputRef.current.value = "";
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

/** Truncate a base58 pubkey for headlines and filenames. Mirrors the
 *  popup's `IdentityBadge.truncateMiddle`. */
function shortPubkey(pub: string, head = 6, tail = 4): string {
  if (pub.length <= head + tail + 1) return pub;
  return `${pub.slice(0, head)}…${pub.slice(-tail)}`;
}
