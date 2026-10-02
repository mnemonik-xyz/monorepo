import {beforeAll,afterAll,describe,it,expect} from 'vitest';
import * as wasm from '../../../core/pkg-nodejs/mnemonic_core.js';
import {__setWasmForTesting} from '../src/wasm.js';
import {Keypair} from '../src/keypair.js';
import {LocalSigner} from '../src/signer.js';
import {MnemonicClient} from '../src/client.js';
import {decodeAttestation} from '../src/a2a-recovery.js';
import {checkpointBytes,signRecoveryCheckpoint,verifyRecoveryCheckpoint,checkpointA2ARestoreOptions} from '../src/checkpoint.js';
import type {RecoveryCheckpoint} from '../src/checkpoint.js';
import {createRecoveryBackup,openRecoveryBackup} from '../src/backup.js';
import v from './fixtures/sealed-a2a.json';
const kp=new Keypair(v.author),signer=new LocalSigner(kp), locator=`ar://${'A'.repeat(43)}`;
const signedBytes=Uint8Array.from(v.stream.match(/../g)!.map(x=>parseInt(x,16)));
const binding=JSON.parse(wasm.verify_a2a(signedBytes,v.author.pubkey_base58));
const id=`a2a:${binding.content_hash}`;
const spec=():RecoveryCheckpoint=>({version:1,artifactKind:'a2a',scope:v.context_id,expectedAuthors:[v.author.pubkey_base58],heads:[id],locators:[{artifactId:id,author:v.author.pubkey_base58,locator}],backendHints:['irys'],createdAt:'2026-10-02T00:00:00.000Z'});
beforeAll(()=>{globalThis.self=globalThis as any;__setWasmForTesting(wasm);});
afterAll(()=>__setWasmForTesting(null));
describe('authenticated recovery checkpoints',()=>{
 it('canonicalizes object keys and binds owner, kind, context, heads and locators',async()=>{
  const cp=spec();expect(checkpointBytes(cp)).toEqual(checkpointBytes(Object.fromEntries(Object.entries(cp).reverse()) as unknown as RecoveryCheckpoint));
  const signed=await signRecoveryCheckpoint(cp,signer);
  expect(await verifyRecoveryCheckpoint(signed,kp.pubkey,{artifactKind:'a2a',scope:cp.scope})).toEqual(cp);
  for(const changed of [{...signed,checkpoint:{...cp,heads:['forged']}},{...signed,checkpoint:{...cp,locators:[]}},{...signed,signer:v.outsider.pubkey_base58}]) await expect(verifyRecoveryCheckpoint(changed,kp.pubkey,{artifactKind:'a2a',scope:cp.scope})).rejects.toThrow();
  await expect(verifyRecoveryCheckpoint(signed,v.outsider.pubkey_base58,{artifactKind:'a2a',scope:cp.scope})).rejects.toThrow();
  await expect(verifyRecoveryCheckpoint(signed,kp.pubkey,{artifactKind:'memory',scope:cp.scope})).rejects.toThrow();
  await expect(verifyRecoveryCheckpoint(signed,kp.pubkey,{artifactKind:'a2a',scope:'other'})).rejects.toThrow();
 });
 it('verification returns only the snapshot authenticated before asynchronous crypto',async()=>{
  const signed=await signRecoveryCheckpoint(spec(),signer);
  const verification=verifyRecoveryCheckpoint(signed,kp.pubkey,{artifactKind:'a2a',scope:v.context_id});
  signed.checkpoint.heads=['forged-during-await'];
  expect((await verification).heads).toEqual([id]);
 });
 it('rejects unknown versions, oversized scopes, untrusted locator authors and nonfinite time',()=>{
  for(const changes of [{version:2},{scope:'x'.repeat(1025)},{createdAt:'invalid'},{heads:['same','same']},{locators:[{artifactId:id,author:v.outsider.pubkey_base58,locator}]}])expect(()=>checkpointBytes({...spec(),...changes} as RecoveryCheckpoint)).toThrow();
 });
 it('fresh client restores authenticated pinned head and full plaintext from encrypted key backup with index unavailable',async()=>{
  const signed=await signRecoveryCheckpoint(spec(),signer);
  const backup=await createRecoveryBackup(signed,kp,'independently-kept-long-passphrase');
  expect(JSON.stringify(backup)).not.toContain(v.author.pubkey_base58);
  const restored=await openRecoveryBackup(backup,'independently-kept-long-passphrase',kp.pubkey,{artifactKind:'a2a',scope:v.context_id});
  const client=new MnemonicClient({baseUrl:'https://mcp.invalid',a2aGatewayUrl:'https://gateway.invalid',a2aIndexUrl:'https://index.invalid',signer:new LocalSigner(restored.identity),fetch:async(url)=>{expect(String(url)).not.toContain('mcp.invalid');if(String(url).includes('index.invalid'))throw new Error('index unavailable');return new Response(signedBytes);}});
  client.setKeypair(restored.identity);
  const report=await client.restoreA2AContext(v.context_id,await checkpointA2ARestoreOptions(restored.signedCheckpoint,kp.pubkey,v.context_id));
  expect(report.completeToHeads).toBe(true);expect(report.error).toContain('unavailable');
  expect(await client.openA2AAttestation(report.attestations[0]!,kp.pubkey)).toEqual(v.payload);
  await expect(openRecoveryBackup(backup,'wrong-but-long-passphrase',kp.pubkey,{artifactKind:'a2a',scope:v.context_id})).rejects.toThrow();
  await expect(openRecoveryBackup({...backup,ciphertext:backup.ciphertext.slice(0,-2)+(parseInt(backup.ciphertext.slice(-2),16)^1).toString(16).padStart(2,'0')},'independently-kept-long-passphrase',kp.pubkey,{artifactKind:'a2a',scope:v.context_id})).rejects.toThrow();
  await expect(openRecoveryBackup(backup,'independently-kept-long-passphrase',v.outsider.pubkey_base58,{artifactKind:'a2a',scope:v.context_id})).rejects.toThrow();
 });
 it('refuses silent locator loss in the A2A adapter and preserves explicit encryption-key backups',async()=>{
  const cp=spec();cp.locators.push({...cp.locators[0]!,locator:`ar://${'B'.repeat(43)}`});
  const signed=await signRecoveryCheckpoint(cp,signer);
  await expect(checkpointA2ARestoreOptions(signed,kp.pubkey,v.context_id)).rejects.toThrow('one selected locator');
  const backup=await createRecoveryBackup(signed,kp,'another-strong-passphrase',{reader:'ab'.repeat(32)});
  const restored=await openRecoveryBackup(backup,'another-strong-passphrase',kp.pubkey,{artifactKind:'a2a',scope:v.context_id});
  expect(restored.encryptionKeys).toEqual({reader:'ab'.repeat(32)});
 });
 it('requires content digest bindings for memory checkpoints because IDs alone are not content addressed',()=>{
  expect(()=>checkpointBytes({...spec(),artifactKind:'memory'})).toThrow();
  const cp={...spec(),artifactKind:'memory' as const,locators:spec().locators.map(entry=>({...entry,envelopeDigest:'ab'.repeat(32)}))};
  expect(()=>checkpointBytes(cp)).not.toThrow();
  expect(()=>checkpointBytes({...cp,heads:['missing-digest-pin']})).toThrow('digest-bound');
 });
 it('no-head checkpoint makes no completeness promise and does not recover lost decryption keys',async()=>{
  const signed=await signRecoveryCheckpoint({...spec(),heads:[]},signer);
  expect((await checkpointA2ARestoreOptions(signed,kp.pubkey,v.context_id)).heads).toEqual([]);
  const outsider=new MnemonicClient({baseUrl:'https://mcp.invalid',signer:new LocalSigner(new Keypair(v.outsider))});outsider.setKeypair(new Keypair(v.outsider));
  const row=await decodeAttestation(signedBytes,kp.pubkey,locator);
  await expect(outsider.openA2AAttestation(row,kp.pubkey)).rejects.toThrow();
 });
});
