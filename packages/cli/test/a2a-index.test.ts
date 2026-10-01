import {describe,it,expect} from 'vitest';
import {stat} from 'node:fs/promises';
import {join} from 'node:path';
import {fileA2AIndex} from '../src/a2a-index.js';
import {withTmpConfigDir} from './helpers.js';
describe('durable agent A2A index',()=>{
 it('preserves concurrent writes across independent stores and uses private files',async()=>{
  const tmp=withTmpConfigDir();try{const first=fileA2AIndex('Abc'),second=fileA2AIndex('Abc');
   await Promise.all(Array.from({length:8},(_,i)=>(i%2?first:second).put({attestationId:`a2a:${i}`,kind:'message',signedAt:'2026-10-01T00:00:00Z',payload:{parts:[]}})));
   expect(await fileA2AIndex('Abc').list()).toHaveLength(8);
   if(process.platform!=='win32')expect((await stat(join(tmp.dir,'a2a-Abc.json'))).mode&0o777).toBe(0o600);
  }finally{tmp.cleanup();}
 });
});
