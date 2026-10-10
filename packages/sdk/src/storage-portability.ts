/** A manifest authenticates routing, not replacement signatures or availability. */
import {verifyAsync} from '@noble/ed25519';
import {jcsBytes} from './erc8004/jcs.js';
import {checkpointBytes,checkpointPublicKey,verifyRecoveryCheckpoint} from './checkpoint.js';
import type {SignedRecoveryCheckpoint,RecoveryCheckpoint} from './checkpoint.js';
import {decodeAttestation,restoreA2AContext} from './a2a-recovery.js';
import type {RecoveryHost} from './a2a-recovery.js';
import {envelopeSha256,storageAdapter,storageNamespace} from './storage.js';
import type {StorageAdapter} from './storage.js';
import type {A2AIndexStore,A2ARestoreReport,Attestation,SignerInterface} from './types.js';
import {IntegrityError,UserError} from './errors.js';
export interface LocatorManifest {
  version:1;
  checkpointSha256:string;
  artifactKind:'a2a';
  scope:string;
  createdAt:string;
  entries:{artifactId:string;author:string;envelopeSha256:string;locators:string[]}[];
}
export interface SignedLocatorManifest { manifest:LocatorManifest; signer:string; signature:string }
function keys(value:unknown,expected:string[]):void {
  if (!value || typeof value!=='object' || Array.isArray(value) || Object.keys(value).sort().join('|')!==expected.sort().join('|'))throw new UserError('invalid locator manifest fields');
}
function manifestBytes(manifest:LocatorManifest):Uint8Array {
  keys(manifest,['version','checkpointSha256','artifactKind','scope','createdAt','entries']);
  if(manifest.version!==1||manifest.artifactKind!=='a2a'||typeof manifest.scope!=='string'||!manifest.scope||manifest.scope.length>1024||!/^[0-9a-f]{64}$/.test(manifest.checkpointSha256))throw new UserError('unsupported manifest or scope');
  if(typeof manifest.createdAt!=='string'||!Number.isFinite(Date.parse(manifest.createdAt))||new Date(manifest.createdAt).toISOString()!==manifest.createdAt)throw new UserError('invalid manifest timestamp');
  if(!Array.isArray(manifest.entries)||manifest.entries.length>1000)throw new UserError('manifest exceeds 1000 artifacts');
  const ids=new Set<string>();
  for(const entry of manifest.entries){
    keys(entry,['artifactId','author','envelopeSha256','locators']);
    if(typeof entry.artifactId!=='string'||!/^a2a:[0-9a-f]{64}$/.test(entry.artifactId)||ids.has(entry.artifactId)||!/^[0-9a-f]{64}$/.test(entry.envelopeSha256))throw new UserError('invalid manifest artifact binding');
    ids.add(entry.artifactId);checkpointPublicKey(entry.author);
    if(!Array.isArray(entry.locators)||entry.locators.length<1||entry.locators.length>8||new Set(entry.locators).size!==entry.locators.length)throw new UserError('invalid manifest locators');
    entry.locators.forEach(storageNamespace);
  }
  const bytes=jcsBytes(manifest),prefix=new TextEncoder().encode('mnemonic.locator-manifest.v1\n');
  if(bytes.length>1048576)throw new UserError('manifest exceeds 1 MiB');
  const output=new Uint8Array(prefix.length+bytes.length);output.set(prefix);output.set(bytes,prefix.length);return output;
}
async function authenticateCheckpoint(signed:SignedRecoveryCheckpoint,owner:string,context:string):Promise<RecoveryCheckpoint>{
  return verifyRecoveryCheckpoint(signed,owner,{artifactKind:'a2a',scope:context});
}
export async function verifyLocatorManifest(signed:SignedLocatorManifest,checkpoint:SignedRecoveryCheckpoint,owner:string,context:string):Promise<LocatorManifest>{
  keys(signed,['manifest','signer','signature']);
  signed=JSON.parse(JSON.stringify(signed)) as SignedLocatorManifest;
  const cp=await authenticateCheckpoint(checkpoint,owner,context);
  const bytes=manifestBytes(signed.manifest);
  if(signed.signer!==owner||typeof signed.signature!=='string'||!/^[0-9a-f]{128}$/.test(signed.signature)||signed.manifest.scope!==context||signed.manifest.checkpointSha256!==await envelopeSha256(checkpointBytes(cp)))throw new IntegrityError('manifest owner/checkpoint/scope mismatch');
  if(signed.manifest.entries.some(e=>!cp.expectedAuthors.includes(e.author)))throw new IntegrityError('manifest introduced an untrusted author');
  if(!await verifyAsync(Uint8Array.from(signed.signature.match(/../g)!.map(h=>parseInt(h,16))),bytes,checkpointPublicKey(owner)))throw new IntegrityError('invalid locator manifest signature');
  return signed.manifest;
}
/** Authenticate a routing manifest without claiming its locations are available. */
export async function signLocatorManifest(manifest:LocatorManifest,checkpoint:SignedRecoveryCheckpoint,owner:string,context:string,signer:SignerInterface):Promise<SignedLocatorManifest>{
  manifest=JSON.parse(JSON.stringify(manifest)) as LocatorManifest;
  checkpoint=JSON.parse(JSON.stringify(checkpoint)) as SignedRecoveryCheckpoint;
  if(signer.pubkey!==owner)throw new UserError('manifest requires owner signer');
  const cp=await authenticateCheckpoint(checkpoint,owner,context);
  const bytes=manifestBytes(manifest);
  if(manifest.scope!==context || manifest.checkpointSha256!==await envelopeSha256(checkpointBytes(cp)) || manifest.entries.some(e=>!cp.expectedAuthors.includes(e.author)))throw new IntegrityError('manifest checkpoint/scope/author mismatch');
  const signature=await signer.sign(bytes);
  const signed={manifest,signer:owner,signature:Array.from(signature,b=>b.toString(16).padStart(2,'0')).join('')};
  await verifyLocatorManifest(signed,checkpoint,owner,context);return signed;
}
async function graph(rows:Attestation[],cp:RecoveryCheckpoint):Promise<A2ARestoreReport>{
  const index=new Map(rows.map(row=>[row.attestationId,row]));
  const store:A2AIndexStore={list:async()=>[...index.values()],put:async row=>{index.set(row.attestationId,row);}};
  const host:RecoveryHost={_a2aIndexStore:()=>store,_a2aGateway:()=>'',_a2aDiscovery:()=>({url:''}),_a2aExternal:async()=>{throw new Error('migration graph must not contact a network');}};
  return restoreA2AContext.call(host,cp.scope,{expectedAuthors:cp.expectedAuthors,heads:cp.heads,discoverySource:false});
}
class StorageBudgetError extends UserError {}
async function fetchOriginal(adapters:readonly StorageAdapter[],entry:{artifactId:string;author:string;locators:string[];envelopeSha256?:string},context:string,budget:{remaining:number}):Promise<{bytes:Uint8Array;row:Attestation}>{
  const failures:string[]=[];
  for(const locator of entry.locators){
    try{
      if(budget.remaining<=0)throw new StorageBudgetError('storage fetch budget exhausted');
      const fetched=await storageAdapter(adapters,locator).fetch(locator);
      budget.remaining-=fetched.length;
      if(budget.remaining<0)throw new StorageBudgetError('storage fetch budget exhausted');
      if(fetched.length>1048576)throw new IntegrityError('artifact exceeds 1 MiB');
      const bytes=fetched.slice();
      if(entry.envelopeSha256&&await envelopeSha256(bytes)!==entry.envelopeSha256)throw new IntegrityError('manifest envelope mismatch');
      const row=await decodeAttestation(bytes,entry.author,locator);
      if(row.attestationId!==entry.artifactId||row.contextId!==context)throw new IntegrityError('signed artifact identity or context mismatch');
      return {bytes,row};
    }catch(e){if(e instanceof StorageBudgetError)throw e;failures.push(`${locator}: ${String(e)}`);}
  }
  throw new IntegrityError(`all artifact locators failed: ${failures.join('; ')}`);
}
/** Copies only verified complete ancestry; signs a manifest after exact read-back.
 * No decryption key is read or sent. Source copies are never deleted. */
