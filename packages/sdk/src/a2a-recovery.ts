/** External A2A discovery: indexes supply hints; only original signatures establish trust. */
import { ArweaveDiscoverySource, DiscoveryError } from './discovery.js';
import { IntegrityError, UserError } from './errors.js';
import { verifyA2AAttestation } from './a2a.js';
import type { Attestation, A2AIndexStore, A2ARestoreOptions, A2ARestoreReport } from './types.js';

export interface RecoveryHost {
  _a2aIndexStore(): A2AIndexStore;
  _a2aExternal(url:string, init?:RequestInit): Promise<Response>;
  _a2aGateway(): string;
  _a2aDiscovery(): {url:string};
}
const MAX=1048576;
export function envelopeHex(bytes:Uint8Array):string { return Array.from(bytes,b=>b.toString(16).padStart(2,'0')).join(''); }
export async function boundedBody(response:Response):Promise<Uint8Array> {
  if(!response.ok) throw new Error(`external delivery status ${response.status}`);
  const length=Number(response.headers.get('content-length')??0);
  if(length>MAX)throw new IntegrityError('external response too large');
  const reader=response.body?.getReader();if(!reader)throw new IntegrityError('missing external body');
  const chunks:Uint8Array[]=[];let size=0;
  try { while(true){const r=await reader.read();if(r.done)break;size+=r.value.length;if(size>MAX)throw new IntegrityError('external response too large');chunks.push(r.value);} }
  finally {await reader.cancel().catch(()=>{});reader.releaseLock();}
  const out=new Uint8Array(size);let i=0;for(const c of chunks){out.set(c,i);i+=c.length;}return out;
}
export async function decodeAttestation(bytes:Uint8Array,author:string,locator?:string):Promise<Attestation> {
  const hex=envelopeHex(bytes),v=await verifyA2AAttestation(hex,author),b=v.binding;
  return {attestationId:`a2a:${v.content_hash}`,contentHash:v.content_hash,signerPubkey:v.signer,
    contextId:b.context_id,prevId:b.prev_id,kind:b.kind as Attestation['kind'],signedAt:b.created_at,
    sealed:b.sealed,payload:b.payload,coseEnvelopeHex:hex,...(locator?{locator}:{}),
    ...(b.stream?{stream:b.stream}:{}),...(b.sealed?{sealedPayload:(b.payload.parts as any[])[0].data}:{})};
}
function validatedLocator(locator:string):string {
  if(!/^ar:\/\/[A-Za-z0-9_-]{43}$/.test(locator))throw new UserError('invalid ar:// A2A locator');
  return locator.slice(5);
}
export async function fetchAttestation(this:RecoveryHost,locator:string,author:string):Promise<Attestation> {
  if(!/^ar:\/\/[A-Za-z0-9_-]{43}$/.test(locator))throw new UserError('invalid ar:// A2A locator');
  const response=await this._a2aExternal(`${this._a2aGateway()}/${locator.slice(5)}`);
  return decodeAttestation(await boundedBody(response),author,locator);
}
export async function importA2AAttestation(this:RecoveryHost,locator:string,author:string):Promise<Attestation> {
  const row=await fetchAttestation.call(this,locator,author);await this._a2aIndexStore().put(row);return row;
}

/** Reverify cached bytes: caller-supplied/local metadata never authorizes an edge. */
export async function verifyParent(child:Attestation,parent:Attestation):Promise<void> {
  if(!parent.coseEnvelopeHex||!parent.signerPubkey)throw new IntegrityError('parent omitted signed bytes');
  const verified=await decodeAttestation(Uint8Array.from(parent.coseEnvelopeHex.match(/../g)!.map(s=>parseInt(s,16))),parent.signerPubkey);
  if(child.prevId!==verified.attestationId||child.attestationId===verified.attestationId||child.contextId!==verified.contextId)throw new IntegrityError('parent hash/context mismatch');
  if(child.signerPubkey!==verified.signerPubkey){
    // Grant signatures/bindings are verified by verify_a2a. Read identities are
    // obtained from the signed grant CBOR via the WASM parent-link verifier.
    const {loadWasm}=await import('./wasm.js');const wasm=await loadWasm();
    if(!wasm.verify_a2a_parent)throw new IntegrityError('parent-link WASM bindings unavailable');
    wasm.verify_a2a_parent(child.coseEnvelopeHex!,parent.coseEnvelopeHex);
  }
}

