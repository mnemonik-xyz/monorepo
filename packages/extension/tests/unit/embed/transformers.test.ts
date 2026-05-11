// T04 unit tests + TDD anchors for the browser embedder.
//
// Two anchors are binding per D13:
//
//   1. `dim_matches_server_default_384` — `embedder.dim === 384`. Drives D3
//      vector-dim parity with the server (`core/src/embed/mod.rs:20`).
//   2. `deterministic_seed_matches_fixture` — running the all-MiniLM-L6-v2
//      pipeline on "hello mnemonik" produces a Float32Array with L2 norm
//      ≈ 1.0 and first-8-dims matching `tests/fixtures/embed/seed-vector.json`
//      within 1e-3. Drives reproducibility for cross-client recall.
//
// The wrapper class spawns a real `Worker`, which doesn't exist in Vitest's
// node env. We therefore split the surface:
//
//   - Class-level tests inject a fake `workerFactory` so we exercise the
//     spawn / message / dispose lifecycle without `transformers.js`.
//   - Pipeline-level tests load `@xenova/transformers` directly in node
//     (it ships with `onnxruntime-node`) and assert the *output* matches
//     the recorded fixture. This is the same canonical vector the worker
//     produces in the browser — both call `pipeline("feature-extraction",
//     "Xenova/all-MiniLM-L6-v2", { quantized: true })` with the same
//     pooling/normalize defaults.

import { describe, it, expect, vi } from "vitest";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { TransformersEmbedder } from "../../../src/runtime/embed/transformers-embedder.js";
import {
  EmbedderInitTimeout,
  EmbedderRuntimeError,
  type WorkerRequest,
  type WorkerResponse,
} from "../../../src/runtime/embed/types.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../fixtures/embed/seed-vector.json",
);

interface SeedFixture {
  _comment?: string;
  model_id: string;
  quantized: boolean;
  input: string;
  dim: number;
  first_eight_dims: number[] | null;
  l2_norm: number | null;
  recorded_at: string | null;
}

// ── Fake worker plumbing ────────────────────────────────────────────────────
// Implements just enough of the `Worker` shape for the wrapper to drive its
// lifecycle. The handler decides what to reply for each request.

type Reply = WorkerResponse | WorkerResponse[];

interface FakeWorkerOptions {
  /** If true, never reply to `init` — used to exercise the timeout path. */
  silentInit?: boolean;
  /** Per-request reply, called for each `embed`. */
  onEmbed?: (req: Extract<WorkerRequest, { type: "embed" }>) => Reply;
}

class FakeWorker {
  terminated = false;
  postMessages: WorkerRequest[] = [];
  private readonly listeners = {
    message: new Set<(e: MessageEvent<WorkerResponse>) => void>(),
    error: new Set<(e: ErrorEvent) => void>(),
  };
  constructor(private readonly opts: FakeWorkerOptions = {}) {}

  addEventListener(
    type: "message" | "error",
    cb: ((e: MessageEvent<WorkerResponse>) => void) &
      ((e: ErrorEvent) => void),
  ): void {
    if (type === "message") this.listeners.message.add(cb);
    else this.listeners.error.add(cb);
  }
  removeEventListener(
    type: "message" | "error",
    cb: ((e: MessageEvent<WorkerResponse>) => void) &
      ((e: ErrorEvent) => void),
  ): void {
    if (type === "message") this.listeners.message.delete(cb);
    else this.listeners.error.delete(cb);
  }

  postMessage(msg: WorkerRequest): void {
    this.postMessages.push(msg);
    if (msg.type === "init") {
      if (!this.opts.silentInit) {
        queueMicrotask(() => this.emit({ type: "ready" }));
      }
      return;
    }
    if (msg.type === "embed" && this.opts.onEmbed) {
      const reply = this.opts.onEmbed(msg);
      const replies = Array.isArray(reply) ? reply : [reply];
      queueMicrotask(() => {
        for (const r of replies) this.emit(r);
      });
    }
  }

  terminate(): void {
    this.terminated = true;
  }

