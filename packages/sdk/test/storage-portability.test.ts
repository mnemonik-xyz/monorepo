import {beforeAll,afterAll,describe,it,expect,vi} from 'vitest';
import * as wasm from '../../../core/pkg-nodejs/mnemonic_core.js';
import {__setWasmForTesting} from '../src/wasm.js';
import {Keypair} from '../src/keypair.js';
import {LocalSigner} from '../src/signer.js';
import {MnemonicClient} from '../src/client.js';
import {decodeAttestation} from '../src/a2a-recovery.js';
import {checkpointBytes,signRecoveryCheckpoint} from '../src/checkpoint.js';
import {ArweaveStorageAdapter,HttpObjectStorageAdapter,envelopeSha256} from '../src/storage.js';
import {migrateA2AStorage,restoreMigratedA2A,verifyLocatorManifest,signLocatorManifest} from '../src/storage-portability.js';
import type {RecoveryCheckpoint} from '../src/checkpoint.js';
import type {A2AIndexStore,Attestation} from '../src/types.js';
import v from './fixtures/sealed-a2a.json';
const owner=v.author.pubkey_base58,context=v.context_id,signer=new LocalSigner(new Keypair(v.author));
beforeAll(()=>{globalThis.self=globalThis as any;__setWasmForTesting(wasm);});afterAll(()=>__setWasmForTesting(null));
async function fixture(){
 let sourceOnline=true,sourceFetches=0,writes=0;
 const sourceBytes=new Map<string,Uint8Array>(),destinationBytes=new Map<string,Uint8Array>();
 const payloads=new Map<string,unknown>();
 const source=new ArweaveStorageAdapter('https://source.invalid',async(url,init)=>{sourceFetches++;expect(new Headers(init?.headers).has('authorization')).toBe(false);expect(init?.redirect).toBe('error');if(!sourceOnline)throw new Error('source disabled');const bytes=sourceBytes.get(String(url).split('/').at(-1)!);return bytes?new Response(bytes):new Response('',{status:404});});
 const destinationRequest:typeof fetch=async(url,init)=>{expect(String(url)).toMatch(/^https:\/\/destination.invalid\/objects\/[0-9a-f]{64}$/);expect(new Headers(init?.headers).has('authorization')).toBe(false);const digest=String(url).split('/').at(-1)!;if(init?.method==='PUT'){writes++;const bytes=new Uint8Array(init.body as ArrayBuffer);if(destinationBytes.has(digest))return new Response('',{status:412});destinationBytes.set(digest,bytes.slice());return new Response('',{status:201});}const bytes=destinationBytes.get(digest);return bytes?new Response(bytes):new Response('',{status:404});};
 const destination=new HttpObjectStorageAdapter('https://destination.invalid',destinationRequest);
 const rows:Attestation[]=[];
 for(let i=0;i<3;i++){
  const payload={...v.payload,messageId:`migration-${i}`};
  const bytes=wasm.prepare_a2a(v.author,'message',JSON.stringify(payload),context,i?rows[0]!.attestationId:undefined,`2026-10-02T00:00:0${i}Z`,JSON.stringify([{card:v.card,trusted_card_signer:v.reader.pubkey_base58}]),i?19:undefined);
  const locator=`ar://${String.fromCharCode(65+i).repeat(43)}`;sourceBytes.set(locator.slice(5),bytes);
  const row=await decodeAttestation(bytes,owner,locator);rows.push(row);payloads.set(row.attestationId,payload);
 }
 const spec:RecoveryCheckpoint={version:1,artifactKind:'a2a',scope:context,expectedAuthors:[owner],heads:rows.slice(1).map(r=>r.attestationId),locators:rows.map(r=>({artifactId:r.attestationId,author:owner,locator:r.locator!})),backendHints:['arweave'],createdAt:'2026-10-02T00:00:00.000Z'};
 return {source,destination,destinationRequest,destinationBytes,sourceBytes,rows,payloads,spec,disableSource:()=>{sourceOnline=false;},stats:()=>({sourceFetches,writes})};
}
describe('exact-byte portable A2A storage',()=>{
 it('migrates sealed grants and completed streams, shuts off original source/operator, restores forks and opens as recipient',async()=>{
  const f=await fixture(),cp=await signRecoveryCheckpoint(f.spec,signer);
  const manifest=await migrateA2AStorage(cp,owner,context,[f.source],f.destination,signer);
  expect(manifest.manifest.entries).toHaveLength(3);
  expect(await verifyLocatorManifest(manifest,cp,owner,context)).toEqual(manifest.manifest);
  for(const entry of manifest.manifest.entries){const original=f.sourceBytes.get(f.rows.find(r=>r.attestationId===entry.artifactId)!.locator!.slice(5))!;expect(f.destinationBytes.get(entry.envelopeSha256)).toEqual(original);}
  const alternate=JSON.parse(JSON.stringify(manifest.manifest));
  alternate.entries[0].locators.unshift('blob://'+'0'.repeat(64));
  const fallback=await signLocatorManifest(alternate,cp,owner,context,signer);
  f.disableSource();const calls=f.stats().sourceFetches;
  const indexMap=new Map<string,Attestation>(),index:A2AIndexStore={list:async()=>[...indexMap.values()],put:async row=>{indexMap.set(row.attestationId,row);}};
  const freshDestination=new HttpObjectStorageAdapter('https://destination.invalid',f.destinationRequest);
  const report=await restoreMigratedA2A(fallback,cp,owner,context,[freshDestination],index);
  expect(report.completeToHeads).toBe(true);expect(report.attestations).toHaveLength(3);expect(report.source.status).toBe('disabled');expect(f.stats().sourceFetches).toBe(calls);
  const kp=new Keypair(v.reader),client=new MnemonicClient({baseUrl:'https://disabled-operator.invalid',signer:new LocalSigner(kp),a2aIndex:index,fetch:async()=>{throw new Error('operator disabled');}});client.setKeypair(kp);
  for(const row of report.attestations)expect((await client.openA2AAttestation(row,owner)).payload).toEqual(f.payloads.get(row.attestationId));
  // Local continuation retains original parent binding after backend replacement.
  const child=await client.attestA2AMessage({...v.payload,messageId:'continuation'},context,{mode:'local',prevId:f.rows[1]!.attestationId});expect(child).toMatch(/^a2a:/);
  const outsider=new Keypair(v.outsider),outsiderClient=new MnemonicClient({baseUrl:'https://disabled.invalid',signer:new LocalSigner(outsider)});outsiderClient.setKeypair(outsider);
  await expect(outsiderClient.openA2AAttestation(report.attestations[0]!,owner)).rejects.toThrow();
 });
 it('rejects missing ancestry before any destination write, and rejects forged manifest/checkpoint scope',async()=>{
  const f=await fixture();const cp=await signRecoveryCheckpoint({...f.spec,locators:f.spec.locators.slice(1)},signer);
  await expect(migrateA2AStorage(cp,owner,context,[f.source],f.destination,signer)).rejects.toThrow('ancestry');expect(f.stats().writes).toBe(0);
  const full=await signRecoveryCheckpoint(f.spec,signer),manifest=await migrateA2AStorage(full,owner,context,[f.source],f.destination,signer);
  const forged=JSON.parse(JSON.stringify(manifest));forged.manifest.entries[0].locators=['blob://'+'0'.repeat(64)];
  await expect(verifyLocatorManifest(forged,full,owner,context)).rejects.toThrow();
  await expect(verifyLocatorManifest(manifest,full,v.outsider.pubkey_base58,context)).rejects.toThrow();
  await expect(verifyLocatorManifest(manifest,full,owner,'other-context')).rejects.toThrow();
  const missing=manifest.manifest.entries.find(e=>e.artifactId===f.rows[0]!.attestationId)!;f.destinationBytes.delete(missing.envelopeSha256);
  const report=await restoreMigratedA2A(manifest,full,owner,context,[f.destination]);expect(report.completeToHeads).toBe(false);expect(report.missingParents).toContain(f.rows[0]!.attestationId);
 });
 it('rejects corrupt destination bytes, arbitrary origins and unsupported upload capability',async()=>{
  const f=await fixture();await expect(f.source.upload(new Uint8Array([1]))).rejects.toThrow('does not upload');
  expect(()=>new HttpObjectStorageAdapter('https://user:password@host.invalid')).toThrow();expect(()=>new HttpObjectStorageAdapter('https://host.invalid/other')).toThrow();
  await expect(f.destination.fetch('https://attacker.invalid/secret')).rejects.toThrow();
  const bytes=new Uint8Array([1,2,3]),digest=await envelopeSha256(bytes);f.destinationBytes.set(digest,new Uint8Array([9,9,9]));
  await expect(f.destination.fetch(`blob://${digest}`)).rejects.toThrow('digest');
  const cp=await signRecoveryCheckpoint(f.spec,signer);expect(await envelopeSha256(checkpointBytes(cp.checkpoint))).toHaveLength(64);
 });
 it('stops fetches at the aggregate byte budget even when all candidates fail verification',async()=>{
  const f=await fixture(),cp=await signRecoveryCheckpoint(f.spec,signer);
  const manifest={version:1 as const,artifactKind:'a2a' as const,scope:context,checkpointSha256:await envelopeSha256(checkpointBytes(cp.checkpoint)),createdAt:'2026-10-02T00:00:00.000Z',entries:Array.from({length:65},(_,i)=>({artifactId:`a2a:${i.toString(16).padStart(64,'0')}`,author:owner,envelopeSha256:'f'.repeat(64),locators:['blob://'+'a'.repeat(64)]}))};
  const signed=await signLocatorManifest(manifest,cp,owner,context,signer);let calls=0;const large=new Uint8Array(1048576);
  const adapter={namespace:'blob' as const,identity:'test-budget',capabilities:f.destination.capabilities,fetch:async()=>{calls++;return large;},upload:async()=>{throw new Error('not called');}};
  const report=await restoreMigratedA2A(signed,cp,owner,context,[adapter]);
  expect(calls).toBe(64);expect(report.budgetExhausted).toBe(true);expect(report.completeToHeads).toBe(false);
 });
 it('enforces deadlines when an injected fetch ignores cancellation',async()=>{
  vi.useFakeTimers();
  try{const adapter=new ArweaveStorageAdapter('https://unavailable.invalid',async()=>new Promise<Response>(()=>{}));
   const assertion=expect(adapter.fetch('ar://'+'A'.repeat(43))).rejects.toThrow('deadline');
   await vi.advanceTimersByTimeAsync(10001);await assertion;
  }finally{vi.useRealTimers();}
 });

});
