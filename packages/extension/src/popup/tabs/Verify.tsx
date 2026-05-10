// Verify tab — paste an `attestation_id` or drop a `.cose` file and
// show the verification outcome. Three terminal states: verified,
// tampered, not_found. Cloud-tier details (signer, tx ids) render when
// the attestation row carries them.

import { useState, type DragEvent, type JSX } from "react";
import { getRuntime, type VerifyOutcome } from "../runtime.js";

export function Verify(): JSX.Element {
  const [value, setValue] = useState("");
  const [outcome, setOutcome] = useState<VerifyOutcome | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isOver, setIsOver] = useState(false);

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

  const handleDrop = async (e: DragEvent<HTMLDivElement>): Promise<void> => {
    e.preventDefault();
    setIsOver(false);
    setError(null);
    setOutcome(null);
    const file = e.dataTransfer.files?.[0];
    if (!file) return;
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
        aria-label="Drop a COSE file"
        className={`text-[11px] text-center font-mono py-4 rounded border border-dashed transition-colors ${
          isOver
            ? "border-accent-primary text-accent-primary"
            : "border-white/15 text-text-muted"
        }`}
      >
        …or drop a signed .cose file here
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
    return (
      <div
        role="status"
        className="border border-success/40 bg-success/10 text-success rounded p-2 flex flex-col gap-1 text-xs font-mono"
      >
        <span className="uppercase tracking-wide text-[10px]">verified</span>
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
        <span className="uppercase tracking-wide text-[10px]">tampered</span>
        <span>{value.reason}</span>
      </div>
    );
  }
  return (
    <div
      role="status"
      className="border border-white/15 bg-white/5 text-text-muted rounded p-2 text-xs font-mono flex flex-col gap-1"
    >
      <span className="uppercase tracking-wide text-[10px]">not found</span>
      {value.attestation_id ? <span>id: {value.attestation_id}</span> : null}
    </div>
  );
}

function truncateMiddle(value: string, head: number, tail: number): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}
