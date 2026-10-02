import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {MnemonicClient,Keypair,LocalSigner} from '../dist/index.js';
const v=JSON.parse(readFileSync(new URL('../test/fixtures/sealed-a2a.json',import.meta.url),'utf8'));
const remote=process.env.A2A_GATEWAY_URL;
const make=(identity,jwt,offline=false)=>{const kp=new Keypair(identity);const c=new MnemonicClient({baseUrl:process.env.A2A_TEST_URL,signer:new LocalSigner(kp),...(jwt?{jwt}:{}),a2aGatewayUrl:remote,a2aIndexUrl:`${remote}/graphql`,fetch:async(url,init)=>{if(offline)assert(!String(url).startsWith(process.env.A2A_TEST_URL),'restore contacted MCP');if(String(url).startsWith(remote))assert(!new Headers(init?.headers).has('authorization'));return fetch(url,init);}});c.setKeypair(kp);return c;};
const context='sql-loss-sealed-chain';const recipients=[{card:v.card,trustedCardSigner:v.reader.pubkey_base58}];
if(process.argv[2]==='append'){
 const expected=JSON.parse(process.env.A2A_EXPECTED);const c=make(v.author,process.env.A2A_AUTHOR_JWT);const r=await c.restoreA2AContext(context,{expectedAuthors:[v.author.pubkey_base58],heads:[expected.head]});assert.equal(r.completeToHeads,true);await c.attestA2AMessage({...v.payload,messageId:'after-sql-loss',contextId:context},context,{prevId:expected.head,sealed:{recipients}});console.log('chain continuation with empty MCP receipts passed');
}else if(process.argv[2]==='restore'){
 const expected=JSON.parse(process.env.A2A_EXPECTED);
 for(const identity of [v.author,v.reader]){const client=make(identity,undefined,true);const report=await client.restoreA2AContext(context,{expectedAuthors:[v.author.pubkey_base58],heads:[expected.head]});assert.equal(report.error,undefined);assert.equal(report.completeToHeads,true);assert.equal(report.attestations.length,3);for(const row of report.attestations)assert.deepEqual(await client.openA2AAttestation(row,v.author.pubkey_base58),expected.payloads[row.attestationId]);}
 const outsider=make(v.outsider,undefined,true);const report=await outsider.restoreA2AContext(context,{expectedAuthors:[v.author.pubkey_base58],heads:[expected.head]});assert.equal(report.completeToHeads,true);await assert.rejects(()=>outsider.openA2AAttestation(report.attestations[0],v.author.pubkey_base58));console.log('SQL-loss recovery and local open passed');
}else{
 const client=make(v.author,process.env.A2A_AUTHOR_JWT);const payloads={};let previous;
 for(let i=0;i<3;i++){const payload={...v.payload,messageId:`chain-${i}`,contextId:context};const id=await client.attestA2AMessage(payload,context,{sealed:{recipients,...(i===2?{chunkSize:19}:{})},...(previous?{prevId:previous}:{})});payloads[id]=payload;previous=id;}
 const directory=await make(v.author,process.env.A2A_AUTHOR_JWT).recallA2AContext(context,{sealed:true});assert.equal(directory.length,3);
 console.log(JSON.stringify({head:previous,payloads}));
}
