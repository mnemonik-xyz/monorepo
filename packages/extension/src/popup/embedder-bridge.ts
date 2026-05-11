// Thin wrapper around the T04 transformers.js Web Worker. Kept in the
// popup folder (not `runtime/`) because the popup is the only consumer
// today and we want this entire module — plus the worker bootstrap —
// to land behind a dynamic import so the popup's first-paint bundle
// stays under the 50KB size-limit budget.

import type { Embedder } from "../runtime/embed/types.js";

let cachedEmbedder: Embedder | null = null;
let cachedPromise: Promise<Embedder> | null = null;

/**
 * Resolve the singleton embedder. Constructed lazily on first call
 * (popup cold-open never triggers this). Subsequent calls return the
 * same instance — the underlying Web Worker is heavyweight (~25MB
 * model download) and recreating it would burn the model cache.
 */
async function getEmbedder(): Promise<Embedder> {
  if (cachedEmbedder) return cachedEmbedder;
  if (cachedPromise) return cachedPromise;
  cachedPromise = (async () => {
    const { TransformersEmbedder } = await import(
      "../runtime/embed/transformers-embedder.js"
    );
    const e = new TransformersEmbedder();
    cachedEmbedder = e;
    return e;
  })();
  return cachedPromise;
}

/** Embed `text` to a length-`dim` Float32Array. Forwards to the worker
 *  embedder; popup callers await this from inside the Recall / Capture
 *  flows. */
export async function embedText(text: string): Promise<Float32Array> {
  const e = await getEmbedder();
  return e.embed(text);
}

/** @internal — test seam. Tests inject a deterministic embedder so the
 *  popup's recall path can run without spinning up the real worker. */
export function __setEmbedderForTesting(e: Embedder | null): void {
  cachedEmbedder = e;
  cachedPromise = e ? Promise.resolve(e) : null;
}
