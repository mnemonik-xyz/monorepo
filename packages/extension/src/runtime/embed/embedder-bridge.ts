// Shared embedder-init helper (T25b prewarm path).
//
// `__ensureEmbedderInitialised` is intentionally idempotent and side-
// effect-only: it spawns the realm's `TransformersEmbedder`, runs one
// throwaway embed call to force the ORT WASM + ONNX model fetch
// through the worker's `ensureReady` lazy gate, and returns. The same
// code path is exercised by the popup's first capture (cold-init paid
// once per popup-open) and by `chrome.runtime.onInstalled` on
// install / update so that first capture doesn't sit for 60-120s
// downloading the model.
//
// Each realm (popup, SW) keeps its own embedder singleton — JS realms
// in Chrome are isolated, so the shared module exists for symmetry +
// to keep prewarm callers honest about what they import. The popup
// continues to drive its embedder through `popup/embedder-bridge.ts`
// for the embed-text path; this module is purely for "prime the
// pump" callers.

import type { Embedder } from "./types.js";

let cachedEmbedder: Embedder | null = null;
let cachedPromise: Promise<Embedder> | null = null;

/**
 * Spawn the embedder (worker + WASM + ONNX) on the current realm.
 * Idempotent: subsequent calls reuse the same singleton. Used by the
 * SW `onInstalled` handler to pre-warm the model cache so first
 * capture is fast.
 *
 * Failure modes (slow network, denied storage, missing WebAssembly)
 * are non-fatal at the callsite — the SW callback wraps in try/catch
 * and logs a warning. The user's first capture re-runs through the
 * popup's bridge and pays the cold-init cost there if prewarm
 * couldn't finish in time.
 */
export async function __ensureEmbedderInitialised(): Promise<void> {
  if (cachedEmbedder) return;
  if (!cachedPromise) {
    cachedPromise = (async () => {
      const { TransformersEmbedder } =
        await import("./transformers-embedder.js");
      const e = new TransformersEmbedder();
      // Trigger the lazy init by issuing one tiny embed call. The
      // worker pulls down the ORT WASM + ONNX model on the first
      // `embed` request; embed() throws if init fails so the caller's
      // try/catch absorbs slow-network / missing-WebAssembly errors.
      await e.embed("init");
      cachedEmbedder = e;
      return e;
    })();
  }
  try {
    await cachedPromise;
  } catch (err) {
    // Drop the failed promise so a subsequent call can retry. Without
    // this the singleton would stick in a rejected state forever.
    cachedPromise = null;
    throw err;
  }
}

/** @internal — test seam. Tests inject a deterministic embedder so
 *  the prewarm path can run without downloading the real model. */
export function __setEmbedderForTesting(e: Embedder | null): void {
  cachedEmbedder = e;
  cachedPromise = e ? Promise.resolve(e) : null;
}
