import {beforeAll,afterAll,expect,it} from 'vitest';
import * as wasm from '../../../core/pkg-nodejs/mnemonic_core.js';
import {__setWasmForTesting} from '../src/wasm.js';
import {MnemonicClient} from '../src/client.js';
import {Keypair} from '../src/keypair.js';
import {LocalSigner} from '../src/signer.js';
import type {Attestation} from '../src/types.js';
import vectors from './fixtures/sealed-a2a.json';
beforeAll(()=>{globalThis.self=globalThis as any;__setWasmForTesting(wasm);});
afterAll(()=>__setWasmForTesting(null));
it('retains original bytes across wallet challenge and retries without a second signature',async()=>{
  const rows=new Map<string,Attestation>();let submitted:string|undefined;let posts=0;
  const kp=new Keypair(vectors.author);
  const fetcher:typeof fetch=async(url,init)=>{
    if(String(url).endsWith('/mcp')){
      posts++;const args=JSON.parse(String(init?.body)).params.arguments;
      if(posts===1){submitted=args.signed;return new Response(JSON.stringify({error:{data:{operation_id:'same-op',status:'awaiting_wallet_link'}}}),{status:428});}
      expect(args.signed).toBe(submitted);
      expect(new Headers(init?.headers).get('x-mnemonic-operation-id')).toBe('same-op');
      const v=JSON.parse(wasm.verify_a2a(Uint8Array.from(args.signed.match(/../g).map((x:string)=>parseInt(x,16))),vectors.author.pubkey_base58));
      return new Response(JSON.stringify({result:{attestation_id:`a2a:${v.content_hash}`,locator:`ar://${'A'.repeat(43)}`}}));
    }
    return new Response(Uint8Array.from(submitted!.match(/../g)!.map(x=>parseInt(x,16))));
  };
  const client=new MnemonicClient({baseUrl:'https://operator.test',signer:new LocalSigner(kp),jwt:'test',fetch:fetcher,a2aIndex:{list:async()=>[...rows.values()],put:async r=>{rows.set(r.attestationId,r);}}});client.setKeypair(kp);
  await expect(client.attestA2AMessage(vectors.payload as any,vectors.context_id)).rejects.toMatchObject({status:428,cause:{response:{error:{data:{operation_id:'same-op'}}}}});
  expect(rows.size).toBe(1);const original=[...rows.values()][0]!;expect(original.coseEnvelopeHex).toBe(submitted);
  const delivered=await client.retryA2ADelivery(original.attestationId,{operationId:'same-op'});
  expect(delivered.coseEnvelopeHex).toBe(submitted);expect(delivered.locator).toBe(`ar://${'A'.repeat(43)}`);expect(posts).toBe(2);
});
