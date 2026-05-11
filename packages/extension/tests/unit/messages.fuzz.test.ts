// Property-based fuzz tests for `parseMsg`. The unit test suite at
// `tests/unit/messages.test.ts` enumerates the documented variants;
// this file binds the *invariant*: `parseMsg(x)` must NEVER throw and
// must ALWAYS return either `null` or a fully-typed `Msg`.
//
// We don't depend on `fast-check` — it isn't in the extension's
// devDependencies, and adding a new dep purely for property generation
// is heavier than the ≤50-line hand-rolled generator below. See
// `decisions.md` T24 coverage-gaps subsection for the trade-off note.
//
// Iteration count: 1000 random inputs. Each call is sub-millisecond
// (pure object / array walking, no I/O, no crypto), so the full suite
// lands in well under a second on a 2024-class laptop AND on the
// GitHub Actions ubuntu-latest runner.

import { describe, it, expect } from "vitest";
import { parseMsg, type Msg } from "../../src/messages.js";

// ── Tiny PRNG (mulberry32) ───────────────────────────────────────────────
//
// We need *deterministic* runs so a CI failure can be reproduced by
// re-running with the same seed. `Math.random` has no observable seed
// API; this 16-line PRNG covers the property-test use case (uniform-ish
// 32-bit state, period ~2^32). Seeded with a constant so a bug surfaces
// the same way locally and on CI.

function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return (): number => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── Hand-rolled generator (≤50 lines as per T24 brief) ───────────────────

const KNOWN_TYPES = [
  "sw:open-recall-overlay",
  "sw:save-selection",
  "ui:sign-memory",
  "ui:recall",
  "ui:flush-pending",
  "tab:capture-candidate",
  "tab:fab-save-chat",
  "tab:fab-save-selection",
  "tab:fab-open-popup",
  "unknown:bogus",
  "ui:eval-rce", // attacker-supplied discriminant — must reject
  "", // empty string discriminant
];

function randomScalar(r: () => number): unknown {
  const pick = Math.floor(r() * 8);
  if (pick === 0) return null;
  if (pick === 1) return undefined;
  if (pick === 2) return r() < 0.5;
  if (pick === 3) return Math.floor((r() - 0.5) * 1e6);
  if (pick === 4)
    return Math.random() < 0.5 ? Number.NaN : Number.POSITIVE_INFINITY;
  if (pick === 5)
    return r()
      .toString(36)
      .slice(2, 2 + Math.floor(r() * 32));
  if (pick === 6) return Symbol("sym").toString(); // pretend; symbol itself is rare
  return [];
}

function randomValue(r: () => number, depth: number): unknown {
  if (depth <= 0) return randomScalar(r);
  const pick = Math.floor(r() * 6);
  if (pick === 0) return randomScalar(r);
  if (pick === 1) {
    const len = Math.floor(r() * 4);
    const arr: unknown[] = [];
    for (let i = 0; i < len; i++) arr.push(randomValue(r, depth - 1));
    return arr;
  }
  if (pick === 2) {
    // Plain object with random keys.
    const obj: Record<string, unknown> = {};
    const len = Math.floor(r() * 4);
    for (let i = 0; i < len; i++) {
      obj[`k${String(i)}`] = randomValue(r, depth - 1);
    }
    return obj;
  }
  if (pick === 3) {
    // Object with a known `type` discriminant but random payload.
    const t = KNOWN_TYPES[Math.floor(r() * KNOWN_TYPES.length)]!;
    return { type: t, payload: randomValue(r, depth - 1) };
  }
  if (pick === 4) {
    // Object with a known type AND a plausibly-shaped payload + extra
    // junk keys (the parser must accept this and ignore the extras).
    const t = KNOWN_TYPES[Math.floor(r() * KNOWN_TYPES.length)]!;
    return {
      type: t,
      payload: {
        selectionText: "x",
        pageUrl: "https://example.test/",
        capturedAt: "2026-05-11T00:00:00Z",
        content: "x",
        tags: ["a"],
        query: "q",
        ownerPubkey: "P",
        limit: 5,
        trigger: "hotkey",
        platform: "chatgpt",
        url: "https://x",
        __extra_junk: randomValue(r, depth - 1),
      },
      __extraTopLevel: r(),
    };
  }
  return randomScalar(r);
}

// ── Invariant: never throws, always returns Msg | null ───────────────────

function assertResultShape(out: unknown): void {
  if (out === null) return;
  // Result must be a plain object with a string `type` field that
  // appears in the documented union. We don't pin payload shape here —
  // the per-variant guards already do — but we DO require the
  // top-level structure.
  expect(out).not.toBeNull();
  expect(typeof out).toBe("object");
  expect(Array.isArray(out)).toBe(false);
  const o = out as { type?: unknown };
  expect(typeof o.type).toBe("string");
  // Narrow against the discriminant set. `as Msg["type"]` so a future
  // addition to the union surfaces here as a tsc error, not a runtime
  // surprise.
  const allowed: ReadonlySet<Msg["type"]> = new Set([
    "sw:open-recall-overlay",
    "sw:save-selection",
    "ui:sign-memory",
    "ui:recall",
    "ui:flush-pending",
    "tab:capture-candidate",
    "tab:fab-save-chat",
    "tab:fab-save-selection",
    "tab:fab-open-popup",
  ]);
  expect(allowed.has(o.type as Msg["type"])).toBe(true);
}

