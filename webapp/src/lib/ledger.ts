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

/* -------------------------------------------------------------------------- */
/*                              Sample fallback                               */
/*                                                                            */
/* Deterministic illustrative data, used only when the live endpoint is       */
/* absent. Anchored to a fixed date so the bucket counts are test-stable and  */
/* never depend on the wall clock.                                            */
/* -------------------------------------------------------------------------- */

const SAMPLE_ANCHOR = Date.UTC(2026, 5, 27); // 2026-06-27 UTC

/** Representative recalled artifacts saved on the node. */
export function sampleArtifacts(): Artifact[] {
  const rows: Array<Omit<Artifact, "created_at"> & { daysAgo: number }> = [
    {
      id: "a1c0ffee-0001-4a00-9c01-000000000001",
      content:
        "Shipped v0.2 on Tuesday with Alex as release owner. Cut from main, tagged v0.2.0.",
      content_hash:
        "b1946ac92492d2347c6235b4d2611184d3f0a3f9e1c6a2e7c0d4b8a1f2e3d4c5",
      tags: ["release", "v0.2", "decision"],
      solana_tx: "5Nf5h5x2qQk8wq7Yk3J9p1d2c3b4a5n6m7l8k9j0i1h2g3f4e5d6c7b8a9",
      arweave_tx: "kTQ7t1f9c2X3v4B5n6M7l8K9j0I1h2G3f4E5d6C7b8A",
      write_mode: "participate",
      daysAgo: 1,
    },
    {
      id: "a1c0ffee-0002-4a00-9c01-000000000002",
      content:
        "Decided TurboQuant bit width stays at 4 for the production DB — never change for an existing store.",
      content_hash:
        "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b",
      tags: ["architecture", "turboquant"],
      solana_tx: null,
      arweave_tx: null,
      write_mode: "local",
      daysAgo: 3,
    },
    {
      id: "a1c0ffee-0003-4a00-9c01-000000000003",
      content:
        "Agent note: recall spans both local and participate writes for one owner by design.",
      content_hash:
        "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae",
      tags: ["recall", "storage-mode"],
      solana_tx: "local:offline-7f3a",
      arweave_tx: "local:offline-7f3a",
      write_mode: "local",
      daysAgo: 6,
    },
    {
      id: "a1c0ffee-0004-4a00-9c01-000000000004",
      content:
        "Threat model: compressed bytes on Arweave are proof-of-existence only; recall uses uncompressed f32.",
      content_hash:
        "fcde2b2edba56bf408601fb721fe9b5c338d10ee429ea04fae5511b68fbf8fb9",
      tags: ["security", "arweave", "whitepaper"],
      solana_tx: "2Zk9c8x7v6b5n4m3l2k1j0i9h8g7f6e5d4c3b2a1Qp0Wq9Yk8J7p6d5c4b3",
      arweave_tx: "9XbQ7t1f9c2X3v4B5n6M7l8K9j0I1h2G3f4E5d6C7bZ",
      write_mode: "participate",
      daysAgo: 11,
    },
    {
      id: "a1c0ffee-0005-4a00-9c01-000000000005",
      content:
        "Customer call: they want cross-device recall — sign on laptop, recall on phone with the same identity.",
      content_hash:
        "19581e27de7ced00ff1ce50b2047e7a567c76b1cbaebabe5ef03f7c3017bb5b7",
      tags: ["product", "cross-device"],
      solana_tx: null,
      arweave_tx: null,
      write_mode: "local",
      daysAgo: 18,
    },
    {
      id: "a1c0ffee-0006-4a00-9c01-000000000006",
      content:
        "Ed25519 identity is the portable key across providers; the secret never leaves the device.",
      content_hash:
        "4355a46b19d348dc2f57c046f8ef63d4538ebb936000f3c9ee954a27460dd865",
      tags: ["identity", "ed25519"],
      solana_tx: "7Yk3J9p1d2c3b4a5n6m7l8k9j0i1h2g3f4e5d6c7b8a95Nf5h5x2qQk8wq",
      arweave_tx: "tQ7t1f9c2X3v4B5n6M7l8K9j0I1h2G3f4E5d6C7b8Ak",
      write_mode: "participate",
      daysAgo: 27,
    },
  ];
  return rows.map(({ daysAgo, ...rest }) => ({
    ...rest,
    created_at: new Date(SAMPLE_ANCHOR - daysAgo * 86_400_000).toISOString(),
  }));
}

/** Synthetic attestation timeline. Bucket count is fixed per range. */
export function sampleTimeline(range: TimeRange): AttestationTimeline {
  const spec: Record<TimeRange, { count: number; stepDays: number }> = {
    "30d": { count: 30, stepDays: 1 },
    "90d": { count: 90, stepDays: 1 },
    "12m": { count: 12, stepDays: 30 },
  };
  const { count, stepDays } = spec[range];
  let totalNode = 0;
  let totalChain = 0;
  const buckets: TimelineBucket[] = [];
  for (let i = count - 1; i >= 0; i--) {
    const ms = SAMPLE_ANCHOR - i * stepDays * 86_400_000;
    // Deterministic growth curve: rises over time, on-chain ~35% of on-node.
    const idx = count - i;
    const onNode = Math.round(4 + idx * 1.7 + (idx % 5) * 2);
    const onChain = Math.round(onNode * 0.35);
    totalNode += onNode;
    totalChain += onChain;
    buckets.push({
      date: new Date(ms).toISOString().slice(0, 10),
      on_node: onNode,
      on_chain: onChain,
    });
  }
  return {
    buckets,
    total_on_node: totalNode,
    total_on_chain: totalChain,
    unique_users: Math.round(count * 1.4) + 12,
    sample: true,
  };
}