export async function migrateA2AStorage(checkpoint:SignedRecoveryCheckpoint,owner:string,context:string,sources:readonly StorageAdapter[],destination:StorageAdapter,signer:SignerInterface):Promise<SignedLocatorManifest>{
  checkpoint=JSON.parse(JSON.stringify(checkpoint)) as SignedRecoveryCheckpoint;
  const cp=await authenticateCheckpoint(checkpoint,owner,context);
  if(signer.pubkey!==owner||!destination.capabilities.upload||!destination.capabilities.fetch)throw new UserError('migration requires owner signer and read/write destination');
  const grouped=new Map<string,{artifactId:string;author:string;locators:string[]}>();
  for(const hint of cp.locators){const item=grouped.get(hint.artifactId)??{artifactId:hint.artifactId,author:hint.author,locators:[]};item.locators.push(hint.locator);grouped.set(hint.artifactId,item);}
  if(grouped.size>1000)throw new UserError('migration exceeds 1000 artifacts');
  const originals:{bytes:Uint8Array;row:Attestation}[]=[];const budget={remaining:67108864};
  for(const entry of grouped.values()){const item=await fetchOriginal(sources,entry,context,budget);originals.push(item);}
  const report=await graph(originals.map(item=>item.row),cp);
  if(!report.completeToHeads||report.attestations.length!==originals.length)throw new IntegrityError('migration requires all checkpoint ancestry; no destination writes performed');
  const entries:LocatorManifest['entries']=[];
  for(const item of originals){
    const locator=await destination.upload(item.bytes.slice());storageNamespace(locator);
    if(storageNamespace(locator)!==destination.namespace)throw new IntegrityError('destination returned wrong namespace');
    const fetched=await destination.fetch(locator);
    if(fetched.length!==item.bytes.length)throw new IntegrityError('destination changed original byte length');
    const copy=fetched.slice();
    if(copy.length!==item.bytes.length||copy.some((v,i)=>v!==item.bytes[i]))throw new IntegrityError('destination changed original bytes');
    await decodeAttestation(copy,item.row.signerPubkey! ,locator);
    entries.push({artifactId:item.row.attestationId,author:item.row.signerPubkey!,envelopeSha256:await envelopeSha256(copy),locators:[locator]});
  }
  entries.sort((a,b)=>a.artifactId.localeCompare(b.artifactId));
  const manifest:LocatorManifest={version:1,artifactKind:'a2a',scope:context,checkpointSha256:await envelopeSha256(checkpointBytes(cp)),createdAt:new Date().toISOString(),entries};
  return signLocatorManifest(manifest,checkpoint,owner,context,signer);
}
/** Original source and MCP may be offline. Copy only validated ancestry into a
 * caller index so subsequent SDK writes retain original signed parent IDs. */
