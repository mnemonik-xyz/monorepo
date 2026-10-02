// Invoked by the Rust HTTP integration test. Uses the built SDK and real WASM.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runResearchCryptoFailures} from './research-crypto-failures.mjs';
import {runResearchDiscoveryFailures} from './research-discovery-failures.mjs';
import {
  MnemonicClient, Keypair, LocalSigner, signRecoveryCheckpoint,
  ArweaveStorageAdapter, HttpObjectStorageAdapter, migrateA2AStorage,
  restoreMigratedA2A, verifyLocatorManifest, createRecoveryBackup, openRecoveryBackup, envelopeSha256,
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
const context='research-handoff-demo';
const collector=v.author.pubkey_base58, owner=v.reader.pubkey_base58;
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
  const kp=identity instanceof Keypair?identity:new Keypair(identity);
  const c=new MnemonicClient({baseUrl:process.env.A2A_OPERATOR_URL,signer:new LocalSigner(kp),a2aIndex:store.index,a2aGatewayUrl:process.argv[2]==='restore'?destination:source,a2aIndexUrl:`${source}/graphql`,fetch:request,...(jwt?{jwt}:{})});
  c.setKeypair(kp);return c;
};
const recipients=[{card:v.card,trustedCardSigner:v.reader.pubkey_base58}];
const rowEvidence=async(label,row,opened)=>({
  label,artifactId:row.attestationId,author:row.signerPubkey,prevId:row.prevId??null,
  locator:row.locator,originalSha256:await envelopeSha256(Uint8Array.from(Buffer.from(row.coseEnvelopeHex,'hex'))),
  recipientOpened:opened,streamChunks:row.stream?.chunks.length??0,grants:row.sealedPayload?.grants.length??0,
});
if(process.argv[2]!=='restore'){
  const aStore=fresh(),writer=client(v.author,aStore,process.env.A2A_JWT),payloads={},labels={};
  let head;
  const research={R:'Research question: compare two synthetic battery designs. Scope: fabricated lab notes only.',S:'Synthetic source bundle: design alpha has 100 cycles; design beta has 120 cycles. These invented figures demonstrate transport, not scientific evidence.'};
  for(const label of ['R','S']){
    const payload={...v.payload,messageId:label,contextId:context,parts:[{kind:'text',text:research[label]}]};
    head=await writer.attestA2AMessage(payload,context,{sealed:{recipients,...(label==='S'?{chunkSize:19}:{})},...(head?{prevId:head}:{})});
    payloads[head]=payload;labels[head]=label;
  }
  // B has a separate empty index and discovers/verifies A's artifacts externally.
  const bStore=fresh(),reviewer=client(v.reader,bStore,process.env.A2A_READER_JWT);
  const handoff=await reviewer.restoreA2AContext(context,{expectedAuthors:[collector],heads:[head]});
  assert.equal(handoff.completeToHeads,true);assert.equal(handoff.attestations.length,2);
  for(const row of handoff.attestations)assert.deepEqual(await reviewer.openA2AAttestation(row,collector),payloads[row.attestationId]);
  const review={...v.payload,messageId:'V',contextId:context,parts:[{kind:'text',text:'Review: beta exceeds alpha in the supplied synthetic notes. Uncertainty: invented data and no independent experiment. Next: request a replication.'}]};
  head=await reviewer.attestA2AMessage(review,context,{prevId:head,sealed:{recipients}});
  payloads[head]=review;labels[head]='V';
  const rows=[...bStore.rows.values()];
  assert.equal(rows.length,3);assert(rows.every(row=>row.sealedPayload.grants.length>0));
  assert(rows.find(row=>labels[row.attestationId]==='S').stream.chunks.length>1);
  assert.equal(rows.find(row=>row.attestationId===head).signerPubkey,owner);
  const cryptoFailures=await runResearchCryptoFailures({streamRow:rows.find(row=>labels[row.attestationId]==='S'),readerIdentity:new Keypair(v.reader),authorIdentity:v.author,outsiderIdentity:v.outsider,outsiderFetchClient:client(v.outsider,fresh())});
  const checkpoint=await signRecoveryCheckpoint({version:1,artifactKind:'a2a',scope:context,expectedAuthors:[collector,owner],heads:[head],locators:rows.map(row=>({artifactId:row.attestationId,author:row.signerPubkey,locator:row.locator})),backendHints:['arweave'],createdAt:new Date().toISOString()},new LocalSigner(new Keypair(v.reader)));
  const manifest=await migrateA2AStorage(checkpoint,owner,context,[new ArweaveStorageAdapter(source,request)],new HttpObjectStorageAdapter(destination,request),new LocalSigner(new Keypair(v.reader)));
  await verifyLocatorManifest(manifest,checkpoint,owner,context);
  const backup=await createRecoveryBackup(checkpoint,new Keypair(v.reader),process.env.A2A_BACKUP_PASSPHRASE);
  console.log(JSON.stringify({checkpoint,manifest,backup,payloads,labels,cryptoFailures,authors:Object.fromEntries(rows.map(row=>[row.attestationId,row.signerPubkey])),originals:Object.fromEntries(rows.map(row=>[row.attestationId,row.coseEnvelopeHex]))}));
}else{
  const saved=JSON.parse(process.env.A2A_RECOVERY_PACKET);
  // A new Node runtime reconstructs B from an authenticated encrypted backup.
  const restored=await openRecoveryBackup(saved.backup,process.env.A2A_BACKUP_PASSPHRASE,owner,{artifactKind:'a2a',scope:context});
  assert.equal(restored.identity.pubkey,owner);assert.deepEqual(restored.signedCheckpoint,saved.checkpoint);
  const store=fresh(),reader=client(restored.identity,store,process.env.A2A_JWT);
  const report=await restoreMigratedA2A(saved.manifest,restored.signedCheckpoint,owner,context,[new HttpObjectStorageAdapter(destination,request)],store.index);
  assert.equal(report.completeToHeads,true);assert.equal(report.source.status,'disabled');assert.equal(report.attestations.length,3);
  const artifacts=[];
  for(const row of report.attestations){
    assert.equal(row.coseEnvelopeHex,saved.originals[row.attestationId]);
    assert.equal(row.signerPubkey,saved.authors[row.attestationId]);
    assert.deepEqual(await reader.openA2AAttestation(row,saved.authors[row.attestationId]),saved.payloads[row.attestationId]);
    assert(row.sealedPayload.grants.length>0);
    artifacts.push(await rowEvidence(saved.labels[row.attestationId],row,true));
  }
  artifacts.sort((a,b)=>a.label.localeCompare(b.label));
  assert.deepEqual(artifacts.map(row=>row.label),['R','S','V']);
  assert.equal(artifacts[0].author,collector);assert.equal(artifacts[1].author,collector);assert.equal(artifacts[2].author,owner);
  assert.equal(artifacts[0].prevId,null);assert.equal(artifacts[1].prevId,artifacts[0].artifactId);assert.equal(artifacts[2].prevId,artifacts[1].artifactId);
  assert(artifacts[1].streamChunks>1);
  const parent=restored.checkpoint.heads[0],parentRow=store.rows.get(parent);
  assert.equal(saved.labels[parent],'V');assert(parentRow.locator.startsWith('blob://'));
  const discoveryFailures=await runResearchDiscoveryFailures({context,rows:report.attestations,identity:restored.identity,head:parent,expectedAuthors:[collector,owner]});
  const outsider=client(v.outsider,fresh());
  await assert.rejects(()=>outsider.openA2AAttestation(store.rows.get(artifacts[1].artifactId),collector));
  const followup={...v.payload,messageId:'W',contextId:context,parts:[{kind:'text',text:'Follow-up after restoring reviewer B: request a replicated measurement; preserve the uncertainty from V.'}]};
  const child=await reader.attestA2AMessage(followup,context,{prevId:parent,sealed:{recipients}});
  const row=store.rows.get(child);
  assert.equal(row.prevId,parent);assert.equal(row.signerPubkey,owner);assert(row.locator.startsWith('ar://'));
  assert.deepEqual(await reader.openA2AAttestation(row,owner),followup);
  artifacts.push(await rowEvidence('W',row,true));
  const evidence={version:1,environment:'local_mock_services_real_sdk_wasm',artifacts,recoveredHeads:[parent],
    phases:[{name:'collector_authored_R_S',status:'passed'},{name:'reviewer_discovered_opened_and_authored_V',status:'passed'},{name:'exact_byte_storage_migration',status:'passed'},{name:'fresh_B_encrypted_identity_restore',status:'passed'},{name:'recipient_recovered_R_S_V',status:'passed'},{name:'W_delivered_through_O2',status:'passed'}],
    checks:{checkpointAuthenticated:true,manifestAuthenticated:true,exactOriginalBytes:true,completeToHeads:true,restoredIdentity:true,privateKeysPublished:false},
    failures:[...saved.cryptoFailures,...discoveryFailures,{name:'outsider_open_S',status:'rejected',detail:'Real SDK/WASM rejects outsider access to the recovered sealed source stream.'}],
    limitations:['Synthetic local storage and operators; no live-provider durability claim.','B uses its restored identity; this does not provision a new independently trusted identity.','This unpaid handoff excludes financial controls; the combined runner records them separately. Live task5 prerequisites remain open.']};
  console.log(JSON.stringify({child,parent,parentLocator:parentRow.locator,locator:row.locator,original:row.coseEnvelopeHex,restored:report.attestations.length,completeToHeads:report.completeToHeads,evidence}));
}
