import { useEffect, useRef, useState } from "react";
import {
  clearIdentity,
  readIdentity,
  writeIdentity,
  type KeypairJson,
} from "../lib/storage";
import { loadWasm } from "../lib/wasm";

/**
 * Identity panel — renders the active DID/pubkey and exposes Generate / Import
 * / Export actions. Backed entirely by the WASM module from `webapp/src/wasm/`.
 *
 * The DID format is `did:sol:<base58_pubkey>` per Decision 4.
 *
 * Storage caveat: the keypair is stored as plain JSON in localStorage today. The
 * Risks table (XSS row) calls for AES-GCM encryption with a passphrase-derived
 * key as a hardening step. Phase 1 ships the unencrypted form gated by the CSP
 * meta tag in `index.html`; the encryption upgrade is tracked as a follow-up.
 */
export default function IdentityPanel() {
  const [identity, setIdentity] = useState<KeypairJson | null>(() =>
    readIdentity()
  );
  const [error, setError] = useState<string | null>(null);
  const [isWorking, setIsWorking] = useState(false);
  const importInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Pre-warm the WASM module so the first user action feels instant.
    loadWasm().catch((e) => {
      setError(`WASM module failed to load: ${formatError(e)}`);
    });
  }, []);

  const handleGenerate = async () => {
    setError(null);
    setIsWorking(true);
    try {
      const wasm = await loadWasm();
      const value = wasm.generate_keypair() as KeypairJson;
      writeIdentity(value);
      setIdentity(value);
    } catch (e) {
      setError(`Generate failed: ${formatError(e)}`);
    } finally {
      setIsWorking(false);
    }
  };

  const handleImportClick = () => {
    importInputRef.current?.click();
  };

  const handleImportFile = async (
    e: React.ChangeEvent<HTMLInputElement>
  ): Promise<void> => {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-picking same file
    if (!file) return;

    setError(null);
    setIsWorking(true);
    try {
      const text = await file.text();
      const wasm = await loadWasm();
      const imported = wasm.import_keypair_json(text) as KeypairJson;
      writeIdentity(imported);
      setIdentity(imported);
    } catch (err) {
      setError(`Import failed: ${formatError(err)}`);
    } finally {
      setIsWorking(false);
    }
  };

  const handleExport = async () => {
    if (!identity) return;
    setError(null);
    setIsWorking(true);
    try {
      const wasm = await loadWasm();
      const json = wasm.export_keypair_json(identity);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      const truncated = identity.pubkey_base58.slice(0, 12);
      a.download = `mnemonic-keypair-${truncated}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (err) {
      setError(`Export failed: ${formatError(err)}`);
    } finally {
      setIsWorking(false);
    }
  };

  const handleClear = () => {
    if (
      !window.confirm(
        "Clear local keypair? You will lose access to memories signed by this identity unless you have a backup."
      )
    ) {
      return;
    }
    clearIdentity();
    setIdentity(null);
  };

  return (
    <section className="space-y-4" aria-label="Agent identity">
      <h2 className="text-lg font-semibold text-text-primary">
        Agent identity
      </h2>

      {identity ? (
        <div
          className="space-y-2 rounded-md border border-text-muted/20 bg-white/5 p-4"
          data-testid="identity-display"
        >
          <div className="flex items-center justify-between gap-4">
            <span className="text-xs uppercase tracking-wide text-text-muted">
              DID
            </span>
            <code
              className="truncate font-mono text-sm text-accent-primary"
              data-testid="identity-did"
              aria-label="Agent decentralized identifier"
            >
              did:sol:{identity.pubkey_base58}
            </code>
          </div>
          <div className="flex items-center justify-between gap-4">
            <span className="text-xs uppercase tracking-wide text-text-muted">
              Pubkey
            </span>
            <code
              className="truncate font-mono text-xs text-text-primary"
              data-testid="identity-pubkey"
              aria-label="Ed25519 public key (base58)"
            >
              {identity.pubkey_base58}
            </code>
          </div>
        </div>
      ) : (
        <p className="rounded-md border border-text-muted/20 bg-white/5 p-4 text-sm text-text-muted">
          No identity yet. Generate a new keypair or import an existing backup.
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={handleGenerate}
          disabled={isWorking}
          className="rounded-md bg-accent-primary px-4 py-2 text-sm font-medium text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-generate"
        >
          {identity ? "Generate new" : "Generate"}
        </button>
        <button
          type="button"
          onClick={handleImportClick}
          disabled={isWorking}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-import"
        >
          Import keypair
        </button>
        <button
          type="button"
          onClick={handleExport}
          disabled={isWorking || !identity}
          className="rounded-md border border-text-muted/30 px-4 py-2 text-sm text-text-primary transition-colors hover:border-accent-primary hover:text-accent-primary disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="identity-export"
        >
          Export backup
        </button>
        {identity && (
          <button
            type="button"
            onClick={handleClear}
            disabled={isWorking}
            className="rounded-md border border-error/30 px-4 py-2 text-sm text-error transition-colors hover:border-error disabled:cursor-not-allowed disabled:opacity-40"
            data-testid="identity-clear"
          >
            Clear local store
          </button>
        )}
      </div>

      <input
        ref={importInputRef}
        type="file"
        accept="application/json,.json"
        onChange={handleImportFile}
        className="hidden"
        aria-hidden="true"
      />

      {error && (
        <p
          className="rounded-md border border-error/30 bg-error/10 p-3 text-sm text-error"
          role="alert"
        >
          {error}
        </p>
      )}
    </section>
  );
}

function formatError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return "unknown error";
  }
}