export async function restoreMigratedA2A(manifest:SignedLocatorManifest,checkpoint:SignedRecoveryCheckpoint,owner:string,context:string,adapters:readonly StorageAdapter[],index?:A2AIndexStore):Promise<A2ARestoreReport>{
  checkpoint=JSON.parse(JSON.stringify(checkpoint)) as SignedRecoveryCheckpoint;
  manifest=JSON.parse(JSON.stringify(manifest)) as SignedLocatorManifest;
  const cp=await authenticateCheckpoint(checkpoint,owner,context);
  const verified=await verifyLocatorManifest(manifest,checkpoint,owner,context);
  const rows:Attestation[]=[],failures:{locator:string;reason:string}[]=[];const budget={remaining:67108864};let budgetExhausted=false;
  for(const entry of verified.entries){
    if(budget.remaining<=0){budgetExhausted=true;break;}
    try{const item=await fetchOriginal(adapters,entry,context,budget);rows.push(item.row);}
    catch(e){if(e instanceof StorageBudgetError){budgetExhausted=true;break;}failures.push({locator:entry.locators.join(','),reason:String(e)});}
  }
  const report=await graph(rows,cp);report.invalidCandidates.push(...failures);
  if(budgetExhausted){report.budgetExhausted=true;report.error='migration restore exceeded 64 MiB budget';}
  if(index)for(const row of report.attestations)await index.put(row);
  return report;
}
