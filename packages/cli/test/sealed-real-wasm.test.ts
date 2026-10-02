import {mkdtempSync,rmSync,readdirSync,readFileSync,writeFileSync,statSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {Keypair} from "@mnemonik-xyz/sdk";
import {saveIdentity} from "../src/config.js";
import {runSign} from "../src/commands/sign.js";
import {runOpen} from "../src/commands/open.js";

let dir:string,previous:string|undefined,realFetch:typeof fetch;
beforeEach(async()=>{
  previous=process.env.MNEMONIC_CONFIG_DIR;
  dir=mkdtempSync(join(tmpdir(),"mnemonic-sealed-real-"));
  process.env.MNEMONIC_CONFIG_DIR=dir;
  saveIdentity(await Keypair.generate());
  realFetch=globalThis.fetch;
  globalThis.fetch=vi.fn(async()=>{throw new Error("offline test: unexpected HTTP");});
  vi.spyOn(process.stdout,"write").mockImplementation(()=>true);
  vi.spyOn(process.stderr,"write").mockImplementation(()=>true);
});
afterEach(()=>{
  globalThis.fetch=realFetch;
  vi.restoreAllMocks();
  rmSync(dir,{recursive:true,force:true});
  if(previous===undefined)delete process.env.MNEMONIC_CONFIG_DIR;else process.env.MNEMONIC_CONFIG_DIR=previous;
});
it("real SDK/WASM persists and reopens offline; rejects ciphertext substituted under another hash",async()=>{
  await runSign("first private memory",{baseUrl:"http://offline"});
  const author=readdirSync(join(dir,"sealed"))[0]!;
  const folder=join(dir,"sealed",author);
  const firstFile=readdirSync(folder)[0]!;
  const first=JSON.parse(readFileSync(join(folder,firstFile),"utf8"));
  expect(JSON.stringify(first)).not.toContain("first private memory");
  if(process.platform!=="win32")expect(statSync(join(folder,firstFile)).mode&0o777).toBe(0o600);
  let printed="";
  vi.spyOn(process.stdout,"write").mockImplementation(s=>{printed+=String(s);return true;});
  await runOpen(first.memoryHash,{baseUrl:"http://offline"});
  expect(printed).toContain("first private memory");
  await runSign("second private memory",{baseUrl:"http://offline"});
  const secondFile=readdirSync(folder).find(name=>name!==firstFile)!;
  const second=JSON.parse(readFileSync(join(folder,secondFile),"utf8"));
  // Keep requested identity metadata while swapping both cryptographic byte fields.
  writeFileSync(join(folder,firstFile),JSON.stringify({...first,outerCbor:second.outerCbor,signedBytes:second.signedBytes}));
  await expect(runOpen(first.memoryHash,{baseUrl:"http://offline"})).rejects.toThrow(/ciphertext hash mismatch/);
  expect(globalThis.fetch).not.toHaveBeenCalled();
});