  private emit(msg: WorkerResponse): void {
    const event = { data: msg } as MessageEvent<WorkerResponse>;
    for (const cb of this.listeners.message) cb(event);
  }
}

// ── Class-level lifecycle tests ─────────────────────────────────────────────

describe("TransformersEmbedder · contract", () => {
  it("dim_matches_server_default_384 — exposes 384-dim per D3 parity", () => {
    const embedder = new TransformersEmbedder({
      workerFactory: () => new FakeWorker() as unknown as Worker,
    });
    expect(embedder.dim).toBe(384);
    expect(embedder.modelId).toBe("Xenova/all-MiniLM-L6-v2");
    expect(embedder.providerName).toBe("transformers.js");
    expect(embedder.isOpenWeights).toBe(true);
  });

  it("lazy-spawns the worker only on the first embed call", async () => {
    const factory = vi.fn(() => new FakeWorker() as unknown as Worker);
    const embedder = new TransformersEmbedder({ workerFactory: factory });
    expect(factory).not.toHaveBeenCalled();

    const fake = new FakeWorker({
      onEmbed: (req) => ({
        type: "embedded",
        id: req.id,
        vector: new Float32Array(384),
      }),
    });
    factory.mockImplementationOnce(() => fake as unknown as Worker);

    await embedder.embed("hello mnemonik");
    expect(factory).toHaveBeenCalledTimes(1);

    await embedder.embed("again");
    expect(factory).toHaveBeenCalledTimes(1);
  });

  it("two embed calls return vectors of the configured dim", async () => {
    const fake = new FakeWorker({
      onEmbed: (req) => {
        const v = new Float32Array(384);
        // Deterministic-from-id payload — the wrapper just propagates bytes.
        v[0] = req.id;
        return { type: "embedded", id: req.id, vector: v };
      },
    });
    const embedder = new TransformersEmbedder({
      workerFactory: () => fake as unknown as Worker,
    });
    const a = await embedder.embed("hello");
    const b = await embedder.embed("hello");
    expect(a.length).toBe(384);
    expect(b.length).toBe(384);
  });

  it("rejects with EmbedderInitTimeout when worker stays silent past budget", async () => {
    const fake = new FakeWorker({ silentInit: true });
    const embedder = new TransformersEmbedder({
      workerFactory: () => fake as unknown as Worker,
      initTimeoutMs: 25,
    });
    await expect(embedder.embed("hello")).rejects.toBeInstanceOf(
      EmbedderInitTimeout,
    );
  });

  it("rejects when worker reports a vector dim mismatch", async () => {
    const fake = new FakeWorker({
      onEmbed: (req) => ({
        type: "embedded",
        id: req.id,
        vector: new Float32Array(128),
      }),
    });
    const embedder = new TransformersEmbedder({
      workerFactory: () => fake as unknown as Worker,
    });
    await expect(embedder.embed("hello")).rejects.toBeInstanceOf(
      EmbedderRuntimeError,
    );
  });

  it("propagates worker-side errors back through the embed promise", async () => {
    const fake = new FakeWorker({
      onEmbed: (req) => ({
        type: "error",
        id: req.id,
        message: "model download blocked",
      }),
    });
    const embedder = new TransformersEmbedder({
      workerFactory: () => fake as unknown as Worker,
    });
    await expect(embedder.embed("hello")).rejects.toThrow(
      /model download blocked/,
    );
  });

  it("dispose_terminates_worker_cleanly", async () => {
    const fake = new FakeWorker({
      onEmbed: (req) => ({
        type: "embedded",
        id: req.id,
        vector: new Float32Array(384),
      }),
    });
    const embedder = new TransformersEmbedder({
      workerFactory: () => fake as unknown as Worker,
    });
    await embedder.embed("hello");
    expect(fake.terminated).toBe(false);

    await embedder.dispose();
    expect(fake.terminated).toBe(true);
    expect(fake.postMessages.at(-1)).toEqual({ type: "dispose" });

    await expect(embedder.embed("again")).rejects.toBeInstanceOf(
      EmbedderRuntimeError,
    );
  });
});

