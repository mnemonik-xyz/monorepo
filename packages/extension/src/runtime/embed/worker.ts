/// <reference lib="webworker" />
// Dedicated Web Worker that hosts `@xenova/transformers` and runs the
// feature-extraction pipeline off the popup thread. The pipeline is
// lazy-initialised on the first `init` message — model bytes (~25MB ONNX) are
// downloaded once and cached by `transformers.js` via the browser's Cache API
// (`env.useBrowserCache = true`, default).
//
// Wire protocol is in `./types.ts`. Keep this file dependency-free at module
// scope so the bundler emits a small synchronous worker entry; the heavy
// transformers import is dynamic and only happens when the popup actually
// signs a memory.

import type { WorkerRequest, WorkerResponse } from "./types";

type FeatureExtractionPipeline = (
  texts: string | string[],
  opts: { pooling: "mean" | "none"; normalize: boolean },
) => Promise<{ data: Float32Array | number[]; dims: number[] }>;

let pipelinePromise: Promise<FeatureExtractionPipeline> | null = null;

function post(msg: WorkerResponse, transfer?: Transferable[]): void {
  // `self` in a module worker is `DedicatedWorkerGlobalScope` — postMessage
  // exists. The cast keeps the file compilable under `lib: ["WebWorker"]`
  // without dragging in `dom` lib types.
  const scope = self as unknown as DedicatedWorkerGlobalScope;
  if (transfer && transfer.length > 0) {
    scope.postMessage(msg, transfer);
  } else {
    scope.postMessage(msg);
  }
}

async function loadPipeline(
  modelId: string,
  quantized: boolean,
): Promise<FeatureExtractionPipeline> {
  if (pipelinePromise) return pipelinePromise;
  pipelinePromise = (async () => {
    const transformers = await import("@xenova/transformers");
    // Cache model artefacts in the browser Cache API across sessions; this
    // is the default but we set it explicitly so a future env override can
    // be reasoned about from one place.
    transformers.env.useBrowserCache = true;
    transformers.env.allowLocalModels = false;
    // Disable ORT-WASM multi-threading. Default spawns workers via
    // `URL.createObjectURL(new Blob([…]))` which Chrome MV3 CSP
    // (`script-src 'self' 'wasm-unsafe-eval'`) blocks. Single-thread
    // inference is ~3x slower per token but the only viable mode here.
    transformers.env.backends.onnx.wasm.numThreads = 1;
    const pipe = await transformers.pipeline("feature-extraction", modelId, {
      quantized,
      progress_callback: (info: {
        status: string;
        file?: string;
        progress?: number;
      }) => {
        post({
          type: "progress",
          status: info.status,
          ...(info.file !== undefined ? { file: info.file } : {}),
          ...(info.progress !== undefined ? { progress: info.progress } : {}),
        });
      },
    });
    return pipe as unknown as FeatureExtractionPipeline;
  })();
  return pipelinePromise;
}

async function handleEmbed(id: number, text: string): Promise<void> {
  if (!pipelinePromise) {
    post({
      type: "error",
      id,
      message: "embed received before init",
    });
    return;
  }
  try {
    const pipe = await pipelinePromise;
    // `pooling: "mean"` averages token embeddings into one sentence vector;
    // `normalize: true` L2-normalizes — both match sentence-transformers /
    // fastembed defaults so vectors round-trip with the server's recall.
    const out = await pipe(text, { pooling: "mean", normalize: true });
    const vector =
      out.data instanceof Float32Array ? out.data : new Float32Array(out.data);
    // Transfer the underlying buffer to avoid a copy across the boundary.
    post({ type: "embedded", id, vector }, [vector.buffer]);
  } catch (err) {
    post({
      type: "error",
      id,
      message: err instanceof Error ? err.message : String(err),
    });
  }
}

self.addEventListener("message", (event: MessageEvent<WorkerRequest>) => {
  const msg = event.data;
  switch (msg.type) {
    case "init": {
      loadPipeline(msg.modelId, msg.quantized).then(
        () => post({ type: "ready" }),
        (err: unknown) =>
          post({
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          }),
      );
      return;
    }
    case "embed": {
      void handleEmbed(msg.id, msg.text);
      return;
    }
    case "dispose": {
      pipelinePromise = null;
      (self as unknown as DedicatedWorkerGlobalScope).close();
      return;
    }
  }
});
