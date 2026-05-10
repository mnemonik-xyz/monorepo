// Browser-side `Embedder` impl backed by `@xenova/transformers` running in a
// dedicated Web Worker (`./worker.ts`). The worker is lazy-spawned on the
// first `embed` call so the popup paints instantly — model download (~25MB)
// only happens when the user actually signs a memory.
//
// Cold-init has a hard 30s budget; longer than that and we surface a
// structured `EmbedderInitTimeout` so the UI layer can show a fallback /
// retry state instead of hanging forever.

import {
  EmbedderInitTimeout,
  EmbedderRuntimeError,
  type Embedder,
  type WorkerRequest,
  type WorkerResponse,
} from "./types";

/** Server's fastembed default — see `core/src/embed/mod.rs:20`. */
const SERVER_DEFAULT_DIM = 384;
const DEFAULT_MODEL_ID = "Xenova/all-MiniLM-L6-v2";
const DEFAULT_INIT_TIMEOUT_MS = 30_000;

export interface TransformersEmbedderOptions {
  modelId?: string;
  /** Use INT8-quantised weights (~25MB vs ~80MB fp32). Default true. */
  quantized?: boolean;
  initTimeoutMs?: number;
  /** Test seam — inject a worker built from a different URL or a stub. */
  workerFactory?: () => Worker;
}

interface PendingEmbed {
  resolve: (v: Float32Array) => void;
  reject: (err: Error) => void;
}

export class TransformersEmbedder implements Embedder {
  readonly dim = SERVER_DEFAULT_DIM;
  readonly providerName = "transformers.js";
  readonly isOpenWeights = true;
  readonly modelId: string;

  private readonly quantized: boolean;
  private readonly initTimeoutMs: number;
  private readonly workerFactory: () => Worker;

  private worker: Worker | null = null;
  private readyPromise: Promise<void> | null = null;
  private disposed = false;
  private nextRequestId = 1;
  private readonly pending = new Map<number, PendingEmbed>();

  constructor(options: TransformersEmbedderOptions = {}) {
    this.modelId = options.modelId ?? DEFAULT_MODEL_ID;
    this.quantized = options.quantized ?? true;
    this.initTimeoutMs = options.initTimeoutMs ?? DEFAULT_INIT_TIMEOUT_MS;
    this.workerFactory =
      options.workerFactory ??
      (() =>
        new Worker(new URL("./worker.ts", import.meta.url), {
          type: "module",
          name: "mnemonik-embedder",
        }));
  }

  async embed(text: string): Promise<Float32Array> {
    if (this.disposed) {
      throw new EmbedderRuntimeError("embed called on disposed embedder");
    }
    if (text.length === 0) {
      throw new EmbedderRuntimeError("embed: text must not be empty");
    }
    await this.ensureReady();
    const worker = this.worker;
    if (!worker) {
      throw new EmbedderRuntimeError("worker missing after init");
    }
    const id = this.nextRequestId++;
    return new Promise<Float32Array>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      const req: WorkerRequest = { type: "embed", id, text };
      worker.postMessage(req);
    });
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    const worker = this.worker;
    if (!worker) return;
    try {
      const req: WorkerRequest = { type: "dispose" };
      worker.postMessage(req);
    } catch {
      // Worker may already be terminated — ignore.
    }
    worker.terminate();
    this.worker = null;
    this.readyPromise = null;
    for (const { reject } of this.pending.values()) {
      reject(new EmbedderRuntimeError("embedder disposed before reply"));
    }
    this.pending.clear();
  }

  private ensureReady(): Promise<void> {
    if (this.readyPromise) return this.readyPromise;
    this.readyPromise = this.spawnAndInit();
    return this.readyPromise;
  }

  private spawnAndInit(): Promise<void> {
    const worker = this.workerFactory();
    this.worker = worker;
    worker.addEventListener(
      "message",
      (event: MessageEvent<WorkerResponse>) => {
        this.onWorkerMessage(event.data);
      },
    );
    worker.addEventListener("error", (event: ErrorEvent) => {
      this.failAllPending(
        new EmbedderRuntimeError(event.message || "worker error", event.error),
      );
    });

    return new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cleanup();
        const err = new EmbedderInitTimeout(this.initTimeoutMs);
        this.failAllPending(err);
        reject(err);
      }, this.initTimeoutMs);

      const onReady = (event: MessageEvent<WorkerResponse>) => {
        if (event.data.type === "ready") {
          cleanup();
          resolve();
        } else if (event.data.type === "error" && event.data.id === undefined) {
          cleanup();
          reject(new EmbedderRuntimeError(event.data.message));
        }
      };
      const cleanup = () => {
        clearTimeout(timeout);
        worker.removeEventListener("message", onReady);
      };
      worker.addEventListener("message", onReady);

      const req: WorkerRequest = {
        type: "init",
        modelId: this.modelId,
        quantized: this.quantized,
      };
      worker.postMessage(req);
    });
  }

  private onWorkerMessage(msg: WorkerResponse): void {
    if (msg.type === "embedded") {
      const slot = this.pending.get(msg.id);
      if (!slot) return;
      this.pending.delete(msg.id);
      if (msg.vector.length !== this.dim) {
        slot.reject(
          new EmbedderRuntimeError(
            `vector dim ${msg.vector.length} does not match expected ${this.dim}`,
          ),
        );
        return;
      }
      slot.resolve(msg.vector);
      return;
    }
    if (msg.type === "error" && msg.id !== undefined) {
      const slot = this.pending.get(msg.id);
      if (!slot) return;
      this.pending.delete(msg.id);
      slot.reject(new EmbedderRuntimeError(msg.message));
    }
    // `ready` and `progress` are handled inline / dropped here.
  }

  private failAllPending(err: Error): void {
    for (const { reject } of this.pending.values()) reject(err);
    this.pending.clear();
  }
}
