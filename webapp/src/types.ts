/** Shared types used across pages and components. */

export interface Message {
  id: string;
  role: "user" | "bot";
  content: string;
  isError?: boolean;
}
