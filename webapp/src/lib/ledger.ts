/**
 * Client for the public ledger surface: recalled artifacts saved on the node,
 * and the attestation-over-time analytics series.
 *
 * Neither endpoint exists on the backend yet (only `GET /stats` counters do).
 * The contracts below are the source of truth for the future Rust handlers:
 *
 *   GET /artifacts?q=&limit=        -> ArtifactPage   (public-visibility rows)
 *   GET /analytics/attestations?range= -> AttestationTimeline
 *
 * Until those ship, each fetcher degrades to representative sample data flagged
 * with `sample: true`, so the UI is always alive and never lies about it.
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
  /** True when the rows are local placeholders, not live node data. */
  sample: boolean;
}

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
  sample: boolean;
}

export type TimeRange = "30d" | "90d" | "12m";

const REQ_TIMEOUT_MS = 6000;

async function getJson<T>(path: string): Promise<T | null> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), REQ_TIMEOUT_MS);
  try {
    const res = await fetch(`${MCP_BASE}${path}`, { signal: ctrl.signal });
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/** Recalled artifacts saved on the node. Falls back to sample rows. */
export async function fetchArtifacts(opts?: {
  q?: string;
  limit?: number;
}): Promise<ArtifactPage> {
  const params = new URLSearchParams();
  if (opts?.q) params.set("q", opts.q);
  if (opts?.limit) params.set("limit", String(opts.limit));
  const qs = params.toString();
  const live = await getJson<Omit<ArtifactPage, "sample">>(
    `/artifacts${qs ? `?${qs}` : ""}`,
  );
  if (live && Array.isArray(live.artifacts)) {
    return { ...live, sample: false };
  }
  const all = sampleArtifacts();
  const q = opts?.q?.toLowerCase().trim();
  const artifacts = q
    ? all.filter(
        (a) =>
          a.content.toLowerCase().includes(q) ||
          a.tags.some((t) => t.toLowerCase().includes(q)),
      )
    : all;
  return { artifacts, total: artifacts.length, sample: true };
}

/** Attestation counts bucketed over time. Falls back to a synthetic series. */
export async function fetchAttestationTimeline(
  range: TimeRange,
): Promise<AttestationTimeline> {
  const live = await getJson<Omit<AttestationTimeline, "sample">>(
    `/analytics/attestations?range=${range}`,
  );
  if (live && Array.isArray(live.buckets)) {
    return { ...live, sample: false };
  }
  return sampleTimeline(range);
}
