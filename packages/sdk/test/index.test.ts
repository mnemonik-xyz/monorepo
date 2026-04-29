// Smoke test for the public barrel — ensures every re-export resolves and
// covers `src/index.ts` (which is otherwise 0% coverage despite being the
// load-bearing public API surface).

import { describe, expect, it } from "vitest";

import {
  AuthError,
  IntegrityError,
  Keypair,
  LocalSigner,
  MnemonicClient,
  MnemonicError,
  ServerError,
  UserError,
  buildAuthorizeUrl,
  coseSignPayload,
  exchangeCodeForToken,
  generatePkceVerifier,
  parseJwtPayload,
  pendingAuthSessions,
  pkceChallenge,
  randomState,
  redactJWT,
} from "../src/index.js";

describe("public barrel exports", () => {
  it("exposes the full T2 + T3 surface", () => {
    // Classes and functions exist with the expected shape. We do not
    // exercise behavior here — that's covered by their dedicated tests —
    // we only verify the barrel wires up.
    expect(typeof MnemonicClient).toBe("function");
    expect(typeof LocalSigner).toBe("function");
    expect(typeof Keypair).toBe("function");
    expect(typeof MnemonicError).toBe("function");
    expect(typeof AuthError).toBe("function");
    expect(typeof ServerError).toBe("function");
    expect(typeof IntegrityError).toBe("function");
    expect(typeof UserError).toBe("function");
    expect(typeof redactJWT).toBe("function");
    expect(typeof coseSignPayload).toBe("function");
    expect(typeof buildAuthorizeUrl).toBe("function");
    expect(typeof exchangeCodeForToken).toBe("function");
    expect(typeof parseJwtPayload).toBe("function");
    expect(typeof generatePkceVerifier).toBe("function");
    expect(typeof pkceChallenge).toBe("function");
    expect(typeof randomState).toBe("function");
    // pendingAuthSessions is a Map-like store; just assert it's defined.
    expect(pendingAuthSessions).toBeDefined();
  });
});
