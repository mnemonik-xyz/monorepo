// Minimal dialog a11y helper: Escape-to-close + initial focus on a
// designated element (typically the primary action button). Keeps the
// section files free of repeated useEffect boilerplate while staying
// well below the complexity of a full focus-trap library — sufficient
// for two-button confirmation dialogs.
//
// Usage:
//   const primaryRef = useRef<HTMLButtonElement>(null);
//   useDialogA11y(primaryRef, onCancel);
//   return <div role="dialog">…<button ref={primaryRef}>Confirm</button></div>;

import { useEffect, type RefObject } from "react";

export function useDialogA11y<T extends HTMLElement>(
  initialFocusRef: RefObject<T | null>,
  onClose: () => void,
): void {
  useEffect(() => {
    // Move focus to the primary action on mount so keyboard users land
    // inside the dialog immediately. Defer one tick — React 19 may not
    // have committed the ref yet when the effect first runs in some
    // testing-library scenarios.
    const focusHandle = window.setTimeout(() => {
      initialFocusRef.current?.focus();
    }, 0);
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(focusHandle);
      document.removeEventListener("keydown", onKey);
    };
  }, [initialFocusRef, onClose]);
}
