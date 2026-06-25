/**
 * Client-side trustless RAG (Wave 3 of project-knowledge-context).
 *
 * Retrieval runs entirely in the browser:
 *   1. fetch the per-release manifest from the MCP server (`/knowledge-manifest`)
 *   2. fetch each chunk's COSE_Sign1 bytes from an Arweave gateway
 *   3. verify the signature + decompress the embedding in WASM
 *      (`verify_and_extract_memory`) — the gateway is NOT trusted
 *   4. embed the query with the same model the corpus used and rank by cosine
 *
 * The orchestrator takes its side-effecting collaborators (wasm, query embedder,
 * byte fetcher) as injected deps so it is unit-testable without a real model,
 * network, or WASM module.
 */

export interface ManifestChunk {
  rel_path: string;
  heading: string;
  content_hash: string;
  arweave_tx: string;
  attestation_id: string;
}

export interface KnowledgeManifest {
  type: string;
  release: string;
  corpus_ts: string;
  git_sha: string;
  embed_provider: string;
  embed_model: string;
  chunk_count: number;
  chunks: ManifestChunk[];
}

export interface RetrievedChunk {
  content: string;
  heading: string;
  rel_path: string;
  signer: string;
  score: number;
}

/** Collaborators the orchestrator needs — injected for testability. */
export interface RagDeps {
  /** Verify + extract a chunk from its raw COSE bytes (WASM). */
  verifyAndExtract: (bytes: Uint8Array) => {
    valid: boolean;
    signer: string;
    content: string;
    embedding: number[];
  };
  /** Cosine similarity between two equal-length vectors (WASM). */
  cosine: (a: Float32Array, b: Float32Array) => number;
  /** Embed the user query into a vector comparable with the corpus. */
  embedQuery: (query: string) => Promise<Float32Array>;
  /** Fetch raw bytes for a URL (Arweave gateway). */
  fetchBytes: (url: string) => Promise<Uint8Array>;
}

/** A chunk whose tx is a real on-chain id (not a synthetic `local:` placeholder). */
export function isAnchored(chunk: ManifestChunk): boolean {
  const tx = chunk.arweave_tx;
  return !!tx && !tx.startsWith("local:");
}

/** Resolve an Arweave tx id to a gateway URL. */
export function arweaveUrl(tx: string, gateway = "https://arweave.net"): string {
  return `${gateway.replace(/\/$/, "")}/${tx}`;
}

/** Fetch the per-release knowledge manifest from the MCP server. */
export async function fetchManifest(baseUrl = ""): Promise<KnowledgeManifest> {
  const res = await fetch(`${baseUrl}/knowledge-manifest`, { method: "GET" });
  if (!res.ok) {
    throw new Error(`knowledge manifest unavailable (HTTP ${res.status})`);
  }
  return (await res.json()) as KnowledgeManifest;
}

export interface RetrieveOptions {
  query: string;
  manifest: KnowledgeManifest;
  deps: RagDeps;
  topK?: number;
  gateway?: string;
  /** Continue past chunks that fail to fetch/verify rather than throwing. */
  onChunkError?: (chunk: ManifestChunk, err: unknown) => void;
}

/**
 * Retrieve the top-k most relevant, signature-verified chunks for `query`.
 *
 * Only anchored chunks (real Arweave tx) are fetched — synthetic `local:` rows
 * have no on-chain bytes to verify. A chunk that fails to fetch, fails
 * verification, or whose embedding can't be ranked is skipped (reported via
 * `onChunkError`) so one bad chunk never aborts the whole retrieval.
 */
export async function retrieveContext(
  opts: RetrieveOptions
): Promise<RetrievedChunk[]> {
  const { query, manifest, deps, topK = 3, gateway, onChunkError } = opts;

  const queryEmbedding = await deps.embedQuery(query);
  const anchored = manifest.chunks.filter(isAnchored);

  const scored: RetrievedChunk[] = [];
  for (const chunk of anchored) {
    try {
      const bytes = await deps.fetchBytes(arweaveUrl(chunk.arweave_tx, gateway));
      const extracted = deps.verifyAndExtract(bytes);
      if (!extracted.valid) {
        onChunkError?.(chunk, new Error("signature did not verify"));
        continue;
      }
      const score = deps.cosine(
        queryEmbedding,
        Float32Array.from(extracted.embedding)
      );
      scored.push({
        content: extracted.content,
        heading: chunk.heading,
        rel_path: chunk.rel_path,
        signer: extracted.signer,
        score,
      });
    } catch (err) {
      onChunkError?.(chunk, err);
    }
  }

  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, Math.max(0, topK));
}
