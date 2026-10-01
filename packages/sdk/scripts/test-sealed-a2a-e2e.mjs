// Run through the real HTTP MCP router; Rust test supplies its local URL/JWTs.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {MnemonicClient,Keypair,LocalSigner} from '../dist/index.js';
const v=JSON.parse(readFileSync(new URL('../test/fixtures/sealed-a2a.json',import.meta.url),'utf8'));
let writes=0;
const auditedFetch=async(url,init)=>{
  if(init?.body) {
    const req=JSON.parse(init.body);
    if(req.params?.name==='mnemonic_attest_a2a') {
      writes++;
      assert(!init.body.includes('PRIVATE SEALED E2E SENTINEL'));
      assert(!init.body.includes('PRIVATE ARTIFACT NAME'));
      assert.equal(req.params.arguments.payload,undefined);
    }
  }
  return fetch(url,init);
};
const make=(json,jwt)=>{
  const kp=new Keypair(json);const c=new MnemonicClient({baseUrl:process.env.A2A_TEST_URL,signer:new LocalSigner(kp),jwt,fetch:auditedFetch});
  c.setKeypair(kp);return c;
};
const author=make(v.author,process.env.A2A_AUTHOR_JWT),reader=make(v.reader,process.env.A2A_READER_JWT),outsider=make(v.outsider,process.env.A2A_OUTSIDER_JWT);
const recipients=[{card:v.card,trustedCardSigner:v.reader.pubkey_base58}];
for(const kind of ['message','artifact']) {
  for(const stream of [false,true]) {
    const context=`real-e2e-${kind}-${stream}`;
    const inner=kind==='message'?{...v.payload,contextId:context}:{artifactId:`art-${stream}`,name:'PRIVATE ARTIFACT NAME',parts:v.payload.parts};
    const opts={sealed:{recipients,...(stream?{chunkSize:19}:{})}};
    const id=kind==='message'?await author.attestA2AMessage(inner,context,opts):await author.attestA2AArtifact(inner,context,opts);
    const rows=await reader.recallA2AContext(context,{sealed:true,kind,limit:1});
    assert.equal(rows.length,1);assert.equal(rows[0].attestationId,id);
    assert.deepEqual(await reader.openA2AAttestation(rows[0],v.author.pubkey_base58),inner);
    const own=await author.recallA2AContext(context,{sealed:true});
    assert.deepEqual(await author.openA2AAttestation(own[0],v.author.pubkey_base58),inner);
    assert.equal((await outsider.recallA2AContext(context,{sealed:true})).length,0);
    await assert.rejects(()=>outsider.openA2AAttestation(rows[0],v.author.pubkey_base58));
  }
}
assert.equal(writes,4);
console.log('SDK → HTTP MCP → sealed SQLite → recipient recall → client WASM open: 4 scenarios passed');
