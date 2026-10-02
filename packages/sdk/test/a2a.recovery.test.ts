import {beforeAll,afterAll,describe,it,expect} from 'vitest';
import * as wasm from '../../../core/pkg-nodejs/mnemonic_core.js';
import {__setWasmForTesting} from '../src/wasm.js';
import {MnemonicClient} from '../src/client.js';
import {Keypair} from '../src/keypair.js';
import {LocalSigner} from '../src/signer.js';
import v from './fixtures/sealed-a2a.json';
const toBytes=(h:string)=>Uint8Array.from(h.match(/../g)!.map(x=>parseInt(x,16)));
const id='A'.repeat(43);
beforeAll(()=>{globalThis.self=globalThis as any;__setWasmForTesting(wasm);});afterAll(()=>__setWasmForTesting(null));
function client(fetch:typeof globalThis.fetch,identity=v.author){const kp=new Keypair(identity);const c=new MnemonicClient({baseUrl:'https://mcp.invalid',signer:new LocalSigner(kp),jwt:'SECRET-JWT',fetch,a2aGatewayUrl:'https://gateway.invalid',a2aIndexUrl:'https://index.invalid/graphql'});c.setKeypair(kp);return c;}
function edge(hex=v.stream,overrides:Record<string,string>={}){const b=JSON.parse(wasm.verify_a2a(toBytes(hex),v.author.pubkey_base58));return {cursor:id,node:{id,tags:Object.entries({'App-Name':'mnemonic-protocol','Mnemonic-Type':'a2a','Producer':v.author.pubkey_base58,'Context-Id':v.context_id,'Content-Hash':b.content_hash,...overrides}).map(([name,value])=>({name,value}))}};}
function remote(edges:any[],blob=v.stream,hasNext=false){return (async(url:any,init?:RequestInit)=>{expect(String(url)).not.toContain('mcp.invalid');expect(new Headers(init?.headers).has('authorization')).toBe(false);return String(url).includes('index.invalid')?Response.json({data:{transactions:{edges,pageInfo:{hasNextPage:hasNext}}}}):new Response(toBytes(blob));}) as typeof fetch;}
describe('SQL-independent client recovery',()=>{
 it('local signing and recall require no network',async()=>{let calls=0;const c=client((async()=>{calls++;throw new Error('offline');}) as typeof fetch);const root=await c.attestA2AMessage(v.payload as any,v.context_id,{mode:'local',sealed:{recipients:[{card:v.card,trustedCardSigner:v.reader.pubkey_base58}],chunkSize:19}});await c.attestA2AMessage(v.payload as any,v.context_id,{mode:'local',prevId:root});const rows=await c.recallA2AContext(v.context_id,{mode:'local'});expect(rows).toHaveLength(2);expect(calls).toBe(0);});
 it('restores native stream fixture and opens locally with pinned head',async()=>{const e=edge();const c=client(remote([e]),v.reader);const hash=e.node.tags.find(t=>t.name==='Content-Hash')!.value;const report=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],heads:[`a2a:${hash}`]});expect(report.completeToHeads).toBe(true);expect(report.scanExhausted).toBe(true);expect(await c.openA2AAttestation(report.attestations[0]!,v.author.pubkey_base58)).toEqual(v.payload);expect(await c.recallA2AContext(v.context_id,{mode:'local'})).toHaveLength(1);});
 it('does not claim global completeness without pinned heads',async()=>{const r=await client(remote([edge()])).restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58]});expect(r.completeness).toBe('unknown');});
 it('rejects forged tags and altered signed bytes',async()=>{for(const [edges,blob]of [[ [edge(v.stream,{'Content-Hash':'fake'})],v.stream],[[edge()],v.stream.slice(0,-2)+(parseInt(v.stream.slice(-2),16)^1).toString(16).padStart(2,'0')]] as const){const r=await client(remote([...edges],blob)).restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58]});expect(r.attestations).toHaveLength(0);expect(r.invalidCandidates).toHaveLength(1);}});
 it('handles cursor loops and reports budget-limited scans',async()=>{const c=client(remote([edge()],v.stream,true));const loop=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58]});expect(loop.error).toContain('cursor loop');expect(loop.scanExhausted).toBe(false);const limited=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],maxPages:1});expect(limited.budgetExhausted).toBe(true);expect(limited.checkpoint).toBeDefined();});
 it('checks known-locator authors and rejects arbitrary fetch URLs',async()=>{const c=client(remote([]));await expect(c.importA2AAttestation(`ar://${id}`,v.outsider.pubkey_base58)).rejects.toThrow();await expect(c.importA2AAttestation('http://attacker/secret',v.author.pubkey_base58)).rejects.toThrow();});
 it('keeps ciphertext public without granting outsiders decryption',async()=>{const c=client(remote([edge()]),v.outsider);const r=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58]});expect(r.attestations).toHaveLength(1);await expect(c.openA2AAttestation(r.attestations[0]!,v.author.pubkey_base58)).rejects.toThrow();});
 it('reports absent pinned heads and index errors',async()=>{const c=client((async()=>Response.json({errors:[{message:'down'}]})) as typeof fetch);const r=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],heads:['a2a:missing']});expect(r.completeToHeads).toBe(false);expect(r.error).toContain('GraphQL');});
});

