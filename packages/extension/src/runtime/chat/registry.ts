import type { ChatAdapter } from "./types.js";

/**
 * Flat adapter registry. T07–T09 self-register concrete adapters
 * (ChatGPT / Claude.ai / Gemini) via `registerAdapter`; array order
 * follows registration order, which is match priority.
 *
 * Tests inject stubs via `__setAdaptersForTesting`; the production
 * registry stays empty in T06 (framework-only deliverable per D10).
 */
const adapters: ChatAdapter[] = [];

/**
 * Append an adapter to the registry. Idempotent: registering the
 * same adapter instance twice is a no-op (lets adapter modules
 * call `registerAdapter` at the top level without worrying about
 * double-import side effects from test runners or HMR).
 */
export function registerAdapter(adapter: ChatAdapter): void {
  if (!adapters.includes(adapter)) {
    adapters.push(adapter);
  }
}

/**
 * Match `new URL(url).hostname + pathname` against each adapter's
 * `hostPattern` in registration order. First hit wins; unknown hosts
 * — including non-http(s) URLs and parse failures — return `null`,
 * which the caller treats as "fall back to generic page-selection
 * capture" (D8).
 */
export function selectAdapter(url: string): ChatAdapter | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const target = parsed.hostname + parsed.pathname;
  for (const adapter of adapters) {
    if (adapter.hostPattern.test(target)) {
      return adapter;
    }
  }
  return null;
}

/** Read-only snapshot — exposed for tests + the options page. */
export function listAdapters(): readonly ChatAdapter[] {
  return adapters;
}

/**
 * Replace the registry. Test-only entry point so the production array
 * stays a private constant; concrete adapters land via T07–T09 by
 * pushing into this module directly.
 */
export function __setAdaptersForTesting(next: ChatAdapter[]): void {
  adapters.length = 0;
  adapters.push(...next);
}
