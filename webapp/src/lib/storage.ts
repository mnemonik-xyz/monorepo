/**
 * Centralised localStorage accessors for the webapp.
 *
 * Two values live in localStorage today:
 *
 *   `mnemonic.identity` — the user's keypair JSON (`{ secret: number[], pubkey_base58: string }`)
 *                         produced by the WASM `generate_keypair` / `import_keypair_json`
 *                         exports. The shape mirrors `core/src/wasm/mod.rs::KeypairJson`.
 *
 *   `mnemonic.jwt`      — the OAuth-issued Bearer token used for `/api/*` and `/mcp` requests.
 *                         Issued by `mcp/src/oauth.rs::token_handler` (Task 4).
 *
 * Wrapping every read/write here keeps the keys (and any future schema bumps) in one place.
 */

export interface KeypairJson {
  /** 64-byte Solana keypair (32-byte seed || 32-byte pubkey), encoded as a number array. */
  secret: number[];
  /** Base58 Ed25519 pubkey, kept alongside the secret for display without re-derivation. */
  pubkey_base58: string;
}

const IDENTITY_KEY = "mnemonic.identity";
const JWT_KEY = "mnemonic.jwt";

export function readIdentity(): KeypairJson | null {
  try {
    const raw = localStorage.getItem(IDENTITY_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    // Defensive shape-check — refuse to return malformed identity to callers.
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !Array.isArray(parsed.secret) ||
      typeof parsed.pubkey_base58 !== "string"
    ) {
      return null;
    }
    return parsed as KeypairJson;
  } catch {
    return null;
  }
}

export function writeIdentity(identity: KeypairJson): void {
  localStorage.setItem(IDENTITY_KEY, JSON.stringify(identity));
}

export function clearIdentity(): void {
  localStorage.removeItem(IDENTITY_KEY);
}

export function readJwt(): string | null {
  return localStorage.getItem(JWT_KEY);
}

export function writeJwt(token: string): void {
  localStorage.setItem(JWT_KEY, token);
}

export function clearJwt(): void {
  localStorage.removeItem(JWT_KEY);
}

/**
 * Decode a JWT payload without verifying the signature. Browser-side use only —
 * never trust the result for authorization decisions; the server is the source of truth.
 * Useful for surfacing the `sub` (DID) on the Sign page so the user knows which
 * identity is about to sign.
 */
export function decodeJwtPayload(
  token: string
): Record<string, unknown> | null {
  try {
    const parts = token.split(".");
    if (parts.length !== 3) return null;
    const payload = parts[1];
    if (!payload) return null;
    // base64url -> base64
    const b64 = payload.replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
    const decoded = atob(padded);
    return JSON.parse(decoded) as Record<string, unknown>;
  } catch {
    return null;
  }
}
