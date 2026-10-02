/** Explicit storage namespaces. An adapter configuration, never a locator,
 * supplies the network origin. No adapter promises perpetual availability. */
import { IntegrityError, UserError } from './errors.js';
import { boundedBody } from './a2a-recovery.js';
export interface StorageCapabilities {
  upload: boolean; fetch: boolean; discovery: boolean; maxBytes: number;
  retention: string; consistency: string; metadataExposure: string;
}
export interface StorageAdapter {
  readonly namespace: 'ar'|'blob';
  readonly identity: string;
  readonly capabilities: StorageCapabilities;
  fetch(locator: string): Promise<Uint8Array>;
  upload(bytes: Uint8Array): Promise<string>;
}
export async function envelopeSha256(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256',bytes.slice().buffer));
  return Array.from(digest,b=>b.toString(16).padStart(2,'0')).join('');
}
export function storageNamespace(locator: string): 'ar'|'blob' {
  if (/^ar:\/\/[A-Za-z0-9_-]{43}$/.test(locator)) return 'ar';
  if (/^blob:\/\/[0-9a-f]{64}$/.test(locator)) return 'blob';
  throw new UserError('unsupported or invalid storage locator');
}
function origin(value: string): string {
  const url = new URL(value);
  if (!['http:','https:'].includes(url.protocol) || url.username || url.password || !['','/'].includes(url.pathname) || url.search || url.hash) throw new UserError('storage endpoint must be a configured HTTP(S) origin');
  return url.origin;
}
function checkBytes(bytes: Uint8Array): void {
  if (!(bytes instanceof Uint8Array) || bytes.length === 0 || bytes.length > 1048576) throw new UserError('storage artifact must contain 1–1048576 bytes');
}
async function deadline<T>(operation:(signal:AbortSignal)=>Promise<T>):Promise<T> {
  const controller=new AbortController();
  let rejectTimeout:(reason:Error)=>void=()=>{};
  const timeout=new Promise<never>((_,reject)=>{rejectTimeout=reject;});
  const timer=setTimeout(()=>{const error=new Error('storage request deadline exceeded');controller.abort(error);rejectTimeout(error);},10000);
  try{return await Promise.race([operation(controller.signal),timeout]);}
  finally{clearTimeout(timer);}
}
export class ArweaveStorageAdapter implements StorageAdapter {
  readonly namespace = 'ar' as const;
  readonly identity: string;
  readonly capabilities = {upload:false,fetch:true,discovery:false,maxBytes:1048576,retention:'External Arweave/Irys policy; availability not guaranteed by this adapter',consistency:'Known-locator gateway reads; index lag is separate',metadataExposure:'Requested locator and original signed bytes are visible to gateway'};
  private readonly origin: string;
  constructor(endpoint: string, private readonly request: typeof fetch = globalThis.fetch) { this.origin=origin(endpoint);this.identity=`arweave-fetch-v1:${this.origin}`; }
  async fetch(locator: string): Promise<Uint8Array> {
    if (storageNamespace(locator)!=='ar') throw new UserError('Arweave adapter requires ar://');
    return deadline(async signal=>boundedBody(await this.request(`${this.origin}/${locator.slice(5)}`,{redirect:'error',credentials:'omit',signal})));
  }
  async upload(_bytes: Uint8Array): Promise<string> { throw new UserError('Arweave migration adapter does not upload; use the paid ingestion contract'); }
}
/** Simple content-addressed object API: PUT/GET /objects/<sha256>. The server
 * must enforce immutable writes and its own retention/access policy. */
export class HttpObjectStorageAdapter implements StorageAdapter {
  readonly namespace = 'blob' as const;
  readonly identity: string;
  readonly capabilities = {upload:true,fetch:true,discovery:false,maxBytes:1048576,retention:'Configured object server policy; no permanent retention claim',consistency:'PUT followed by exact read-back; existing objects must be immutable',metadataExposure:'Object digest, original signed bytes, size and request timing visible to server'};
  private readonly origin: string;
  constructor(endpoint: string, private readonly request: typeof fetch = globalThis.fetch) { this.origin=origin(endpoint);this.identity=`http-object-v1:${this.origin}`; }
  async fetch(locator: string): Promise<Uint8Array> {
    if (storageNamespace(locator)!=='blob') throw new UserError('object adapter requires blob://');
    const digest=locator.slice(7);
    const bytes=await deadline(async signal=>boundedBody(await this.request(`${this.origin}/objects/${digest}`,{redirect:'error',credentials:'omit',signal})));
    if (await envelopeSha256(bytes)!==digest) throw new IntegrityError('object digest mismatch');
    return bytes;
  }
  async upload(bytes: Uint8Array): Promise<string> {
    checkBytes(bytes);bytes=bytes.slice();const digest=await envelopeSha256(bytes);
    const response=await deadline(signal=>this.request(`${this.origin}/objects/${digest}`,{method:'PUT',headers:{'content-type':'application/octet-stream','if-none-match':'*'},body:bytes.buffer as ArrayBuffer,redirect:'error',credentials:'omit',signal}));
    void response.body?.cancel().catch(()=>{});
    if (!response.ok && response.status!==412) throw new Error(`object upload status ${response.status}`);
    const locator=`blob://${digest}`,fetched=await this.fetch(locator);
    if (fetched.length!==bytes.length || fetched.some((v,i)=>v!==bytes[i])) throw new IntegrityError('destination changed original bytes');
    return locator;
  }
}
export function storageAdapter(adapters: readonly StorageAdapter[], locator: string): StorageAdapter {
  const namespace=storageNamespace(locator),matches=adapters.filter(a=>a.namespace===namespace);
  if (matches.length!==1 || !matches[0]!.capabilities.fetch) throw new UserError(`exactly one configured ${namespace} fetch adapter is required`);
  return matches[0]!;
}