describe('ancestry and provider bounds',()=>{
 it('reports a signed child whose parent is absent',async()=>{
   const bytes=wasm.prepare_a2a(v.author,'message',JSON.stringify(v.payload),v.context_id,'a2a:'+'b'.repeat(64),v.created_at,undefined,undefined);
   const hex=Array.from(bytes,n=>n.toString(16).padStart(2,'0')).join('');const e=edge(hex);const c=client(remote([e],hex));const r=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],heads:['a2a:'+e.node.tags.find(t=>t.name==='Content-Hash')!.value]});expect(r.completeToHeads).toBe(false);expect(r.missingParents).toContain('a2a:'+'b'.repeat(64));expect(r.attestations).toHaveLength(0);
 });
 it('rejects scope-substituted checkpoints and oversized gateway bodies',async()=>{
   const c=client(remote([]));await expect(c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],checkpoint:{scope:'other'}})).rejects.toThrow();
   const large=client((async()=>new Response(new Uint8Array(1048577))) as typeof fetch);await expect(large.importA2AAttestation(`ar://${id}`,v.author.pubkey_base58)).rejects.toThrow('too large');
 });
 it('uses Arweave query fields only for Arweave indexes',async()=>{
   for(const flavour of ['irys','arweave'] as const){let query='';const kp=new Keypair(v.author);const c=new MnemonicClient({baseUrl:'https://mcp.invalid',signer:new LocalSigner(kp),a2aIndexFlavour:flavour,fetch:(async(_u,init)=>{query=JSON.parse(init!.body as string).query;return Response.json({data:{transactions:{edges:[],pageInfo:{hasNextPage:false}}}});}) as typeof fetch});await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58]});expect(query.includes('sort:HEIGHT_ASC')).toBe(flavour==='arweave');expect(query).not.toContain('block');}
 });
});

it('resumes with staged signed children when their parent appears on a later page',async()=>{
 const rootBytes=wasm.prepare_a2a(v.author,'message',JSON.stringify(v.payload),v.context_id,undefined,v.created_at,undefined,undefined);
 const rootHex=Array.from(rootBytes,n=>n.toString(16).padStart(2,'0')).join('');
 const root=JSON.parse(wasm.verify_a2a(rootBytes,v.author.pubkey_base58));
 const childBytes=wasm.prepare_a2a(v.author,'message',JSON.stringify(v.payload),v.context_id,`a2a:${root.content_hash}`,v.created_at,undefined,undefined);
 const childHex=Array.from(childBytes,n=>n.toString(16).padStart(2,'0')).join('');
 const child=edge(childHex),parent=edge(rootHex);parent.cursor='B'.repeat(43);parent.node.id=parent.cursor;
 const fetcher=(async(url:any,init?:RequestInit)=>{
  if(String(url).includes('index.invalid')){const after=JSON.parse(init!.body as string).variables.after;return Response.json({data:{transactions:{edges:after?[parent]:[child],pageInfo:{hasNextPage:!after}}}});}
  return new Response(String(url).endsWith(parent.cursor)?rootBytes:childBytes);
 }) as typeof fetch;
 const first=await client(fetcher).restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],maxPages:1});expect(first.attestations).toHaveLength(0);expect(first.checkpoint?.staged).toHaveLength(1);
 const resumed=await client(fetcher).restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],heads:[`a2a:${child.node.tags.find(t=>t.name==='Content-Hash')!.value}`],checkpoint:first.checkpoint!});expect(resumed.completeToHeads).toBe(true);expect(resumed.attestations).toHaveLength(2);
});
