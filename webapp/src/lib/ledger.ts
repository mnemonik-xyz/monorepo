/**
 * Client for the public ledger surface: recalled artifacts saved on the node,
 * and the attestation-over-time analytics series.
 *
 *   GET /artifacts?q=&limit=          -> { artifacts, total }   (public rows)
 *   GET /analytics/attestations?range= -> AttestationTimeline
 *
 * Both fetchers hit the live backend and THROW on any failure (network error,
 * non-2xx, malformed body). There is no sample/placeholder fallback — a broken
 * endpoint must surface as an error in the UI, never as fabricated data.
 */
import { MCP_BASE } from "./api";

export type WriteMode = "local" | "participate";

export interface Artifact {
  /** attestation_id (uuid). */
  id: string;
  /** Recalled memory text. */
  content: string;
  /** blake3 hex of the canonical CBOR payload. */
  content_hash: string;
  tags: string[];
  /** SPL-Memo signature, or null / `local:` when not anchored. */
  solana_tx: string | null;
  /** Arweave tx id, or null / `local:` when not anchored. */
  arweave_tx: string | null;
  /** ISO-8601 timestamp. */
  created_at: string;
  write_mode: WriteMode;
}

export interface ArtifactPage {
  artifacts: Artifact[];
  total: number;
}

/**
 * The raw artifact row as the live backend emits it: the primary key arrives as
 * `attestation_id` (core/mcp naming), not `id`. `fetchArtifacts` remaps it to
 * the webapp's `id`. Both forms are typed optional so a backend that already
 * sends `id` (or omits the key) maps cleanly without a runtime surprise.
 */
type LiveArtifact = Omit<Artifact, "id"> & {
  id?: string;
  attestation_id?: string;
};

export interface TimelineBucket {
  /** ISO date (day granularity). */
  date: string;
  on_node: number;
  on_chain: number;
}

export interface AttestationTimeline {
  buckets: TimelineBucket[];
  total_on_node: number;
  total_on_chain: number;
  unique_users: number;
}

export type TimeRange = "30d" | "90d" | "12m" | "all";

const REQ_TIMEOUT_MS = 6000;

/** GET JSON from the backend. Throws on timeout, network error, non-2xx, or a
 *  non-JSON body — the caller surfaces it as an error state. */
async function getJson<T>(path: string): Promise<T> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), REQ_TIMEOUT_MS);
  try {
    const res = await fetch(`${MCP_BASE}${path}`, {
      signal: ctrl.signal,
      // Same-origin deploy: the apex routes /blog* by Accept (JSON -> backend,
      // text/html -> prerendered page). Be explicit so negotiation is reliable.
      headers: { Accept: "application/json" },
    });
    if (!res.ok) {
      throw new Error(`GET ${path} -> ${res.status}`);
    }
    return (await res.json()) as T;
  } finally {
    clearTimeout(timer);
  }
}

/** Recalled public artifacts saved on the node. */
export async function fetchArtifacts(opts?: {
  q?: string;
  limit?: number;
}): Promise<ArtifactPage> {
  const params = new URLSearchParams();
  if (opts?.q) params.set("q", opts.q);
  if (opts?.limit) params.set("limit", String(opts.limit));
  const qs = params.toString();
  const live = await getJson<{ artifacts: LiveArtifact[]; total?: number }>(
    `/artifacts${qs ? `?${qs}` : ""}`,
  );
  if (!live || !Array.isArray(live.artifacts)) {
    throw new Error("GET /artifacts: malformed response");
  }
  // The backend keys each row on `attestation_id`; the webapp Artifact (and
  // <li key={a.id}> in Ledger.tsx) keys on `id`. Remap so live rows have a
  // usable `id`. solana_tx/arweave_tx pass through untouched — including the
  // `local:`-prefixed and null forms the explorer-link helpers expect.
  const artifacts: Artifact[] = live.artifacts.map(
    ({ attestation_id, ...rest }) => ({
      ...rest,
      id: rest.id ?? attestation_id ?? "",
    }),
  );
  return { artifacts, total: live.total ?? artifacts.length };
}

/** Attestation counts bucketed over time. */
export async function fetchAttestationTimeline(
  range: TimeRange,
): Promise<AttestationTimeline> {
  const live = await getJson<AttestationTimeline>(
    `/analytics/attestations?range=${range}`,
  );
  if (!live || !Array.isArray(live.buckets)) {
    throw new Error("GET /analytics/attestations: malformed response");
  }
  return live;
}
