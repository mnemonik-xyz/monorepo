// SW-side embedder for the `ui:recall` handler (audit B4 / AUD-C-04).
// Mirrors `popup/embedder-bridge.ts`'s singleton pattern but lives in
// the background folder so the SW realm has its own
// `TransformersEmbedder` instance — the popup and SW are independent
// JS realms; sharing the cached embedder is not possible across them
// without a heavy postMessage hop.
//
// Cold-start cost (~25MB model fetch) is amortised over the SW's
// lifetime: subsequent recall queries reuse the same instance until
// the SW unloads (Chrome MV3 idle-timeout, ~30s with no event).

import type { Embedder } from "../runtime/embed/types.js";

let cachedEmbedder: Embedder | null = null;
let cachedPromise: Promise<Embedder> | null = null;

async function getEmbedder(): Promise<Embedder> {
  if (cachedEmbedder) return cachedEmbedder;
  if (cachedPromise) return cachedPromise;
  cachedPromise = (async () => {
    const { TransformersEmbedder } =
      await import("../runtime/embed/transformers-embedder.js");
    const e = new TransformersEmbedder();
    cachedEmbedder = e;
    return e;
  })();
  return cachedPromise;
}

/** Embed `text` to a length-`dim` Float32Array using the SW's
 *  TransformersEmbedder singleton. The SW invokes this from the
 *  `ui:recall` message handler when the recall-overlay (T13) fires
 *  a query from the content-script realm. */
export async function embedTextForSw(text: string): Promise<Float32Array> {
  const e = await getEmbedder();
  return e.embed(text);
}

/** @internal — test seam. */
export function __setEmbedderForTesting(e: Embedder | null): void {
  cachedEmbedder = e;
  cachedPromise = e ? Promise.resolve(e) : null;
}
