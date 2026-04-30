// Unit tests for redactJWT — defense-in-depth secret scrubbing.
//
// Drives:
//   - Decision 10 (every thrown error is JWT-redacted before reaching stderr)
//   - T11 audit fix (A09): Solana-style 64-element keypair arrays must be
//     redacted alongside JWTs and 128-hex secrets.

import { describe, expect, it } from "vitest";

import { redactJWT, AuthError } from "../src/errors.js";

describe("redactJWT", () => {
  it("redacts a JWT-shaped run", () => {
    // The redaction regex matches each `eyJ...` segment up to a non-base64url
    // char (`.`). A standard 3-segment JWT therefore yields two redaction
    // markers — header and payload — separated by literal `.`. Signature
    // segment is NOT redacted because it doesn't start with `eyJ`.
    const jwt =
      "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0SnNubGFiZWwifQ.sigfoobarsigfoobar";
    const out = redactJWT(`token=${jwt}`);
    expect(out).toContain("[REDACTED-JWT]");
    // Neither base64url segment should survive verbatim.
    expect(out).not.toContain("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
    expect(out).not.toContain("eyJzdWIiOiJ0ZXN0SnNubGFiZWwifQ");
  });

  it("redacts a 128-hex ed25519 secret run", () => {
    const hex = "a".repeat(128);
    expect(redactJWT(`secret=${hex}`)).toBe("secret=[REDACTED-SECRET]");
  });

  it("redacts a Solana 64-element keypair JSON array (T11 A09)", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const message = `signer error at sign_challenge with input ${arr} (oops)`;
    const out = redactJWT(message);
    expect(out).toContain("[REDACTED-KEYPAIR]");
    expect(out).not.toContain(arr);
    // Surrounding text preserved.
    expect(out.startsWith("signer error at sign_challenge with input ")).toBe(
      true
    );
    expect(out.endsWith(" (oops)")).toBe(true);
  });

  it("redacts Solana keypair array with whitespace between elements", () => {
    const elems = Array.from({ length: 64 }, (_, i) => i).join(", ");
    const arr = `[ ${elems} ]`;
    const message = `keypair=${arr};`;
    expect(redactJWT(message)).toBe("keypair=[REDACTED-KEYPAIR];");
  });

  it("does NOT redact 63-element arrays (off-by-one guard)", () => {
    const arr = `[${Array.from({ length: 63 }, (_, i) => i).join(",")}]`;
    expect(redactJWT(arr)).toBe(arr);
  });

  it("does NOT redact 65-element arrays (off-by-one guard)", () => {
    const arr = `[${Array.from({ length: 65 }, (_, i) => i).join(",")}]`;
    // 65 elements: regex anchors `]` after exactly 64 numbers, so the
    // bracket must be followed by `,<n>]` — no match. Input passes through.
    expect(redactJWT(arr)).toBe(arr);
  });

  it("redacts multiple secret types in one string", () => {
    // Use a single-segment eyJ-prefixed token so the asserted output is
    // simple — header/payload boundary handling is covered by the JWT-run
    // case above.
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
    const hex = "f".repeat(128);
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const msg = `jwt=${jwt} hex=${hex} kp=${arr}`;
    const out = redactJWT(msg);
    expect(out).toBe(
      "jwt=[REDACTED-JWT] hex=[REDACTED-SECRET] kp=[REDACTED-KEYPAIR]"
    );
  });

  it("is idempotent", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const once = redactJWT(arr);
    const twice = redactJWT(once);
    expect(twice).toBe(once);
  });

  it("coerces non-string input via String(...)", () => {
    expect(redactJWT(undefined)).toBe("");
    expect(redactJWT(null)).toBe("");
    expect(redactJWT(42)).toBe("42");
  });

  it("AuthError message is run through redactJWT including keypair arrays", () => {
    const arr = `[${Array.from({ length: 64 }, (_, i) => i).join(",")}]`;
    const err = new AuthError(`bad keypair ${arr}`);
    expect(err.message).toContain("[REDACTED-KEYPAIR]");
    expect(err.message).not.toContain(arr);
  });
});
