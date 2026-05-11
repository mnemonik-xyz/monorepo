// Tiny toast component for transient success / error feedback. Pure
// presentational by default — the parent owns `show` state. When
// `onDismiss` is supplied the toast self-clears after 4 seconds and
// listens for the Escape key as a keyboard-driven close path
// (round-2 UX-major #3).

import { useCallback, useEffect, type JSX } from "react";

export type ToastKind = "success" | "error" | "info";

export interface ToastProps {
  message: string;
  kind?: ToastKind;
  /** Optional copy-target — when set, renders a small "Copy" button to
   *  the right of the message. The popup uses this for the `attestation_id`
   *  echo after a successful Sign. */
  copyValue?: string;
  /**
   * Called when the user dismisses the toast (Escape key or close
   * button) or when the auto-clear timer fires (default 4 seconds).
   * When omitted the toast stays put — useful for test fixtures that
   * drive visibility directly.
   */
  onDismiss?: () => void;
  /**
   * Auto-clear delay in milliseconds. Defaults to 4000. Set to 0 to
   * disable auto-clear (the Escape key + close button still work).
   */
  autoClearMs?: number;
}

const STYLES: Record<ToastKind, string> = {
  success: "bg-success/20 text-success border-success/40",
  error: "bg-error/20 text-error border-error/40",
  info: "bg-white/10 text-text-muted border-white/20",
};

export function Toast(props: ToastProps): JSX.Element {
  const { message, kind = "info", copyValue, onDismiss, autoClearMs } = props;

  const handleKeyDown = useCallback(
    (e: globalThis.KeyboardEvent): void => {
      if (e.key === "Escape" && onDismiss) {
        e.preventDefault();
        onDismiss();
      }
    },
    [onDismiss]
  );

  useEffect(() => {
    if (!onDismiss) return;
    document.addEventListener("keydown", handleKeyDown);
    const delay = autoClearMs ?? 4000;
    const timer = delay > 0 ? window.setTimeout(onDismiss, delay) : null;
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [onDismiss, autoClearMs, handleKeyDown]);

  return (
    <div
      role="status"
      aria-live="polite"
      className={`flex items-center justify-between gap-2 text-xs px-3 py-2 rounded border ${STYLES[kind]}`}
    >
      <span className="font-mono break-all">{message}</span>
      <div className="flex items-center gap-1">
        {copyValue ? (
          <button
            type="button"
            onClick={() => {
              // navigator.clipboard is allow-listed by the
              // `clipboardWrite` manifest permission.
              void navigator.clipboard?.writeText(copyValue);
            }}
            aria-label="Copy"
            className="text-[10px] uppercase tracking-wide font-mono px-2 py-0.5 rounded border border-current opacity-80 hover:opacity-100"
          >
            copy
          </button>
        ) : null}
        {onDismiss ? (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Dismiss"
            className="text-[10px] uppercase tracking-wide font-mono px-2 py-0.5 rounded border border-current opacity-80 hover:opacity-100"
          >
            ×
          </button>
        ) : null}
      </div>
    </div>
  );
}
