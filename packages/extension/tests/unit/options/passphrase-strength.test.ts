// PassphraseStrength acceptance helper — guards the (length >= 12 AND
// score >= 3) gate that user-spec Scenario 1 requires.

import { describe, it, expect } from "vitest";
import { isPassphraseAcceptable } from "../../../src/options/components/PassphraseStrength.js";

describe("isPassphraseAcceptable", () => {
  it("rejects empty input", () => {
    expect(isPassphraseAcceptable("")).toBe(false);
  });

  it("rejects passphrases shorter than 12 chars even if otherwise strong", () => {
    // 11 chars — under the floor.
    expect(isPassphraseAcceptable("xZq!7$Bwn3K")).toBe(false);
  });

  it("rejects 12+ char passphrases that score below 3 (length-12, score-2 case)", () => {
    // A 12-char trivial sequence — zxcvbn rates this <3.
    expect(isPassphraseAcceptable("password1234")).toBe(false);
  });

  it("accepts a long, non-dictionary passphrase (length-12+, score-3+)", () => {
    expect(isPassphraseAcceptable("Tr0ub4dor&3-Mnemonik-lazy-bear")).toBe(true);
  });

  it("trims surrounding whitespace before evaluating", () => {
    // 11-char core padded to 18 chars by leading/trailing spaces — once
    // trimmed it falls below the 12-char floor.
    expect(isPassphraseAcceptable("    xZq!7$Bwn3K   ")).toBe(false);
  });
});