// ── Pipeline-level fixture parity ───────────────────────────────────────────
// Loads `@xenova/transformers` directly in node to capture / verify the
// canonical vector. Running the same pipeline in the worker produces the
// same bytes (same model file, same tokenizer, same `pooling: "mean"` +
// `normalize: true` flags). When the dependency isn't installed locally
// the test is skipped with an explicit reason; in CI the dep is always
// installed and the assertion runs.

async function tryLoadTransformers(): Promise<typeof import("@xenova/transformers") | null> {
  try {
    return (await import("@xenova/transformers")) as typeof import(
      "@xenova/transformers"
    );
  } catch {
    return null;
  }
}

const pipelineDescribe =
  process.env.MNEMONIK_SKIP_PIPELINE_TESTS === "1" ? describe.skip : describe;

pipelineDescribe("transformers.js pipeline · fixture parity", () => {
  it("deterministic_seed_matches_fixture — first 8 dims of L2-normalised vector for 'hello mnemonik'", async () => {
    const transformers = await tryLoadTransformers();
    if (!transformers) {
      // Surface the skip explicitly so CI can spot dependency drift.
      console.warn(
        "[transformers.test] @xenova/transformers not installed; skipping pipeline parity test.",
      );
      return;
    }
    const fixture = JSON.parse(
      readFileSync(FIXTURE_PATH, "utf8"),
    ) as SeedFixture;
    expect(fixture.model_id).toBe("Xenova/all-MiniLM-L6-v2");
    expect(fixture.dim).toBe(384);

    transformers.env.useBrowserCache = false;
    transformers.env.allowLocalModels = false;
    const pipe = await transformers.pipeline(
      "feature-extraction",
      fixture.model_id,
      { quantized: fixture.quantized },
    );
    const out = (await pipe(fixture.input, {
      pooling: "mean",
      normalize: true,
    })) as { data: Float32Array | number[]; dims: number[] };
    const vector =
      out.data instanceof Float32Array
        ? out.data
        : new Float32Array(out.data);

    expect(vector.length).toBe(fixture.dim);
    const norm = Math.sqrt(vector.reduce((s, x) => s + x * x, 0));
    expect(norm).toBeCloseTo(1.0, 3);

    const firstEight = Array.from(vector.slice(0, 8));

    if (
      !fixture.first_eight_dims ||
      process.env.MNEMONIK_RECORD_EMBED_FIXTURE === "1"
    ) {
      const updated: SeedFixture = {
        ...fixture,
        first_eight_dims: firstEight,
        l2_norm: norm,
        recorded_at: new Date().toISOString(),
      };
      writeFileSync(FIXTURE_PATH, JSON.stringify(updated, null, 2) + "\n");
      console.warn(
        "[transformers.test] recorded fixture; commit tests/fixtures/embed/seed-vector.json to lock parity.",
      );
      return;
    }

    for (let i = 0; i < 8; i++) {
      expect(firstEight[i]).toBeCloseTo(fixture.first_eight_dims[i]!, 3);
    }
    if (fixture.l2_norm !== null) {
      expect(norm).toBeCloseTo(fixture.l2_norm, 3);
    }
  }, 120_000);

  it("two consecutive calls produce identical vectors", async () => {
    const transformers = await tryLoadTransformers();
    if (!transformers) return;
    const pipe = await transformers.pipeline(
      "feature-extraction",
      "Xenova/all-MiniLM-L6-v2",
      { quantized: true },
    );
    const a = (await pipe("hello mnemonik", {
      pooling: "mean",
      normalize: true,
    })) as { data: Float32Array | number[] };
    const b = (await pipe("hello mnemonik", {
      pooling: "mean",
      normalize: true,
    })) as { data: Float32Array | number[] };
    const va = a.data instanceof Float32Array ? a.data : new Float32Array(a.data);
    const vb = b.data instanceof Float32Array ? b.data : new Float32Array(b.data);
    expect(vb.length).toBe(va.length);
    for (let i = 0; i < va.length; i++) {
      expect(vb[i]).toBe(va[i]);
    }
  }, 120_000);
});
