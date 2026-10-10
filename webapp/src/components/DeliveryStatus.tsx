import { useEffect, useState } from "react";
import {
  checkDelivery,
  isArweaveId,
  type DeliveryStage,
  type DeliveryStatus as Delivery,
} from "../lib/delivery";

/** Poll interval while the item is not yet permanent. */
export const DELIVERY_POLL_MS = 30_000;
/** Stop polling after this long; the user can reload the page. */
export const DELIVERY_POLL_LIMIT_MS = 2 * 60 * 60 * 1000;

const STEPS: { stage: Exclude<DeliveryStage, "unknown">; label: string; hint: string }[] = [
  {
    stage: "uploaded",
    label: "Uploaded to ArDrive Turbo",
    hint: "Signed bytes accepted. Readable now from turbo-gateway.com.",
  },
  {
    stage: "bundled",
    label: "In a bundle",
    hint: "Turbo posted the bundle to Arweave. It waits to be mined.",
  },
  {
    stage: "permanent",
    label: "Permanent on Arweave",
    hint: "In an Arweave block. Every Arweave gateway can serve it.",
  },
];

const ORDER: Record<DeliveryStage, number> = { unknown: -1, uploaded: 0, bundled: 1, permanent: 2 };

interface Props {
  arweaveTx: string;
  /** Injected in tests. */
  check?: (id: string) => Promise<Delivery>;
  pollMs?: number;
}

/**
 * Live delivery status after a successful sign. Shows the stage reached and a
 * link that works now, instead of a gateway that answers 404 until the block.
 */
export function DeliveryStatus({ arweaveTx, check = checkDelivery, pollMs = DELIVERY_POLL_MS }: Props) {
  const [status, setStatus] = useState<Delivery | null>(null);

  useEffect(() => {
    // Synthetic ids (for example `local:<id>`) are never delivered to Arweave.
    if (!isArweaveId(arweaveTx)) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const started = Date.now();
    const tick = async () => {
      const next = await check(arweaveTx).catch(
        (): Delivery => ({ stage: "unknown", readUrl: null }),
      );
      if (cancelled) return;
      // Never move backwards: a gateway hiccup must not hide a reached stage.
      setStatus((prev) => (prev && ORDER[prev.stage] > ORDER[next.stage] ? prev : next));
      if (next.stage !== "permanent" && Date.now() - started < DELIVERY_POLL_LIMIT_MS) {
        timer = setTimeout(tick, pollMs);
      }
    };
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [arweaveTx, check, pollMs]);

  const reached = status ? ORDER[status.stage] : -1;
  if (!isArweaveId(arweaveTx)) return null;

  return (
    <div className="space-y-2" data-testid="delivery-status" aria-live="polite">
      <div className="font-medium">Delivery to Arweave</div>
      <ol className="space-y-1">
        {STEPS.map((step, index) => {
          const done = index <= reached;
          const current = index === reached + 1 && reached < 2;
          return (
            <li
              key={step.stage}
              className={done ? "" : "opacity-60"}
              data-testid={`delivery-step-${step.stage}`}
              data-state={done ? "done" : current ? "waiting" : "pending"}
            >
              <span className="font-mono">{done ? "[x]" : current ? "[..]" : "[ ]"}</span>{" "}
              {step.label}
              {done && step.stage === "bundled" && status?.bundleId && (
                <span className="font-mono text-xs break-all"> (bundle {status.bundleId})</span>
              )}
              {done && step.stage === "permanent" && status?.blockHeight !== undefined && (
                <span className="font-mono text-xs"> (block {status.blockHeight})</span>
              )}
              <div className="text-xs text-text-muted">{step.hint}</div>
            </li>
          );
        })}
      </ol>
      {!status && <div className="text-xs">Checking delivery status…</div>}
      {status?.stage === "unknown" && (
        <div className="text-xs">Status is not available yet. The page checks again automatically.</div>
      )}
      {status?.readUrl && (
        <a
          href={status.readUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-block rounded border border-success/40 bg-success/20 px-3 py-1 text-xs font-medium hover:border-success/70 hover:bg-success/30"
          data-testid="delivery-read-link"
        >
          {status.stage === "permanent" ? "View on Arweave" : "View now (turbo-gateway.com)"}
        </a>
      )}
      {status && status.stage !== "permanent" && (
        <div className="text-xs text-text-muted">
          arweave.net shows the memory only after it is in a block, usually within about 10 to 30 minutes.
        </div>
      )}
    </div>
  );
}