export async function restoreA2AContext(this:RecoveryHost,context:string,opts:A2ARestoreOptions):Promise<A2ARestoreReport> {
  if(!context||!opts.expectedAuthors.length||opts.expectedAuthors.some(a=>!a))throw new UserError('context and pinned authors required');
  const authors=[...new Set(opts.expectedAuthors)].sort();const cfg=this._a2aDiscovery();
  const source=opts.discoverySource === false ? undefined : opts.discoverySource ?? new ArweaveDiscoverySource(cfg.url, this._a2aExternal.bind(this));
  const scope=JSON.stringify(['a2a',context,authors,source?.identity??'disabled',source?.supportedBackends??[]]);
  if(opts.checkpoint && opts.checkpoint.scope!==scope)throw new UserError('checkpoint scope mismatch');
  const maxPages=opts.maxPages??100,maxCandidates=opts.maxCandidates??10000;
  if(!Number.isInteger(maxPages)||maxPages<1||maxPages>100||!Number.isInteger(maxCandidates)||maxCandidates<1||maxCandidates>10000)throw new UserError('invalid discovery budget');
  const report:A2ARestoreReport={attestations:[],scanExhausted:false,budgetExhausted:false,missingParents:[],invalidCandidates:[],completeToHeads:false,completeness:'unknown',source:{source:source?.identity??'disabled',status:source?'partial':'disabled',pages:0,candidates:0}};
  const verified=new Map<string,Attestation>();
  for(const row of [...await this._a2aIndexStore().list(),...(opts.checkpoint?.staged??[])])if(row.contextId===context&&row.coseEnvelopeHex&&row.signerPubkey&&authors.includes(row.signerPubkey)){
    try {const v=await decodeAttestation(Uint8Array.from(row.coseEnvelopeHex.match(/../g)!.map(s=>parseInt(s,16))),row.signerPubkey,row.locator);if(v.contextId===context)verified.set(v.attestationId,v);}catch{/* keep bad local metadata out of graph */}
  }
  let cursor=opts.checkpoint?.cursor;const seen=new Set<string>(opts.checkpoint?.seenCursors??[]);let count=0;
  for(let page=0;source && page<maxPages;page++) {
    try {
      opts.signal?.throwIfAborted();
      const result=await source.page({artifactKind:'a2a',context,expectedAuthors:[...authors]},cursor,opts.signal);
      report.source.pages++;
      if(!Array.isArray(result.candidates)||result.candidates.length>100||
         (result.nextCursor!==undefined&&(typeof result.nextCursor!=='string'||!result.nextCursor)))
        throw new DiscoveryError('malformed','malformed discovery page');
      // Finish whole pages; a continuation must never skip unprocessed candidates.
      if(count+result.candidates.length>maxCandidates){report.budgetExhausted=true;break;}
      for(let i=0;i<result.candidates.length;i+=4){
        opts.signal?.throwIfAborted();
        const batch=result.candidates.slice(i,i+4);
        const results=await Promise.allSettled(batch.map(async(candidate)=>{
          if(candidate.backend!=='arweave'||!source.supportedBackends.includes(candidate.backend))throw new Error('unsupported discovery backend');
          const metadata=candidate.metadata;
          // Metadata is optional. If provided, conflicting scoped hints are rejected.
          const required:Record<string,string[]>={'App-Name':['mnemonic-protocol'],'Mnemonic-Type':['a2a'],'Context-Id':[context],'Producer':authors};
          if(metadata)for(const [name,allowed] of Object.entries(required)){
            const values=metadata[name];if(!values||values.length!==1||!allowed.includes(values[0]!))throw new Error('conflicting discovery tags');
          }
          const response=await this._a2aExternal(`${this._a2aGateway()}/${validatedLocator(candidate.locator)}`,opts.signal?{signal:opts.signal}:undefined);
          const bytes=await boundedBody(response);
          let row:Attestation|undefined;
          // A source may suggest a pinned author, but cannot extend the trusted set.
          for(const author of metadata?.Producer??authors){try{row=await decodeAttestation(bytes,author,candidate.locator);break;}catch{}}
          if(!row)throw new IntegrityError('signature does not match pinned authors');
          if(row.contextId!==context)throw new Error('signed context mismatch');
          if(metadata){const hashes=metadata['Content-Hash'];if(!hashes||hashes.length!==1||hashes[0]!==row.contentHash)throw new Error('binding hash tag mismatch');}
          return row;
        }));
        for(let j=0;j<results.length;j++){const r=results[j]!;count++;if(r.status==='fulfilled')verified.set(r.value.attestationId,r.value);else report.invalidCandidates.push({locator:String(batch[j]?.locator??''),reason:String(r.reason)});}
      }
      opts.signal?.throwIfAborted();
      if(result.nextCursor===undefined){report.scanExhausted=true;report.source.status='exhausted';break;}
      const next=result.nextCursor;if(seen.has(next)||next===cursor)throw new DiscoveryError('malformed','cursor loop');seen.add(next);cursor=next;
      if(page===maxPages-1||count>=maxCandidates){report.budgetExhausted=true;break;}
    }catch(e){report.error=String(e);report.source.status=opts.signal?.aborted?'cancelled':e instanceof DiscoveryError?e.status:'unavailable';report.source.error=report.error;break;}
  }
  report.source.candidates=count;
  if(report.budgetExhausted)report.source.status='budget_exhausted';
  if(cursor)report.source.cursor=cursor;
  if(source&&!report.scanExhausted)report.checkpoint={scope,...(cursor?{cursor}:{}),seenCursors:[...seen],staged:[...verified.values()]};
  for(const [id,hint] of Object.entries(opts.parentLocators??{}))if(!opts.signal?.aborted&&!verified.has(id)){
    try{if(!authors.includes(hint.author))throw new Error('untrusted parent author');const row=await fetchAttestation.call(this,hint.locator,hint.author);if(row.attestationId!==id||row.contextId!==context)throw new Error('parent locator mismatch');verified.set(id,row);}catch(e){report.invalidCandidates.push({locator:hint.locator,reason:String(e)});}
  }
  const good=new Set<string>(),bad=new Set<string>(),visiting=new Set<string>();
  async function visit(id:string):Promise<boolean>{if(good.has(id))return true;if(bad.has(id))return false;const row=verified.get(id);if(!row){report.missingParents.push(id);return false;}if(visiting.has(id)){bad.add(id);return false;}visiting.add(id);let ok=true;if(row.prevId){const parent=verified.get(row.prevId);if(!parent){report.missingParents.push(row.prevId);ok=false;}else{try{await verifyParent(row,parent);ok=await visit(row.prevId);}catch(e){report.invalidCandidates.push({locator:row.locator??id,reason:String(e)});ok=false;}}}visiting.delete(id);(ok?good:bad).add(id);return ok;}
  for(const id of verified.keys())await visit(id);
  report.attestations=[...good].map(id=>verified.get(id)!).sort((a,b)=>Date.parse(b.signedAt)-Date.parse(a.signedAt)||b.attestationId.localeCompare(a.attestationId));
  for(const row of report.attestations)await this._a2aIndexStore().put(row);
  report.missingParents=[...new Set(report.missingParents)];
  for(const head of opts.heads??[])if(!good.has(head)&&!report.missingParents.includes(head))report.missingParents.push(head);
  report.completeToHeads=!!opts.heads?.length&&opts.heads.every(h=>good.has(h));
  report.completeness=report.completeToHeads?'complete_to_heads':'unknown';return report;
}
