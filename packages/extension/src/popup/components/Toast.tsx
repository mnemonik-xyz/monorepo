// Tiny toast component for transient success / error feedback. Pure
// presentational — the parent owns `show` state and timing so unit
// tests can drive the visible state directly without fake timers.

import type { JSX } from "react";

export type ToastKind = "success" | "error" | "info";

export interface ToastProps {
  message: string;
  kind?: ToastKind;
  /** Optional copy-target — when set, renders a small "Copy" button to
   *  the right of the message. The popup uses this for the `attestation_id`
   *  echo after a successful Sign. */
  copyValue?: string;
}

const STYLES: Record<ToastKind, string> = {
  success: "bg-success/20 text-success border-success/40",
  error: "bg-error/20 text-error border-error/40",
  info: "bg-white/10 text-text-muted border-white/20",
};

export function Toast(props: ToastProps): JSX.Element {
  const { message, kind = "info", copyValue } = props;
  return (
    <div
      role="status"
      aria-live="polite"
      className={`flex items-center justify-between gap-2 text-xs px-3 py-2 rounded border ${STYLES[kind]}`}
    >
      <span className="font-mono break-all">{message}</span>
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
    </div>
  );
}