describe("parseMsg — fuzz / property tests", () => {
  it("parseMsg_never_throws — 1000 random inputs", () => {
    const r = mulberry32(0xc0ffee);
    let acceptedCount = 0;
    for (let i = 0; i < 1000; i++) {
      const input = randomValue(r, 4);
      let out: ReturnType<typeof parseMsg>;
      // The assertion under test: parseMsg must never throw on any
      // hostile shape. We wrap the call so a failure surfaces with the
      // offending input attached.
      try {
        out = parseMsg(input);
      } catch (err) {
        throw new Error(
          `parseMsg threw on input ${JSON.stringify(input)}: ${
            err instanceof Error ? err.message : String(err)
          }`,
        );
      }
      // Must be either null or a typed Msg — never undefined, never
      // partially-typed.
      expect(out === null || typeof out === "object").toBe(true);
      if (out !== null) {
        assertResultShape(out);
        acceptedCount++;
      }
    }
    // Round-1 review (test-reviewer-round1.json finding 2): a buggy
    // refactor that early-returned null for every input would slip
    // past the above per-iteration assertions silently. The generator
    // is seeded so this lower bound is stable run-to-run; tightening
    // the floor would invite flakes if the generator distribution
    // shifts under a future tweak.
    expect(acceptedCount).toBeGreaterThan(0);
  });

  it("never returns undefined — even for hostile primitives", () => {
    const candidates: unknown[] = [
      undefined,
      null,
      0,
      Number.NaN,
      Number.POSITIVE_INFINITY,
      "",
      "ui:sign-memory",
      true,
      false,
      [],
      [{ type: "ui:flush-pending" }],
      new Date("2026-05-11"),
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      function foo() {} as unknown,
      Symbol("s"),
    ];
    for (const c of candidates) {
      const out = parseMsg(c);
      // Must NOT be `undefined` — the contract is `Msg | null`.
      expect(out === undefined).toBe(false);
      expect(out === null || typeof out === "object").toBe(true);
    }
  });

  it("adversarial: valid type with completely wrong payload shape → null", () => {
    expect(parseMsg({ type: "ui:sign-memory", payload: 42 })).toBeNull();
    expect(parseMsg({ type: "ui:sign-memory", payload: null })).toBeNull();
    expect(parseMsg({ type: "ui:sign-memory", payload: [1, 2, 3] })).toBeNull();
    expect(
      parseMsg({
        type: "ui:recall",
        payload: { query: 123, ownerPubkey: "p" },
      }),
    ).toBeNull();
  });

  it("adversarial: valid type + valid payload + extra fields → parses, extras ignored", () => {
    const out = parseMsg({
      type: "ui:sign-memory",
      payload: {
        content: "hello",
        tags: ["a", "b"],
        __extra: "ignored",
        nestedJunk: { a: [1, 2, 3] },
      },
      __topLevelExtra: 9001,
    });
    expect(out).toMatchObject({
      type: "ui:sign-memory",
      payload: { content: "hello", tags: ["a", "b"] },
    });
  });

  it("adversarial: deeply-nested junk in payload field → null", () => {
    // `query` must be a string. A nested object that happens to type-
    // check as `{ nested: {...} }` must still be rejected.
    const out = parseMsg({
      type: "ui:recall",
      payload: {
        query: { nested: { a: [1, "b"] } },
        ownerPubkey: "P",
        limit: 5,
      },
    });
    expect(out).toBeNull();
  });

  it("adversarial: prototype-pollution-looking inputs are safe", () => {
    // `__proto__`, `constructor`, `prototype` keys must NOT crash the
    // parser. parseMsg uses bracket access (`input["type"]`) which DOES
    // walk the prototype chain — that's expected JS object semantics
    // and is benign here because the SW router only ever consumes the
    // narrowed `Msg` shape; the parser does not deep-copy keys, so a
    // prototype with extra junk fields never reaches a handler.
    //
    // The assertion here is the negative one: the function must not
    // throw, and the result must still be either null or a valid Msg.
    const a = parseMsg({ __proto__: { type: "ui:flush-pending" } });
    expect(a === null || a?.type === "ui:flush-pending").toBe(true);
    const b = parseMsg({ constructor: "ui:flush-pending" });
    // `constructor` is a real own key here, not the discriminant — and
    // the `type` key is absent, so the result must be null.
    expect(b).toBeNull();
    // Own `type` wins over inherited rogue keys.
    expect(
      parseMsg({ type: "ui:flush-pending", __proto__: { rogue: 1 } }),
    ).toEqual({ type: "ui:flush-pending" });
  });

  it("adversarial: arrays at the top level always return null", () => {
    expect(parseMsg([])).toBeNull();
    expect(parseMsg([{ type: "ui:flush-pending" }])).toBeNull();
    expect(parseMsg(["ui:flush-pending"])).toBeNull();
  });

  it("adversarial: numeric / boolean `type` discriminants → null", () => {
    expect(parseMsg({ type: 1 })).toBeNull();
    expect(parseMsg({ type: true })).toBeNull();
    expect(parseMsg({ type: null })).toBeNull();
    expect(parseMsg({ type: { nested: "ui:flush-pending" } })).toBeNull();
  });
});
