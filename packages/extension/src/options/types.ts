// Shared types for the options page sections. Kept minimal — section-
// specific shapes live next to their consumers; this module only holds
// types reused by 2+ sections.

export interface ToastState {
  message: string;
  kind: "success" | "error" | "info";
  nonce: number;
}
