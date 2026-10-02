import {describe,it,expect,vi} from 'vitest';
import {IrysDiscoverySource,ArweaveDiscoverySource} from '../src/discovery.js';
const scope={artifactKind:'a2a' as const,context:'context',expectedAuthors:['pinned']};
describe('public discovery adapters',()=>{
 it('uses provider-specific queries and opaque cursors without account credentials',async()=>{
  for(const Source of [IrysDiscoverySource,ArweaveDiscoverySource]){
   let body:any;const source=new Source('https://index.invalid/graphql',async(_url,init)=>{
    expect(init?.credentials).toBe('omit');expect(init?.redirect).toBe('error');expect(new Headers(init?.headers).has('authorization')).toBe(false);body=JSON.parse(init!.body as string);
    return Response.json({data:{transactions:{edges:[{cursor:'opaque-cursor',node:{id:'A'.repeat(43),tags:[{name:'Producer',value:'pinned'}]}}],pageInfo:{hasNextPage:true}}}});
   });
   const page=await source.page(scope,'previous');expect(body.variables.after).toBe('previous');
   expect(body.query.includes('sort:HEIGHT_ASC')).toBe(Source===ArweaveDiscoverySource);
   expect(page.nextCursor).toBe('opaque-cursor');expect(page.candidates[0]!.metadata?.Producer).toEqual(['pinned']);
  }
 });
 it('classifies unavailable and malformed responses including oversized streams',async()=>{
  for(const [response,status] of [[new Response('',{status:503}),'unavailable'],[Response.json({errors:[{}]}),'unavailable'],[new Response('{'),'malformed'],[Response.json({data:{transactions:{edges:[],pageInfo:{hasNextPage:true}}}}),'malformed'],[new Response('x'.repeat(1048577)),'malformed']] as const){
   await expect(new IrysDiscoverySource('https://index.invalid',async()=>response).page(scope)).rejects.toMatchObject({status});
  }
 });
 it('binds source identity to endpoint and flavour and rejects credential-bearing endpoints',()=>{
  expect(new IrysDiscoverySource('https://a.invalid').identity).not.toBe(new ArweaveDiscoverySource('https://a.invalid').identity);
  expect(new IrysDiscoverySource('https://a.invalid').identity).not.toBe(new IrysDiscoverySource('https://b.invalid').identity);
  expect(()=>new IrysDiscoverySource('https://user:password@a.invalid')).toThrow();
 });
});

it('bounds a stalled response body and forwards caller cancellation',async()=>{
 vi.useFakeTimers();
 try {
  const source=new IrysDiscoverySource('https://index.invalid',async()=>new Response(new ReadableStream({start(){}})));
  const pending=expect(source.page(scope)).rejects.toThrow('deadline exceeded');
  await vi.advanceTimersByTimeAsync(15000);await pending;
  const controller=new AbortController();let forwarded:AbortSignal|null|undefined;
  const blocked=new IrysDiscoverySource('https://index.invalid',async(_u,init)=>{forwarded=init?.signal;return new Promise<Response>(()=>{});});
  const cancelled=expect(blocked.page(scope,undefined,controller.signal)).rejects.toThrow('cancelled');
  controller.abort(new Error('cancelled'));await cancelled;expect(forwarded?.aborted).toBe(true);
 } finally {vi.useRealTimers();}
});
