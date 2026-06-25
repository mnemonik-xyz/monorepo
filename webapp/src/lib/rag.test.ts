import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  arweaveUrl,
  fetchManifest,
  isAnchored,
  retrieveContext,
  type KnowledgeManifest,
  type ManifestChunk,
  type RagDeps,
} from "./rag";

function chunk(over: Partial<ManifestChunk>): ManifestChunk {
  return {
    rel_path: "WHITEPAPER.md",
    heading: "WHITEPAPER.md > Abstract",
    content_hash: "h",
    arweave_tx: "tx-real",
    attestation_id: "att",
    ...over,
  };
}

function manifest(chunks: ManifestChunk[]): KnowledgeManifest {
  return {
    type: "knowledge-manifest",
    release: "0.2.4",
    corpus_ts: "2026-06-25T00:00:00+00:00",
    git_sha: "abc",
    embed_provider: "fastembed",
    embed_model: "all-MiniLM-L6-v2",
    chunk_count: chunks.length,
    chunks,
  };
}

describe("isAnchored / arweaveUrl", () => {
  it("treats local: tx as not anchored", () => {
    expect(isAnchored(chunk({ arweave_tx: "local:abc" }))).toBe(false);
    expect(isAnchored(chunk({ arweave_tx: "" }))).toBe(false);
    expect(isAnchored(chunk({ arweave_tx: "realTx123" }))).toBe(true);
  });

  it("builds a gateway url without double slashes", () => {
    expect(arweaveUrl("TX", "https://arweave.net/")).toBe(
      "https://arweave.net/TX"
    );
  });
});

describe("fetchManifest", () => {
  beforeEach(() => vi.stubGlobal("fetch", vi.fn()));
  afterEach(() => vi.unstubAllGlobals());

  it("parses a 200 manifest", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(JSON.stringify(manifest([chunk({})])), { status: 200 })
    );
    const m = await fetchManifest();
    expect(m.release).toBe("0.2.4");
    expect(m.chunks).toHaveLength(1);
  });

  it("throws on non-200", async () => {
    (fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("nope", { status: 404 })
    );
    await expect(fetchManifest()).rejects.toThrow(/unavailable/);
  });
});

describe("retrieveContext", () => {
  // A fake corpus: chunk "near" embeds close to the query, "far" does not.
  const EMB: Record<string, number[]> = {
    near: [1, 0, 0],
    far: [0, 1, 0],
  };

  function deps(over: Partial<RagDeps> = {}): RagDeps {
    return {
      embedQuery: async () => Float32Array.from([1, 0, 0]),
      fetchBytes: async (url) =>
        new TextEncoder().encode(url.endsWith("near") ? "near" : "far"),
      verifyAndExtract: (bytes) => {
        const key = new TextDecoder().decode(bytes);
        return {
          valid: true,
          signer: "SERVER",
          content: `content-${key}`,
          embedding: EMB[key] ?? [0, 0, 0],
        };
      },
      // Real dot-product cosine so ranking is meaningful.
      cosine: (a, b) => {
        let dot = 0;
        let na = 0;
        let nb = 0;
        for (let i = 0; i < a.length; i++) {
          const x = a[i] ?? 0;
          const y = b[i] ?? 0;
          dot += x * y;
          na += x * x;
          nb += y * y;
        }
        return na && nb ? dot / (Math.sqrt(na) * Math.sqrt(nb)) : 0;
      },
      ...over,
    };
  }

  it("ranks the semantically-closest chunk first and skips local: rows", async () => {
    const m = manifest([
      chunk({ arweave_tx: "far" }),
      chunk({ arweave_tx: "near" }),
      chunk({ arweave_tx: "local:skipme" }),
    ]);
    const out = await retrieveContext({ query: "q", manifest: m, deps: deps() });
    expect(out.map((c) => c.content)).toEqual(["content-near", "content-far"]);
    expect(out[0]!.score).toBeGreaterThan(out[1]!.score);
  });

  it("drops chunks whose signature fails to verify", async () => {
    const m = manifest([chunk({ arweave_tx: "near" })]);
    const out = await retrieveContext({
      query: "q",
      manifest: m,
      deps: deps({ verifyAndExtract: () => ({ valid: false, signer: "", content: "x", embedding: [1, 0, 0] }) }),
    });
    expect(out).toHaveLength(0);
  });

  it("continues past a fetch error and reports it", async () => {
    const errs: string[] = [];
    const m = manifest([
      chunk({ arweave_tx: "boom" }),
      chunk({ arweave_tx: "near" }),
    ]);
    const out = await retrieveContext({
      query: "q",
      manifest: m,
      deps: deps({
        fetchBytes: async (url) => {
          if (url.endsWith("boom")) throw new Error("network");
          return new TextEncoder().encode("near");
        },
      }),
      onChunkError: (c) => errs.push(c.arweave_tx),
    });
    expect(out).toHaveLength(1);
    expect(out[0]!.content).toBe("content-near");
    expect(errs).toEqual(["boom"]);
  });

  it("honors topK", async () => {
    const m = manifest([
      chunk({ arweave_tx: "near" }),
      chunk({ arweave_tx: "far" }),
    ]);
    const out = await retrieveContext({
      query: "q",
      manifest: m,
      deps: deps(),
      topK: 1,
    });
    expect(out).toHaveLength(1);
    expect(out[0]!.content).toBe("content-near");
  });
});
