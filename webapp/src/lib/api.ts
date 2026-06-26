/** API client for the Mnemonic Protocol chat backend. */

// Configurable for e2e tests + local dev. Parallels the pattern in
// Sign.tsx / Consent.tsx. Override via Vite env:
//   VITE_MCP_BASE=http://localhost:3000 npm run dev
// In production the default `https://mcp.mnemonik.xyz` is used, which lets
// the static webapp be hosted anywhere (Cloudflare Pages, GitHub Pages,
// any CDN) while the API stays on the VPS.
// Empty string ("") = same-origin (current legacy behaviour when the
// webapp is co-served by nginx on the same domain as the MCP server).
export const MCP_BASE: string =
  (import.meta.env?.VITE_MCP_BASE as string | undefined) ??
  "https://mcp.mnemonik.xyz";

export interface ChatRequest {
  message: string;
  session_id: string;
}

export interface ChatResponse {
  response: string;
}

export interface ChatError {
  error: string;
  code:
    | "rate_limited"
    | "invalid_input"
    | "service_unavailable"
    | "internal_error";
}

/**
 * Maps server error codes to user-facing messages per UX guidelines.
 * No casual language. Precise problem statements.
 */
const ERROR_MESSAGES: Record<string, string> = {
  invalid_input: "Invalid input.",
  rate_limited: "Rate limit exceeded. Wait before sending another request.",
  service_unavailable: "Service temporarily unavailable. Try again later.",
  internal_error: "Internal server error.",
};

/** Returns the user-facing message for a given error code. */
export function getErrorMessage(code: string): string {
  return ERROR_MESSAGES[code] ?? "Unexpected error.";
}

/** HTTP status codes that should trigger auto-retry with backoff. */
const RETRYABLE_STATUS_CODES = new Set([500, 502, 503, 504]);

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 1000;

/**
 * Sends a chat message to the backend with auto-retry on server errors.
 *
 * Retries up to 3 times with exponential backoff (1s, 2s, 4s) for
 * 5xx status codes. Non-retryable errors (400, 429) are thrown immediately.
 */
export async function sendChatMessage(
  request: ChatRequest,
): Promise<ChatResponse> {
  let lastError: Error | null = null;

  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    if (attempt > 0) {
      const delay = BASE_DELAY_MS * Math.pow(2, attempt - 1);
      await new Promise((resolve) => setTimeout(resolve, delay));
    }

    let res: Response;
    try {
      res = await fetch(`${MCP_BASE}/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(request),
      });
    } catch (_networkErr) {
      lastError = new ChatApiError(
        "Network error. Check your connection.",
        "network_error",
        0,
      );
      continue;
    }

    if (res.ok) {
      const data: ChatResponse = await res.json();
      return data;
    }

    // Parse structured error from server (Decision 9 schema)
    let errorBody: ChatError | null = null;
    try {
      errorBody = await res.json();
    } catch {
      // Response body is not JSON -- fall through to generic handling
    }

    const code = errorBody?.code ?? "internal_error";
    const message = getErrorMessage(code);

    // Non-retryable errors: throw immediately
    if (!RETRYABLE_STATUS_CODES.has(res.status)) {
      throw new ChatApiError(message, code, res.status);
    }

    // Retryable server error -- store and continue loop
    lastError = new ChatApiError(message, code, res.status);
  }

  // All retries exhausted
  throw (
    lastError ??
    new ChatApiError(
      "Service temporarily unavailable. Try again later.",
      "service_unavailable",
      503,
    )
  );
}

export interface PublicStats {
  unique_users: number;
  saved_on_node: number;
  saved_onchain: number;
}

/** Fetches public traction counters from `GET /stats`. Returns null on error. */
export async function fetchPublicStats(): Promise<PublicStats | null> {
  try {
    const res = await fetch(`${MCP_BASE}/stats`, { method: "GET" });
    if (!res.ok) return null;
    return (await res.json()) as PublicStats;
  } catch {
    return null;
  }
}

/** Structured error thrown by the API client. */
export class ChatApiError extends Error {
  public readonly code: string;
  public readonly status: number;

  constructor(message: string, code: string, status: number) {
    super(message);
    this.name = "ChatApiError";
    this.code = code;
    this.status = status;
  }
}
