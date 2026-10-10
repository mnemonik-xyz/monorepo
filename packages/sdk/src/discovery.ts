/** Discovery supplies public locator hints, never author trust or plaintext. */
export interface DiscoveryScope { artifactKind: 'a2a'; context: string; expectedAuthors: string[]; }
export interface DiscoveryCandidate { backend: string; locator: string; metadata?: Record<string, string[]>; }
export interface DiscoveryPage { candidates: DiscoveryCandidate[]; nextCursor?: string; }
export interface DiscoverySource {
  /** Stable identity including endpoint, index schema, and configuration version. */
  readonly identity: string;
  readonly supportedBackends: readonly string[];
  page(scope: DiscoveryScope, cursor?: string, signal?: AbortSignal): Promise<DiscoveryPage>;
}
export type DiscoveryStatus = 'exhausted' | 'partial' | 'unavailable' | 'malformed' | 'budget_exhausted' | 'cancelled' | 'disabled';
export interface DiscoveryDiagnostics { source: string; status: DiscoveryStatus; pages: number; candidates: number; cursor?: string; error?: string; }
export class DiscoveryError extends Error {
  constructor(readonly status: 'unavailable' | 'malformed', message: string) { super(message); }
}
async function withAbort<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  signal.throwIfAborted();
  let onAbort: () => void = () => {};
  const aborted = new Promise<never>((_, reject) => { onAbort = () => reject(signal.reason); signal.addEventListener('abort', onAbort, {once:true}); });
  try { return await Promise.race([operation, aborted]); }
  finally { signal.removeEventListener('abort', onAbort); }
}
async function readJson(response: Response, signal: AbortSignal): Promise<any> {
  if (!response.ok) throw new DiscoveryError('unavailable', `index status ${response.status}`);
  const reader = response.body?.getReader();
  if (!reader) throw new DiscoveryError('malformed', 'missing index body');
  const chunks: Uint8Array[] = []; let size = 0;
  try {
    for (;;) { const r = await withAbort(reader.read(), signal); if (r.done) break; size += r.value.length;
      if (size > 1048576) throw new DiscoveryError('malformed', 'index response too large'); chunks.push(r.value); }
  } finally { void reader.cancel().catch(() => {}); reader.releaseLock(); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  try { return JSON.parse(new TextDecoder().decode(bytes)); }
  catch { throw new DiscoveryError('malformed', 'invalid index JSON'); }
}
/** Arweave indexes use explicit ascending block-height ordering. */
export class ArweaveDiscoverySource implements DiscoverySource {
  readonly identity: string;
  readonly supportedBackends = ['arweave'] as const;
  constructor(readonly endpoint: string, private readonly request: (url: string, init?: RequestInit) => Promise<Response> = globalThis.fetch) {
    const url = new URL(endpoint);
    if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password) throw new Error('invalid discovery endpoint');
    this.identity = JSON.stringify(['graphql-v1', endpoint, 'arweave']);
  }
  async page(scope: DiscoveryScope, cursor?: string, signal?: AbortSignal): Promise<DiscoveryPage> {
    signal?.throwIfAborted();
    const controller = new AbortController();
    const abort = () => controller.abort(signal?.reason);
    signal?.addEventListener('abort', abort, {once:true});
    const timeout = setTimeout(() => controller.abort(new Error('discovery deadline exceeded')), 15000);
    try {
    const tags = [ {name:'App-Name',values:['mnemonic-protocol']}, {name:'Mnemonic-Type',values:[scope.artifactKind]},
      {name:'Context-Id',values:[scope.context]}, {name:'Producer',values:scope.expectedAuthors} ];
    const query = `query($tags:[TagFilter!]!,$after:String){transactions(first:100,tags:$tags,after:$after,sort:HEIGHT_ASC){edges{cursor node{id tags{name value}}}pageInfo{hasNextPage}}}`;
    let response: Response;
    try { response = await withAbort(this.request(this.endpoint, {method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({query,variables:{tags,after:cursor??null}}),signal:controller.signal,redirect:'error',credentials:'omit'}),controller.signal); }
    catch (e) { if (signal?.aborted) throw e; throw new DiscoveryError('unavailable', String(e)); }
    const data = await readJson(response, controller.signal);
    if (data.errors?.length) throw new DiscoveryError('unavailable', 'GraphQL errors');
    const tx = data.data?.transactions;
    if (!Array.isArray(tx?.edges) || tx.edges.length > 100 || typeof tx.pageInfo?.hasNextPage !== 'boolean') throw new DiscoveryError('malformed', 'malformed index response');
    const candidates = tx.edges.map((edge: any): DiscoveryCandidate => {
      const metadata: Record<string, string[]> = Object.create(null);
      if (Array.isArray(edge?.node?.tags)) for (const tag of edge.node.tags) {
        if (typeof tag?.name === 'string' && typeof tag.value === 'string') (metadata[tag.name] ??= []).push(tag.value);
      }
      return {backend:'arweave',locator:`ar://${String(edge?.node?.id ?? '')}`,metadata};
    });
    const next = tx.edges.at(-1)?.cursor;
    if (tx.pageInfo.hasNextPage && (typeof next !== 'string' || !next)) throw new DiscoveryError('malformed', 'missing continuation cursor');
    return {candidates, ...(tx.pageInfo.hasNextPage ? {nextCursor:next} : {})};
    } finally { clearTimeout(timeout); signal?.removeEventListener('abort', abort); }
  }
}
