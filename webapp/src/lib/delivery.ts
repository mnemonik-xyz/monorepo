/**
 * Delivery status of an uploaded memory, checked from the browser.
 *
 * Uploads go to ArDrive Turbo, which bundles items into Arweave transactions.
 * An item is permanent only once an Arweave gateway reports it in a block.
 * Until then, gateways that read from Turbo (turbo-gateway.com) already serve
 * the bytes, while arweave.net answers HTTP 404.
 *
 * Both sources below allow cross-origin requests, so no operator server is
 * involved and the check works for every operator.
 */

/** Turbo upload service status endpoint. */
export const TURBO_STATUS_URL = "https://upload.ardrive.io/v1/tx";
/** Arweave gateway GraphQL index used to confirm the block. */
export const ARWEAVE_GRAPHQL_URL = "https://arweave.net/graphql";
/** Gateway that serves Turbo items before they are in a block. */
export const EARLY_READ_GATEWAY = "https://turbo-gateway.com";
/** Gateway for permanent items. */
export const ARWEAVE_GATEWAY = "https://arweave.net";

export type DeliveryStage =
  /** Turbo accepted the item; not yet in a bundle. */
  | "uploaded"
  /** Turbo put the item in a bundle that waits to be mined. */
  | "bundled"
  /** An Arweave gateway reports the item in a block. */
  | "permanent"
  /** Neither source answered; try again later. */
  | "unknown";

export interface DeliveryStatus {
  stage: DeliveryStage;
  bundleId?: string;
  blockHeight?: number;
  /** URL that serves the bytes now (null while nothing is known). */
  readUrl: string | null;
}

const ARWEAVE_ID = /^[A-Za-z0-9_-]{43}$/;

/** True for a 43-character base64url Arweave item id. */
export function isArweaveId(id: string): boolean {
  return ARWEAVE_ID.test(id);
}

/** Block height of `id` from the gateway index, or null while pending. */
async function blockHeight(id: string, fetchImpl: typeof fetch): Promise<number | null> {
  const res = await fetchImpl(ARWEAVE_GRAPHQL_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query: `{ transaction(id: "${id}") { block { height } } }` }),
  });
  if (!res.ok) throw new Error(`graphql HTTP ${res.status}`);
  const body = (await res.json()) as {
    data?: { transaction?: { block?: { height?: unknown } | null } | null };
  };
  const height = body.data?.transaction?.block?.height;
  return typeof height === "number" ? height : null;
}

/** Turbo's view of the item: `new` (accepted), `pending` (bundled), `permanent`. */
async function turboStatus(
  id: string,
  fetchImpl: typeof fetch,
): Promise<{ info?: string; bundleId?: string } | null> {
  const res = await fetchImpl(`${TURBO_STATUS_URL}/${encodeURIComponent(id)}/status`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`turbo HTTP ${res.status}`);
  const body = (await res.json()) as { info?: unknown; bundleId?: unknown };
  return {
    info: typeof body.info === "string" ? body.info : undefined,
    bundleId: typeof body.bundleId === "string" ? body.bundleId : undefined,
  };
}

/**
 * Check where an uploaded item is. The gateway block is authoritative for
 * "permanent"; Turbo's status reports the earlier stages.
 */
export async function checkDelivery(
  id: string,
  fetchImpl: typeof fetch = globalThis.fetch.bind(globalThis),
): Promise<DeliveryStatus> {
  if (!isArweaveId(id)) return { stage: "unknown", readUrl: null };
  const permanentUrl = `${ARWEAVE_GATEWAY}/${id}`;
  const earlyUrl = `${EARLY_READ_GATEWAY}/${id}`;
  const height = await blockHeight(id, fetchImpl).catch(() => null);
  if (height !== null) {
    return { stage: "permanent", blockHeight: height, readUrl: permanentUrl };
  }
  const turbo = await turboStatus(id, fetchImpl).catch(() => undefined);
  if (turbo === undefined || turbo === null) {
    return { stage: "unknown", readUrl: turbo === null ? null : earlyUrl };
  }
  if (turbo.info === "pending" || turbo.info === "permanent") {
    // "permanent" from Turbo before the index shows a block: keep polling the
    // gateway, but the bundle is on its way.
    return { stage: "bundled", bundleId: turbo.bundleId, readUrl: earlyUrl };
  }
  return { stage: "uploaded", readUrl: earlyUrl };
}
