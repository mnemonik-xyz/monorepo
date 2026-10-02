import {mkdir,readFile,rename,writeFile,unlink} from 'node:fs/promises';
import {join} from 'node:path';
import {randomUUID} from 'node:crypto';
import {open} from 'node:fs/promises';
import {setTimeout as delay} from 'node:timers/promises';
import {configDir} from './config.js';
import type {A2AIndexStore,Attestation} from '@mnemonik-xyz/sdk';
/** Identity-scoped agent storage. Atomic replacement; serialize writes in this process. */
export function fileA2AIndex(identity:string):A2AIndexStore {
 if(!/^[1-9A-HJ-NP-Za-km-z]+$/.test(identity))throw new Error('invalid identity');
 const file=join(configDir(),`a2a-${identity}.json`);let pending=Promise.resolve();
 async function list():Promise<Attestation[]>{try{const rows=JSON.parse(await readFile(file,'utf8'));if(!Array.isArray(rows))throw new Error('invalid A2A index');return rows;}catch(e){if((e as NodeJS.ErrnoException).code==='ENOENT')return [];throw e;}}
 return {list,put(row){const work=pending.then(async()=>{await mkdir(configDir(),{recursive:true,mode:0o700});let lock;const deadline=Date.now()+10000;while(!lock){try{lock=await open(file+'.lock','wx',0o600);}catch(e){if((e as NodeJS.ErrnoException).code!=='EEXIST'||Date.now()>=deadline)throw e;await delay(25);}}try{const rows=await list();const idx=rows.findIndex(r=>r.attestationId===row.attestationId);if(idx<0)rows.push(row);else rows[idx]=row;const tmp=`${file}.${randomUUID()}`;try{await writeFile(tmp,JSON.stringify(rows),{mode:0o600,flag:'wx'});await rename(tmp,file);}finally{await unlink(tmp).catch(()=>{});}}finally{await lock.close();await unlink(file+'.lock');}});pending=work.catch(()=>{});return work;}};
}
