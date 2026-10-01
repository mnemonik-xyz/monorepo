// Real WASM and native Rust fixtures; no mock cryptography.
import { beforeAll, afterAll, describe, expect, it } from "vitest";
import { __setWasmForTesting } from "../src/wasm.js";
import * as wasm from "../../../core/pkg-nodejs/mnemonic_core.js";
import vectors from "./fixtures/sealed-a2a.json";
import { MnemonicClient } from "../src/client.js";
import { Keypair } from "../src/keypair.js";
import { verifyA2AAttestation } from "../src/a2a.js";
import { LocalSigner } from "../src/signer.js";

const hex = (s:string)=>Uint8Array.from(s.match(/../g)!.map(b=>parseInt(b,16)));
const bytesHex = (b:Uint8Array)=>Array.from(b,n=>n.toString(16).padStart(2,"0")).join("");
beforeAll(()=> { globalThis.self=globalThis as any; __setWasmForTesting(wasm); });
afterAll(()=>__setWasmForTesting(null));

function client(json: typeof vectors.author, fetchImpl?:typeof fetch) {
  const kp=new Keypair(json);
  const c=new MnemonicClient({baseUrl:"https://test.invalid",jwt:"test",signer:new LocalSigner(kp),...(fetchImpl?{fetch:fetchImpl}:{})});
  c.setKeypair(kp);return c;
}

describe("sealed A2A real WASM",()=>{
  it("matches native deterministic COSE bytes for plain A2A",()=>{
    const actual=wasm.prepare_a2a(vectors.author,"message",JSON.stringify(vectors.payload),vectors.context_id,undefined,vectors.created_at,undefined,undefined);
    expect(bytesHex(actual)).toBe(vectors.plain);
  });
  it.each(["sealed","stream"] as const)("opens native %s fixture for sender and recipient only",(key)=>{
    const bytes=hex(vectors[key]);
    expect(bytesHex(wasm.sign_cose_payload(hex(vectors[`${key}_jcs`]),vectors.author))).toBe(vectors[key]);
    expect(JSON.parse(wasm.open_a2a(vectors.reader,bytes,vectors.author.pubkey_base58,undefined))).toEqual(vectors.payload);
    expect(JSON.parse(wasm.open_a2a(vectors.author,bytes,vectors.author.pubkey_base58,undefined))).toEqual(vectors.payload);
    expect(()=>wasm.open_a2a(vectors.outsider,bytes,vectors.author.pubkey_base58,undefined)).toThrow();
    const copy=bytes.slice();copy[copy.length-1]^=1;
    expect(()=>wasm.open_a2a(vectors.reader,copy,vectors.author.pubkey_base58,undefined)).toThrow();
  });
  it("seals real SDK payload before transport and preserves actual recall bytes",async()=>{
    let signed="";
    const fetchImpl=(async(_url:unknown,init?:RequestInit)=>{
      const req=JSON.parse(init!.body as string);
      if(req.params.name==="mnemonic_attest_a2a") {
        expect(init!.body).not.toContain("PRIVATE SEALED E2E SENTINEL");
        expect(req.params.arguments.sealed).toBe(true);
        expect(req.params.arguments.payload).toBeUndefined();
        signed=req.params.arguments.signed;
        const v=JSON.parse(wasm.verify_a2a(hex(signed),vectors.author.pubkey_base58));
        return Response.json({jsonrpc:"2.0",id:req.id,result:{content:[{type:"text",text:JSON.stringify({attestation_id:`a2a:${v.content_hash}`})}]}});
      }
      const v=JSON.parse(wasm.verify_a2a(hex(signed),vectors.author.pubkey_base58));
      const b=v.binding;
      return Response.json({jsonrpc:"2.0",id:req.id,result:{content:[{type:"text",text:JSON.stringify({attestations:[{
        attestation_id:`a2a:${v.content_hash}`,cose_envelope_hex:signed,content_hash:v.content_hash,signer_pubkey:vectors.author.pubkey_base58,
        kind:b.kind,signed_at:b.created_at,payload:b.payload,sealed:b.sealed,sealed_payload:b.payload.parts[0].data
      }]})}]}});
    }) as typeof fetch;
    const sender=client(vectors.author,fetchImpl);
    await sender.attestA2AMessage(vectors.payload as any,vectors.context_id,{sealed:{recipients:[{card:vectors.card,trustedCardSigner:vectors.reader.pubkey_base58}],chunkSize:19}});
    await sender.attestA2AMessage(vectors.payload as any,vectors.context_id,{sealed:{recipients:[{card:vectors.card,trustedCardSigner:vectors.reader.pubkey_base58}]}});
    expect((await verifyA2AAttestation(signed,vectors.author.pubkey_base58)).binding.sealed).toBe(true);
    const receiver=client(vectors.reader,fetchImpl);
    const rows=await receiver.recallA2AContext(vectors.context_id,{sealed:true});
    expect(rows[0]!.coseEnvelopeHex).toBe(signed);
    expect(await receiver.openA2AAttestation(rows[0]!,vectors.author.pubkey_base58)).toEqual(vectors.payload);
    await expect(receiver.openA2AAttestation({...rows[0]!,signedAt:"fake"},vectors.author.pubkey_base58)).rejects.toThrow();
    await expect(client(vectors.outsider).openA2AAttestation(rows[0]!,vectors.author.pubkey_base58)).rejects.toThrow();
  });
  it("correctly derives the recipient scalar, rather than passing the seed",()=>{
    const seed=new Uint8Array(vectors.reader.secret.slice(0,32));
    const scalar=wasm.x25519_secret_from_seed(seed);
    expect(scalar).not.toEqual(seed);
  });
  it("refuses substituted unsigned recipient cards before any network write",async()=>{
    let calls=0;
    const c=client(vectors.author,(async()=>{calls++;return Response.json({});}) as typeof fetch);
    const card={...vectors.card,signatures:[]};
    await expect(c.attestA2AMessage(vectors.payload as any,vectors.context_id,{sealed:{recipients:[{card,trustedCardSigner:vectors.reader.pubkey_base58}]}})).rejects.toThrow();
    expect(calls).toBe(0);
  });
});
