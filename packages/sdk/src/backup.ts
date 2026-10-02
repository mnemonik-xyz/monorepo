/** Explicit encrypted export; never accesses a keystore or network. Keep the owner
 * public key and passphrase independently of this portable file. */
import { Keypair } from './keypair.js';
import { verifyRecoveryCheckpoint } from './checkpoint.js';
import type { SignedRecoveryCheckpoint, RecoveryCheckpoint } from './checkpoint.js';
import { IntegrityError, UserError } from './errors.js';

export interface RecoveryBackup {
  version: 1; algorithm: 'PBKDF2-SHA256-AES256GCM'; iterations: 600000;
  salt: string; iv: string; ciphertext: string;
}
const AAD = new TextEncoder().encode('mnemonic.recovery-backup.v1');
const hex = (bytes: Uint8Array) => Array.from(bytes,b=>b.toString(16).padStart(2,'0')).join('');
function bytes(value: string, size?: number): Uint8Array<ArrayBuffer> {
  if (typeof value !== 'string' || value.length > 4194336 || !/^(?:[0-9a-f]{2})+$/.test(value) || (size !== undefined && value.length !== size*2)) throw new UserError('invalid backup encoding');
  return Uint8Array.from(value.match(/../g)!.map(x=>parseInt(x,16)));
}
async function key(passphrase: string, salt: Uint8Array<ArrayBuffer>): Promise<CryptoKey> {
  if (typeof passphrase !== 'string' || passphrase.length < 12 || passphrase.length > 4096) throw new UserError('backup passphrase must have 12–4096 characters');
  const raw = await crypto.subtle.importKey('raw',new TextEncoder().encode(passphrase),'PBKDF2',false,['deriveKey']);
  return crypto.subtle.deriveKey({name:'PBKDF2',hash:'SHA-256',salt,iterations:600000},raw,{name:'AES-GCM',length:256},false,['encrypt','decrypt']);
}
export async function createRecoveryBackup(signed: SignedRecoveryCheckpoint, identity: Keypair, passphrase: string, encryptionKeys: Record<string,string> = {}): Promise<RecoveryBackup> {
  identity = new Keypair(identity.toJSON());
  signed = JSON.parse(JSON.stringify(signed)) as SignedRecoveryCheckpoint;
  encryptionKeys = JSON.parse(JSON.stringify(encryptionKeys)) as Record<string,string>;
  await verifyRecoveryCheckpoint(signed,identity.pubkey,{artifactKind:signed.checkpoint.artifactKind,scope:signed.checkpoint.scope});
  validateEncryptionKeys(encryptionKeys);
  const plaintext = new TextEncoder().encode(JSON.stringify({checkpoint:signed,identity:JSON.parse(await identity.toBackupString()),encryptionKeys}));
  if (plaintext.length > 2097152) throw new UserError('backup exceeds 2 MiB');
  const salt = crypto.getRandomValues(new Uint8Array(16)), iv = crypto.getRandomValues(new Uint8Array(12));
  try {
    const encrypted = await crypto.subtle.encrypt({name:'AES-GCM',iv,additionalData:AAD},await key(passphrase,salt),plaintext);
    return {version:1,algorithm:'PBKDF2-SHA256-AES256GCM',iterations:600000,salt:hex(salt),iv:hex(iv),ciphertext:hex(new Uint8Array(encrypted))};
  } finally { plaintext.fill(0); }
}
function validateEncryptionKeys(keys: Record<string,string>): void {
  if (!keys || typeof keys !== 'object' || Array.isArray(keys) || Object.keys(keys).length > 256) throw new UserError('invalid backed-up encryption keys');
  for (const [name,secret] of Object.entries(keys)) { if (!name || name.length > 256) throw new UserError('invalid encryption key label'); bytes(secret,32); }
}
export async function openRecoveryBackup(backup: RecoveryBackup, passphrase: string, trustedOwner: string, scope: {artifactKind:'memory'|'a2a';scope:string}): Promise<{checkpoint:RecoveryCheckpoint;signedCheckpoint:SignedRecoveryCheckpoint;identity:Keypair;encryptionKeys:Record<string,string>}> {
  if (!backup || Object.keys(backup).sort().join('|') !== ['version','algorithm','iterations','salt','iv','ciphertext'].sort().join('|') || backup.version !== 1 || backup.algorithm !== 'PBKDF2-SHA256-AES256GCM' || backup.iterations !== 600000) throw new UserError('unsupported backup format');
  const salt=bytes(backup.salt,16),iv=bytes(backup.iv,12),ciphertext=bytes(backup.ciphertext);
  let plaintext: Uint8Array;
  try { plaintext = new Uint8Array(await crypto.subtle.decrypt({name:'AES-GCM',iv,additionalData:AAD},await key(passphrase,salt),ciphertext)); }
  catch { throw new IntegrityError('backup authentication failed'); }
  try {
    const content = JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(plaintext));
    if (!content || Object.keys(content).sort().join('|') !== 'checkpoint|encryptionKeys|identity') throw new IntegrityError('invalid backup contents');
    const checkpoint = await verifyRecoveryCheckpoint(content.checkpoint,trustedOwner,scope);
    const identity = await Keypair.fromJSON(content.identity);
    if (identity.pubkey !== trustedOwner) throw new IntegrityError('backup identity differs from pinned owner');
    validateEncryptionKeys(content.encryptionKeys);
    return {checkpoint,signedCheckpoint:content.checkpoint,identity,encryptionKeys:content.encryptionKeys};
  } finally { plaintext.fill(0); }
}
