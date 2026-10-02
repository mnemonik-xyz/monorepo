/** External A2A discovery: indexes supply hints; only original signatures establish trust. */
import { IntegrityError, UserError } from './errors.js';
import { verifyA2AAttestation } from './a2a.js';
import type { Attestation, A2AIndexStore, A2ARestoreOptions, A2ARestoreReport } from './types.js';

export interface RecoveryHost {
  _a2aIndexStore(): A2AIndexStore;
  _a2aExternal(url:string, init?:RequestInit): Promise<Response>;
  _a2aGateway(): string;
  _a2aDiscovery(): {url:string; flavour:'irys'|'arweave'};
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
  const scope=JSON.stringify([context,authors,cfg.url,cfg.flavour]);
  if(opts.checkpoint && opts.checkpoint.scope!==scope)throw new UserError('checkpoint scope mismatch');
  const maxPages=opts.maxPages??100,maxCandidates=opts.maxCandidates??10000;
  if(!Number.isInteger(maxPages)||maxPages<1||maxPages>100||!Number.isInteger(maxCandidates)||maxCandidates<1||maxCandidates>10000)throw new UserError('invalid discovery budget');
  const report:A2ARestoreReport={attestations:[],scanExhausted:false,budgetExhausted:false,missingParents:[],invalidCandidates:[],completeToHeads:false,completeness:'unknown'};
  const verified=new Map<string,Attestation>();
  for(const row of [...await this._a2aIndexStore().list(),...(opts.checkpoint?.staged??[])])if(row.contextId===context&&row.coseEnvelopeHex&&row.signerPubkey&&authors.includes(row.signerPubkey)){
    try {const v=await decodeAttestation(Uint8Array.from(row.coseEnvelopeHex.match(/../g)!.map(s=>parseInt(s,16))),row.signerPubkey,row.locator);if(v.contextId===context)verified.set(v.attestationId,v);}catch{/* keep bad local metadata out of graph */}
  }
  let cursor=opts.checkpoint?.cursor;const seen=new Set<string>();let count=0;
  const tags=[{name:'App-Name',values:['mnemonic-protocol']},{name:'Mnemonic-Type',values:['a2a']},{name:'Context-Id',values:[context]},{name:'Producer',values:authors}];
  for(let page=0;page<maxPages;page++) {
    try {
      opts.signal?.throwIfAborted();
      const query=`query($tags:[TagFilter!]!,$after:String){transactions(first:100,tags:$tags,after:$after${cfg.flavour==='arweave'?',sort:HEIGHT_ASC':''}){edges{cursor node{id tags{name value}}}pageInfo{hasNextPage}}}`;
      const response=await this._a2aExternal(cfg.url,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({query,variables:{tags,after:cursor??null}}),...(opts.signal?{signal:opts.signal}:{})});
      const data=JSON.parse(new TextDecoder().decode(await boundedBody(response)));
      if(data.errors?.length)throw new Error('GraphQL errors');
      const tx=data.data?.transactions;
      if(!Array.isArray(tx?.edges)||typeof tx.pageInfo?.hasNextPage!=='boolean')throw new Error('malformed index response');
      // Finish whole pages so the cursor never skips unfetched candidates.
      if(count+tx.edges.length>maxCandidates){report.budgetExhausted=true;break;}
      for(let i=0;i<tx.edges.length;i+=4){
        const batch=tx.edges.slice(i,i+4);
        const results=await Promise.allSettled(batch.map(async(edge:any)=>{
          const id=edge.node?.id;if(typeof id!=='string'||!/^[A-Za-z0-9_-]{43}$/.test(id))throw new Error('invalid index ID');
          if(!Array.isArray(edge.node.tags))throw new Error('missing tags');
          for(const tag of tags){const values=edge.node.tags.filter((t:any)=>t.name===tag.name).map((t:any)=>t.value);if(values.length!==1||!tag.values.includes(values[0]))throw new Error('conflicting discovery tags');}
          const author=edge.node.tags.find((t:any)=>t.name==='Producer').value;
          const row=await fetchAttestation.call(this,`ar://${id}`,author);
          if(row.contextId!==context)throw new Error('signed context mismatch');
          const hashes=edge.node.tags.filter((t:any)=>t.name==='Content-Hash');if(hashes.length!==1||hashes[0].value!==row.contentHash)throw new Error('binding hash tag mismatch');
          return row;
        }));
        for(let j=0;j<results.length;j++){const r=results[j]!;count++;if(r.status==='fulfilled')verified.set(r.value.attestationId,r.value);else report.invalidCandidates.push({locator:String(batch[j]?.node?.id??''),reason:String(r.reason)});}
      }
      if(!tx.pageInfo.hasNextPage){report.scanExhausted=true;break;}
      const next=tx.edges.at(-1)?.cursor;if(typeof next!=='string'||seen.has(next)||next===cursor)throw new Error('cursor loop');seen.add(next);cursor=next;
      if(page===maxPages-1||count>=maxCandidates){report.budgetExhausted=true;break;}
    }catch(e){report.error=String(e);break;}
  }
  if(!report.scanExhausted)report.checkpoint={scope,...(cursor?{cursor}:{}),staged:[...verified.values()]};
  for(const [id,hint] of Object.entries(opts.parentLocators??{}))if(!verified.has(id)){
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
