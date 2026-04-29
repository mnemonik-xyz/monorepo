// `mnemonic whoami` — Decision 14: client-side only.
//
// Reads identity.json + token.json. Computes pubkey, DID, signer_match (does
// the JWT's `sub` claim agree with identity.pubkey_base58?), JWT iat/exp.
// Does NOT call `client.whoami()` — that returns the SERVER's identity, not
// the user's.
//
// Optional: when `--with-count` is passed, performs a recall(' ', topK=0) to
// fetch a total memory count. Off by default — Decision 14's TDD anchor
// asserts `whoami` makes ≤ 1 fetch by default.

import {
  Keypair,
  LocalSigner,
  MnemonicClient,
  parseJwtPayload,
} from "@mnemonik-xyz/sdk";

import {
  identityExists,
  loadIdentityJson,
  loadToken,
  tokenExists,
} from "../config.js";
import { fromSdkError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";

export interface WhoamiOptions extends OutputOptions {
  withCount?: boolean;
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mc.mnemonik.xyz";

export interface WhoamiResult {
  pubkey: string | null;
  did: string | null;
  signer_match: boolean | null;
  jwt: {
    sub: string;
    issued_at: string;
    expires_at: string;
  } | null;
  total_memories?: number;
}

export async function runWhoami(opts: WhoamiOptions): Promise<void> {
  const result: WhoamiResult = {
    pubkey: null,
    did: null,
    signer_match: null,
    jwt: null,
  };

  if (identityExists()) {
    const id = loadIdentityJson();
    result.pubkey = id.pubkey_base58;
    result.did = `did:sol:${id.pubkey_base58}`;
  }

  if (tokenExists()) {
    const tok = loadToken();
    let payload;
    try {
      payload = parseJwtPayload(tok.jwt);
    } catch (e) {
      throw fromSdkError(e);
    }
    result.jwt = {
      sub: payload.sub,
      issued_at: new Date(payload.iat * 1000).toISOString(),
      expires_at: tok.expires_at,
    };
    if (result.pubkey !== null) {
      result.signer_match = result.pubkey === payload.sub;
    }
  }

  if (opts.withCount && result.pubkey && result.jwt) {
    // Optional one-shot server call — gated behind --with-count so the
    // TDD anchor's "no fetch by default" assertion holds.
    const baseUrl =
      opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
    const id = loadIdentityJson();
    const kp = await Keypair.fromJSON(id);
    const tok = loadToken();
    const client = new MnemonicClient({
      baseUrl,
      signer: new LocalSigner(kp),
      jwt: tok.jwt,
    });
    try {
      const r = await client.recall(" ", { topK: 0 });
      result.total_memories = r.total;
    } catch (e) {
      throw fromSdkError(e);
    }
  }

  format(result, opts, (_d, _color) => {
    const lines: string[] = [];
    lines.push(
      `pubkey:       ${result.pubkey ?? "(none — run `mnemonic init`)"}`
    );
    lines.push(`did:          ${result.did ?? "(none)"}`);
    if (result.jwt) {
      lines.push(`jwt sub:      ${result.jwt.sub}`);
      lines.push(`jwt issued:   ${result.jwt.issued_at}`);
      lines.push(`jwt expires:  ${result.jwt.expires_at}`);
      lines.push(
        `signer_match: ${
          result.signer_match === null
            ? "(unknown)"
            : result.signer_match
            ? "yes"
            : "NO — identity and JWT disagree"
        }`
      );
    } else {
      lines.push(`jwt:          (none — run \`mnemonic login\`)`);
    }
    if (typeof result.total_memories === "number") {
      lines.push(`memories:     ${result.total_memories}`);
    }
    return lines.join("\n");
  });
}
