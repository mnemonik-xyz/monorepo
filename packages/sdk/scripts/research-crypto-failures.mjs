// Synthetic attack controls. Cryptography always uses the built SDK and real WASM.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {MnemonicClient,Keypair,LocalSigner,verifyA2AAttestation} from '../dist/index.js';
import {decodeAttestation} from '../dist/a2a-recovery.js';
import {loadWasm} from '../dist/wasm.js';
const bytes=hex=>new Uint8Array(Buffer.from(hex,'hex'));
const hex=value=>Buffer.from(value).toString('hex');
const utf8=value=>new TextEncoder().encode(value);
const candidateId='A'.repeat(43);
function isolated(identity,fetch){
  const kp=identity instanceof Keypair?identity:new Keypair(identity),rows=new Map();
  const client=new MnemonicClient({baseUrl:'https://disabled-operator.invalid',signer:new LocalSigner(kp),fetch,
    a2aGatewayUrl:'https://crypto-fixture.invalid',a2aIndexUrl:'https://crypto-fixture.invalid/graphql',
    a2aIndex:{list:async()=>[...rows.values()],put:async row=>{rows.set(row.attestationId,row);}}});
  client.setKeypair(kp);return {client,rows};
}
export async function runResearchCryptoFailures({streamRow,readerIdentity,authorIdentity,outsiderIdentity,outsiderFetchClient}){
  const original=bytes(streamRow.coseEnvelopeHex),author=authorIdentity.pubkey_base58,context=streamRow.contextId;
  assert.equal(streamRow.signerPubkey,author);assert(streamRow.stream.chunks.length>1);
  const reader=isolated(readerIdentity,async()=>{throw new Error('local crypto control attempted HTTP');}).client;
  await reader.openA2AAttestation(streamRow,author); // Baseline proves correct recipient/key/fixture.
  const changed=original.slice();changed[changed.length-1]^=1;
  const invalid=isolated(readerIdentity,async(url,init)=>{
    assert.equal(String(url),`https://crypto-fixture.invalid/${candidateId}`);
    assert(!new Headers(init?.headers).has('authorization'));
    return new Response(changed);
  });
  await assert.rejects(()=>invalid.client.importA2AAttestation(`ar://${candidateId}`,author),/A2A public verification failed/);
  assert.equal(invalid.rows.size,0);

  // Privileged test re-signing deliberately bypasses the outer-signature barrier
  // to isolate AEAD. It never uploads or claims an attacker can forge A's key.
  const verified=await verifyA2AAttestation(streamRow.coseEnvelopeHex,author);
  const binding=structuredClone(verified.binding),stream=binding.stream;
  const ciphertext=Buffer.from(stream.chunks[0].ciphertext,'base64');ciphertext[0]^=1;
  stream.chunks[0].ciphertext=ciphertext.toString('base64');
  const wasm=await loadWasm();let previous='00'.repeat(32);
  for(const chunk of stream.chunks){
    chunk.prev_hash=previous;
    chunk.hash=wasm.blake3_hash_hex(utf8(JSON.stringify([chunk.index,chunk.last,chunk.prev_hash,chunk.ciphertext])));
    previous=chunk.hash;
  }
  stream.head=previous;
  // Fixture JSON uses only integer/string/boolean/null values. Sort all object
  // keys per existing A2A JCS before invoking the existing COSE signing binding.
  const canonical=JSON.stringify(binding,(_key,value)=>value&&typeof value==='object'&&!Array.isArray(value)
    ?Object.fromEntries(Object.keys(value).sort().map(key=>[key,value[key]])):value);
  const repaired=wasm.sign_cose_payload(utf8(canonical),authorIdentity);
  const repairedRow=await decodeAttestation(repaired,author);
  assert.notEqual(repairedRow.contentHash,streamRow.contentHash);
  assert.equal(repairedRow.stream.head,stream.head); // Public chain + signature verified.
  await assert.rejects(()=>reader.openA2AAttestation(repairedRow,author),error=>{
    assert.match(String(error.cause),/content decryption failed/);return true;
  });

  for(const variant of ['author','context']){
    const trustedOther=readerIdentity instanceof Keypair?readerIdentity.pubkey:readerIdentity.pubkey_base58;
    const requestedContext=variant==='context'?`${context}-forged`:context;
    const claimedAuthor=variant==='author'?trustedOther:author;
    let artifactFetches=0;
    const forged=isolated(readerIdentity,async(url,init)=>{
      assert(!new Headers(init?.headers).has('authorization'));
      if(String(url)==='https://crypto-fixture.invalid/graphql'){
        const tags={'App-Name':'mnemonic-protocol','Mnemonic-Type':'a2a','Producer':claimedAuthor,'Context-Id':requestedContext,'Content-Hash':streamRow.contentHash};
        return Response.json({data:{transactions:{edges:[{cursor:candidateId,node:{id:candidateId,tags:Object.entries(tags).map(([name,value])=>({name,value}))}}],pageInfo:{hasNextPage:false}}}});
      }
      assert.equal(String(url),`https://crypto-fixture.invalid/${candidateId}`);artifactFetches++;return new Response(original);
    });
    const report=await forged.client.restoreA2AContext(requestedContext,{expectedAuthors:[author,trustedOther],heads:[streamRow.attestationId]});
    assert.equal(artifactFetches,1,'forged metadata must be checked against fetched signed bytes');
    assert.equal(report.attestations.length,0);assert.equal(report.invalidCandidates.length,1);
    assert.match(report.invalidCandidates[0].reason,variant==='author'?/signature does not match pinned authors/:/signed context mismatch/);
    assert.equal(report.completeToHeads,false);assert.equal(forged.rows.size,0);
  }
  const outsider=outsiderFetchClient??isolated(outsiderIdentity,async(url,init)=>{
    assert.equal(String(url),`https://crypto-fixture.invalid/${candidateId}`);
    assert(!new Headers(init?.headers).has('authorization'));return new Response(original);
  }).client;
  const fetched=await outsider.importA2AAttestation(outsiderFetchClient?streamRow.locator:`ar://${candidateId}`,author);
  assert.equal(fetched.coseEnvelopeHex,streamRow.coseEnvelopeHex);
  await assert.rejects(()=>outsider.openA2AAttestation(fetched,author),error=>{
    assert.match(String(error.cause),/not a recipient or invalid wrapped key/);return true;
  });
  return [
    {name:'changed_artifact_bytes',status:'rejected',detail:'Changed signed-envelope bytes fail real SDK verification; no artifact enters the local index.'},
    {name:'ciphertext_repaired_public_hashes',status:'rejected',detail:'Ciphertext changed, all public stream hashes/head repaired, and fixture-author transport re-signed. Public verification passes; authenticated local decryption rejects. No malicious envelope uploaded.'},
    {name:'forged_index_tags',status:'rejected',detail:'Forged author and context hints each pass metadata scoping but fail against fetched original signed bytes; neither candidate is imported.'},
    {name:'outsider_fetch_open',status:'rejected',detail:'Outsider fetches and verifies exact public ciphertext, but real SDK/WASM denies plaintext opening.'},
  ];
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
  const fixture=JSON.parse(readFileSync(new URL('../test/fixtures/sealed-a2a.json',import.meta.url),'utf8'));
  const row=await decodeAttestation(bytes(fixture.stream),fixture.author.pubkey_base58,`ar://${candidateId}`);
  const evidence=await runResearchCryptoFailures({streamRow:row,readerIdentity:fixture.reader,authorIdentity:fixture.author,outsiderIdentity:fixture.outsider});
  console.log(JSON.stringify({version:1,environment:'local_mock_services_real_sdk_wasm',failures:evidence},null,2));
}
