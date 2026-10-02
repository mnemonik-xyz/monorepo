// Invoked by the Rust HTTP integration test. Uses the built SDK and real WASM.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {
  MnemonicClient, Keypair, LocalSigner, signRecoveryCheckpoint,
  ArweaveStorageAdapter, HttpObjectStorageAdapter, migrateA2AStorage,
  restoreMigratedA2A, verifyLocatorManifest,
} from '../dist/index.js';
const v=JSON.parse(readFileSync(new URL('../test/fixtures/sealed-a2a.json',import.meta.url),'utf8'));
const source=process.env.A2A_SOURCE_URL, destination=process.env.A2A_DESTINATION_URL;
if(process.argv[2]==='dedup'){
  const expected=JSON.parse(process.env.A2A_DEDUP_PACKET);
  const kp=new Keypair(v.reader);
  const c=new MnemonicClient({baseUrl:'http://127.0.0.1:1',signer:new LocalSigner(kp),a2aGatewayUrl:source,a2aIndexUrl:`${source}/graphql`,fetch:async(url,init)=>{
    assert(String(url).startsWith(source),'dedup contacted an operator');
    assert(!new Headers(init?.headers).has('authorization'));
    return fetch(url,init);
  }});
  const report=await c.restoreA2AContext(v.context_id,{expectedAuthors:[v.author.pubkey_base58],heads:[expected.id]});
  assert.equal(report.source.candidates,2);assert.equal(report.invalidCandidates.length,0);
  assert.equal(report.attestations.length,1);assert.equal(report.completeToHeads,true);
  assert.equal(report.attestations[0].attestationId,expected.id);
  assert.equal(report.attestations[0].coseEnvelopeHex,v.plain);
  console.log(JSON.stringify({verifiedCandidates:report.source.candidates,artifacts:report.attestations.length}));
  process.exit(0);
}
const context='sdk-migrated-sealed-stream';
const owner=v.author.pubkey_base58;
const fresh=()=>{const rows=new Map();return {rows,index:{list:async()=>[...rows.values()],put:async row=>{rows.set(row.attestationId,row);}}};};
const request=async(url,init)=>{
  const target=String(url);
  if(process.argv[2]==='restore'){
    assert(!target.startsWith(source),'recovery contacted deleted source');
    assert(!target.startsWith(process.env.A2A_ORIGINAL_OPERATOR),'recovery contacted original operator');
  }
  if(target.startsWith(source)||target.startsWith(destination))assert(!new Headers(init?.headers).has('authorization'),'operator JWT leaked to storage');
  return fetch(url,init);
};
const client=(identity,store,jwt)=>{
  const kp=new Keypair(identity);
  const c=new MnemonicClient({baseUrl:process.env.A2A_OPERATOR_URL,signer:new LocalSigner(kp),a2aIndex:store.index,a2aGatewayUrl:process.argv[2]==='restore'?destination:source,fetch:request,...(jwt?{jwt}:{})});
  c.setKeypair(kp);return c;
};
const recipients=[{card:v.card,trustedCardSigner:v.reader.pubkey_base58}];
if(process.argv[2]!=='restore'){
  const store=fresh(),writer=client(v.author,store,process.env.A2A_JWT),payloads={};let head;
  for(let i=0;i<3;i++){
    const payload={...v.payload,messageId:`migration-${i}`,contextId:context};
    head=await writer.attestA2AMessage(payload,context,{sealed:{recipients,...(i===2?{chunkSize:19}:{})},...(head?{prevId:head}:{})});
    payloads[head]=payload;
  }
  const rows=[...store.rows.values()];
  assert.equal(rows.length,3);
  assert(rows.every(row=>row.sealedPayload.grants.length>0));
  assert(rows.find(row=>row.attestationId===head).stream.chunks.length>1);
  const checkpoint=await signRecoveryCheckpoint({version:1,artifactKind:'a2a',scope:context,expectedAuthors:[owner],heads:[head],locators:rows.map(row=>({artifactId:row.attestationId,author:owner,locator:row.locator})),backendHints:['arweave'],createdAt:new Date().toISOString()},new LocalSigner(new Keypair(v.author)));
  const manifest=await migrateA2AStorage(checkpoint,owner,context,[new ArweaveStorageAdapter(source,request)],new HttpObjectStorageAdapter(destination,request),new LocalSigner(new Keypair(v.author)));
  await verifyLocatorManifest(manifest,checkpoint,owner,context);
  console.log(JSON.stringify({checkpoint,manifest,payloads,originals:Object.fromEntries(rows.map(row=>[row.attestationId,row.coseEnvelopeHex]))}));
}else{
  const saved=JSON.parse(process.env.A2A_RECOVERY_PACKET);
  // New Node runtime, empty index and recipient identity; no author secret is used.
  const store=fresh(),reader=client(v.reader,store,process.env.A2A_JWT);
  const report=await restoreMigratedA2A(saved.manifest,saved.checkpoint,owner,context,[new HttpObjectStorageAdapter(destination,request)],store.index);
  assert.equal(report.completeToHeads,true);assert.equal(report.source.status,'disabled');assert.equal(report.attestations.length,3);
  for(const row of report.attestations){
    assert.equal(row.coseEnvelopeHex,saved.originals[row.attestationId]);
    assert.deepEqual(await reader.openA2AAttestation(row,owner),saved.payloads[row.attestationId]);
    assert(row.sealedPayload.grants.length>0);
  }
  const parent=saved.checkpoint.checkpoint.heads[0],parentRow=store.rows.get(parent);
  assert(parentRow.stream.chunks.length>1);assert(parentRow.locator.startsWith('blob://'));
  const outsider=client(v.outsider,fresh());
  await assert.rejects(()=>outsider.openA2AAttestation(parentRow,owner));
  const child=await reader.attestA2AMessage({...v.payload,messageId:'recipient-continuation',contextId:context},context,{prevId:parent,sealed:{recipients}});
  const row=store.rows.get(child);
  assert.equal(row.prevId,parent);assert(row.locator.startsWith('ar://'));
  console.log(JSON.stringify({child,parent,parentLocator:parentRow.locator,locator:row.locator,original:row.coseEnvelopeHex,restored:report.attestations.length,completeToHeads:report.completeToHeads}));
}
