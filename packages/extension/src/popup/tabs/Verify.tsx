// Verify tab — paste an `attestation_id` or drop / pick a `.cose`
// file and show the verification outcome. Three terminal states:
// verified, tampered, not_found. Cloud-tier details (signer, tx ids)
// render when the attestation row carries them.
//
// Round-2 review fixes:
//   - MAJ-1: file-drop runs through the runtime which returns
//     `not_found { reason: "file_drop_unsupported" }`; the UI renders
//     a clear placeholder instead of the misleading generic "not
//     found" state.
//   - MAJ-2: the local presence check returns `verified` with
//     `presence_only: true`; the UI labels it "STORED LOCALLY —
//     cryptographic verification coming soon" so users are not misled
//     into thinking the COSE signature was checked.
//   - UX a11y: drag-and-drop zone is now reachable as a button with
//     a `<input type="file">` fallback; each outcome carries a
//     non-color indicator (check / x / question) per WCAG 1.4.1.

import { useRef, useState, type DragEvent, type JSX } from "react";
import { getRuntime, type VerifyOutcome } from "../runtime.js";

export function Verify(): JSX.Element {
  const [value, setValue] = useState("");
  const [outcome, setOutcome] = useState<VerifyOutcome | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isOver, setIsOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const runIdLookup = async (): Promise<void> => {
    setError(null);
    setOutcome(null);
    const trimmed = value.trim();
    if (trimmed === "") {
      setError("Paste an attestation id first.");
      return;
    }
    setBusy(true);
    try {
      const out = await getRuntime().verify({ attestationId: trimmed });
      setOutcome(out);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const verifyFile = async (file: File): Promise<void> => {
    setError(null);
    setOutcome(null);
    setBusy(true);
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      const out = await getRuntime().verify({ fileBytes: buf });
      setOutcome(out);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDrop = async (e: DragEvent<HTMLDivElement>): Promise<void> => {
    e.preventDefault();
    setIsOver(false);
    const file = e.dataTransfer.files?.[0];
    if (!file) return;
    await verifyFile(file);
  };

  const handlePickerChange = async (
    e: React.ChangeEvent<HTMLInputElement>
  ): Promise<void> => {
    const file = e.target.files?.[0];
    if (!file) return;
    await verifyFile(file);
    // Reset so picking the same file twice still fires the change event.
    e.target.value = "";
  };

  return (
    <div className="flex flex-col gap-3">
      <label className="flex flex-col gap-1 text-xs">
        <span className="text-text-muted">Attestation id</span>
        <input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          aria-label="Attestation id"
          placeholder="att_…"
          className="bg-black/40 border border-white/10 rounded p-2 text-xs font-mono focus:outline-none focus:border-accent-primary"
        />
      </label>
      <button
        type="button"
        onClick={() => void runIdLookup()}
        disabled={busy}
        aria-busy={busy}
        className="bg-accent-primary/20 hover:bg-accent-primary/30 disabled:opacity-50 text-accent-primary border border-accent-primary/50 text-xs font-mono uppercase tracking-wide py-2 rounded transition-colors"
      >
        {busy ? "Verifying…" : "Verify"}
      </button>

      <div
        onDragOver={(e) => {
          e.preventDefault();
          setIsOver(true);
        }}
        onDragLeave={() => setIsOver(false)}
        onDrop={(e) => void handleDrop(e)}
        role="group"
        aria-label="Drop a COSE file"
        aria-describedby="cose-drop-hint"
        className={`flex flex-col items-center gap-2 text-[11px] text-center font-mono py-4 rounded border border-dashed transition-colors ${
          isOver
            ? "border-accent-primary text-accent-primary"
            : "border-white/15 text-text-muted"
        }`}
      >
        <span id="cose-drop-hint">…or drop a signed .cose file here</span>
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          className="text-[10px] uppercase tracking-wide font-mono text-text-muted hover:text-accent-primary border border-white/10 hover:border-accent-primary/50 rounded px-3 py-1 transition-colors"
        >
          Choose file
        </button>
        <input
          ref={fileInputRef}
          type="file"
          accept=".cose,application/cbor,application/octet-stream"
          hidden
          onChange={(e) => void handlePickerChange(e)}
        />
      </div>

      {error ? (
        <div className="text-xs text-error font-mono">{error}</div>
      ) : null}
      {outcome ? <Outcome value={outcome} /> : null}
    </div>
  );
}

function Outcome({ value }: { value: VerifyOutcome }): JSX.Element {
  if (value.status === "verified") {
    const presenceOnly = value.presence_only === true;
    return (
      <div
        role="status"
        className={`border rounded p-2 flex flex-col gap-1 text-xs font-mono ${
          presenceOnly
            ? "border-accent-primary/40 bg-accent-primary/10 text-accent-primary"
            : "border-success/40 bg-success/10 text-success"
        }`}
      >
        <span className="uppercase tracking-wide text-[10px] flex items-center gap-1">
          <span aria-hidden="true">{presenceOnly ? "?" : "✓"}</span>
          <span>{presenceOnly ? "stored locally · verified" : "verified"}</span>
        </span>
        {presenceOnly ? (
          <span className="text-[10px] opacity-80">
            Cryptographic verification coming soon — this confirms the local
            record exists with a non-empty COSE envelope.
          </span>
        ) : null}
        <span>signer: {truncateMiddle(value.signer_pubkey, 6, 6)}</span>
        <span>created: {value.created_at}</span>
        <span>hash: {truncateMiddle(value.content_hash, 8, 8)}</span>
        {value.source ? <span>source: {value.source.platform}</span> : null}
        {value.solana_tx ? (
          <span>sol: {truncateMiddle(value.solana_tx, 6, 6)}</span>
        ) : null}
        {value.arweave_tx ? (
          <span>ar: {truncateMiddle(value.arweave_tx, 6, 6)}</span>
        ) : null}
      </div>
    );
  }
  if (value.status === "tampered") {
    return (
      <div
        role="status"
        className="border border-error/40 bg-error/10 text-error rounded p-2 text-xs font-mono flex flex-col gap-1"
      >
        <span className="uppercase tracking-wide text-[10px] flex items-center gap-1">
          <span aria-hidden="true">✗</span>
          <span>tampered</span>
        </span>
        <span>Verification failed: {value.reason}</span>
      </div>
    );
  }
  if (value.reason === "file_drop_unsupported") {
    return (
      <div
        role="status"
        className="border border-white/15 bg-white/5 text-text-muted rounded p-2 text-xs font-mono flex flex-col gap-1"
      >
        <span className="uppercase tracking-wide text-[10px] flex items-center gap-1">
          <span aria-hidden="true">?</span>
          <span>file verification coming soon</span>
        </span>
        <span>
          Paste the attestation id instead — drop-file verification is a pending
          follow-up.
        </span>
      </div>
    );
  }
  return (
    <div
      role="status"
      className="border border-white/15 bg-white/5 text-text-muted rounded p-2 text-xs font-mono flex flex-col gap-1"
    >
      <span className="uppercase tracking-wide text-[10px] flex items-center gap-1">
        <span aria-hidden="true">?</span>
        <span>not found</span>
      </span>
      {value.attestation_id ? <span>id: {value.attestation_id}</span> : null}
    </div>
  );
}

function truncateMiddle(value: string, head: number, tail: number): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}
