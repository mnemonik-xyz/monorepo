// Unit tests for CLI redactSecrets — propagated copy of SDK redactJWT.
//
// Drives T11 audit fix (A09): wasm-bindgen errors and stored-keypair
// payloads carry a 64-element JSON byte array `[n0,...,n63]`. The hex
// regex doesn't catch them; this asserts the new SOLANA_KEYPAIR_ARRAY_RE.

import { describe, expect, it } from "vitest";

import { redactSecrets, AuthError } from "../src/errors.js";

describe("redactSecrets (CLI)", () => {
  it("redacts a JWT-shaped run", () => {
    // Single eyJ-prefixed segment with ≥20 base64url chars matches the regex.
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
    expect(redactSecrets(`token=${jwt}`)).toBe("token=[REDACTED-JWT]");
  });

  it("redacts a 128-hex ed25519 secret run", () => {
    const hex = "a".repeat(128);
    expect(redactSecrets(`secret=${hex}`)).toBe("secret=[REDACTED-SECRET]");
  });

  it("redacts a Solana 64-element keypair JSON array (T11 A09)", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const out = redactSecrets(`keypair=${arr}`);
    expect(out).toBe("keypair=[REDACTED-KEYPAIR]");
    expect(out).not.toContain(arr);
  });

  it("redacts Solana keypair array with surrounding whitespace", () => {
    const elems = Array.from({ length: 64 }, (_, i) => i).join(", ");
    const arr = `[ ${elems} ]`;
    expect(redactSecrets(`kp=${arr};`)).toBe("kp=[REDACTED-KEYPAIR];");
  });

  it("AuthError carries a redacted message including keypair arrays", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const err = new AuthError(`signer rejected ${arr}`);
    expect(err.message).toContain("[REDACTED-KEYPAIR]");
    expect(err.message).not.toContain(arr);
    expect(err.exitCode).toBe(4);
  });

  it("is idempotent", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const once = redactSecrets(arr);
    expect(redactSecrets(once)).toBe(once);
  });
});
