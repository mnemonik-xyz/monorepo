// Browser-side embedder contract. Mirrors `core/src/embed/mod.rs::Embedder`
// (Send/Sync stripped — JS is single-threaded per realm). Concrete impls live
// alongside this file: `transformers-embedder.ts` (production, all-MiniLM-L6-v2
// in a Web Worker) and a mock used in tests.
//
// `dim` MUST equal the server's embedding dimension (384, fastembed default at
// `mcp/src/config.rs` + `core/src/embed/mod.rs:20`). Drift breaks the
// golden-fixture parity test in T05 and silently corrupts cosine recall across
// server / extension memories.

export interface Embedder {
  /** Vector dimension. 384 for the project default. */
  readonly dim: number;
  /** Canonical model id stored in on-chain anchors. Third parties use this
   *  to re-embed and verify. e.g. `"Xenova/all-MiniLM-L6-v2"`. */
  readonly modelId: string;
  /** Provider tag for logs / telemetry. e.g. `"transformers.js"`. */
  readonly providerName: string;
  /** True when the model weights are publicly downloadable — only open
   *  weights produce externally verifiable attestations. */
  readonly isOpenWeights: boolean;

  /** Embed a single piece of text. Resolves to a length-`dim` Float32Array,
   *  L2-normalized (matches fastembed / sentence-transformers default). */
  embed(text: string): Promise<Float32Array>;

  /** Release any underlying resources (workers, model buffers). Calling
   *  `embed` after `dispose` is a programmer error. */
  dispose(): Promise<void>;
}

/** Structured error surfaced when the worker fails to initialise within the
 *  cold-start budget. Callers can branch on `code` to decide whether to
 *  retry, surface a UI banner, or fall back to server-side embedding. */
export class EmbedderInitTimeout extends Error {
  readonly code = "embedder_init_timeout" as const;
  constructor(public readonly timeoutMs: number) {
    super(`Embedder cold-init exceeded ${timeoutMs}ms`);
    this.name = "EmbedderInitTimeout";
  }
}

/** Structured error for runtime embedding failures (worker crashed, model
 *  download blocked by CSP, etc). */
export class EmbedderRuntimeError extends Error {
  readonly code = "embedder_runtime_error" as const;
  constructor(message: string, public readonly cause?: unknown) {
    super(message);
    this.name = "EmbedderRuntimeError";
  }
}

// ── Worker wire protocol ────────────────────────────────────────────────────
// Messages sent across the `postMessage` boundary. Kept narrow on purpose:
// only `embed` requests after `init`, plus a `dispose` cleanup. Progress
// events are advisory (download bar in the popup) and may be dropped.

export type WorkerRequest =
  | { type: "init"; modelId: string; quantized: boolean }
  | { type: "embed"; id: number; text: string }
  | { type: "dispose" };

export type WorkerResponse =
  | { type: "ready" }
  | { type: "progress"; status: string; file?: string; progress?: number }
  | { type: "embedded"; id: number; vector: Float32Array }
  | { type: "error"; id?: number; message: string };
