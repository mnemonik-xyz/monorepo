#!/usr/bin/env node
// Mint an HS256 JWT matching the MCP server's expected claims, for local testing
// of tool calls that require auth (sign_memory / verify; recall is allowlist-open).
//
// The HMAC key is the *base64-decoded bytes* of MCP_JWT_SECRET (same as the
// server: EncodingKey::from_secret(decoded)). Claims: iss=mcp.mnemonik.xyz,
// aud=mcp, sub=<server identity pubkey>, 1h exp.
//
// Usage:
//   MCP_JWT_SECRET=<base64> node mint-jwt.cjs [sub]
//   node mint-jwt.cjs <base64-or-path-to-secret-file> [sub]
const crypto = require('crypto');
const fs = require('fs');

function resolveSecretB64() {
  const a = process.argv[2];
  if (a && !looksLikeSub(a)) {
    // arg is either a file path or the base64 secret itself
    try {
      if (fs.existsSync(a)) return fs.readFileSync(a, 'utf8').trim();
    } catch (_) {}
    return a.trim();
  }
  if (process.env.MCP_JWT_SECRET) return process.env.MCP_JWT_SECRET.trim();
  console.error('No secret: set MCP_JWT_SECRET or pass it (or a file path) as arg 1');
  process.exit(1);
}
// A base58 pubkey "sub" never contains base64 '+' '/' '=' and is ~44 chars; a
// secret is base64. Heuristic only matters when arg1 is actually the sub.
function looksLikeSub(s) {
  return false; // arg1 is always the secret in this tool; sub is arg2
}

const key = Buffer.from(resolveSecretB64(), 'base64');
const subArgIdx = process.env.MCP_JWT_SECRET ? 2 : 3;
const sub = process.argv[subArgIdx] || '4rx4BpZmdp8CZng11QiiWUhjytV4AeSV6BtMKUGFrSEp';

const b64url = (buf) =>
  Buffer.from(buf).toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const now = Math.floor(Date.now() / 1000);
const header = { alg: 'HS256', typ: 'JWT' };
const payload = { iss: 'mcp.mnemonik.xyz', aud: 'mcp', sub, iat: now, exp: now + 3600, jti: crypto.randomUUID() };
const signingInput = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(payload))}`;
const sig = b64url(crypto.createHmac('sha256', key).update(signingInput).digest());
process.stdout.write(`${signingInput}.${sig}`);
