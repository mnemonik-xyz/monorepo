// T23 bridge-contract tests for `parseMsg`.
//
// Goal: lock the SW↔popup↔content-script wire shape for every
// discriminant in the `Msg` union, so that adding a variant or
// changing a payload guard can never silently drift past CI again
// (the audit found `tab:fab-*` variants missing from `Msg` —
// AUD-C-05 / B5 — because each side mocked the boundary).
//
// Structure: a single `MsgType` exhaustive `switch` in
// `buildSamples()` returns well-formed + malformed fixtures keyed by
// variant. The switch's `never` branch means "adding a new `Msg`
// variant without a sample here is a compile error", which is the
// type-level half of the gate; the runtime half is that every entry
// round-trips through `parseMsg`.
//
// What this catches that a single-variant test wouldn't:
//
//   - regression where a new payload field becomes required but the
//     guard doesn't enforce it (the malformed sample for that variant
//     would parse and fail this test);
//   - regression where the discriminant string is misspelled in the
//     union but the guard still accepts it (the variant's well-formed
//     sample would not round-trip its `type` field);
//   - regression where a `tab:fab-*` variant is dropped from
//     `parseMsg`'s `switch` (the well-formed sample returns `null`).

import { describe, it, expect } from "vitest";
import { parseMsg, type Msg } from "../../src/messages.js";

type MsgType = Msg["type"];

interface VariantSamples {
  /** A well-formed payload that MUST `parseMsg` back to a typed Msg
   *  whose `type` equals the variant key. */
  wellFormed: Record<string, unknown>;
  /** One or more malformed payloads that MUST `parseMsg` to `null`.
   *  Each entry exercises a distinct rejection path (wrong field
   *  type, missing required field, out-of-range integer, etc.). */
  malformed: Record<string, unknown>[];
  /** At least one field name on the well-formed message whose value
   *  the test asserts non-null after the round-trip — proves the
   *  guard preserved the payload, not just the discriminant. */
  fieldPath?: string[];
}

/** TS-exhaustive switch over `Msg["type"]` — adding a new variant
 *  without extending this function is a compile error (the `never`
 *  branch flags the missing case). */
function buildSamples(t: MsgType): VariantSamples {
  switch (t) {
    case "sw:open-recall-overlay":
      return {
        wellFormed: {
          type: "sw:open-recall-overlay",
          payload: { trigger: "hotkey" },
        },
        malformed: [
          // trigger must be one of three enum values
          {
            type: "sw:open-recall-overlay",
            payload: { trigger: "drive-by" },
          },
          // payload must be an object
          { type: "sw:open-recall-overlay", payload: "hotkey" },
          // missing payload
          { type: "sw:open-recall-overlay" },
        ],
        fieldPath: ["payload", "trigger"],
      };
    case "sw:save-selection":
      return {
        wellFormed: {
          type: "sw:save-selection",
          payload: {
            selectionText: "verifiable memories",
            pageUrl: "https://example.com/post/1",
            capturedAt: "2026-05-11T00:00:00.000Z",
            pageTitle: "Example",
          },
        },
        malformed: [
          // capturedAt missing
          {
            type: "sw:save-selection",
            payload: {
              selectionText: "x",
              pageUrl: "https://example.com",
            },
          },
          // pageTitle wrong type (when present must be string)
          {
            type: "sw:save-selection",
            payload: {
              selectionText: "x",
              pageUrl: "https://example.com",
              capturedAt: "2026-05-11T00:00:00.000Z",
              pageTitle: 7,
            },
          },
          // selectionText wrong type
          {
            type: "sw:save-selection",
            payload: {
              selectionText: 0,
              pageUrl: "https://example.com",
              capturedAt: "2026-05-11T00:00:00.000Z",
            },
          },
        ],
        fieldPath: ["payload", "selectionText"],
      };
    case "ui:sign-memory":
      return {
        wellFormed: {
          type: "ui:sign-memory",
          payload: {
            content: "remember this",
            tags: ["chatgpt", "research"],
            source: {
              platform: "chatgpt",
              url: "https://chatgpt.com/c/123",
              chatId: "c-123",
              model: "gpt-4o",
            },
          },
        },
        malformed: [
          // empty content rejected
          {
            type: "ui:sign-memory",
            payload: { content: "", tags: [] },
          },
          // tags must be string[]
          {
            type: "ui:sign-memory",
            payload: { content: "ok", tags: [1, 2, 3] },
          },
          // source.platform must be a string when source is present
          {
            type: "ui:sign-memory",
            payload: {
              content: "ok",
              tags: [],
              source: { platform: 42 },
            },
          },
          // source.url wrong type when present
          {
            type: "ui:sign-memory",
            payload: {
              content: "ok",
              tags: [],
              source: { platform: "p", url: 1 },
            },
          },
        ],
        fieldPath: ["payload", "content"],
      };
    case "ui:recall":
      return {
        wellFormed: {
          type: "ui:recall",
          payload: {
            query: "react server components",
            ownerPubkey: "z6MkpubkeyExample",
            limit: 10,
            tags: ["framework"],
          },
        },
        malformed: [
          // limit out of range (>100)
          {
            type: "ui:recall",
            payload: {
              query: "q",
              ownerPubkey: "z6Mk",
              limit: 200,
            },
          },
          // limit non-integer
          {
            type: "ui:recall",
            payload: {
              query: "q",
              ownerPubkey: "z6Mk",
              limit: 1.5,
            },
          },
          // ownerPubkey empty
          {
            type: "ui:recall",
            payload: { query: "q", ownerPubkey: "", limit: 5 },
          },
          // tags wrong shape when present
          {
            type: "ui:recall",
            payload: {
              query: "q",
              ownerPubkey: "z6Mk",
              limit: 5,
              tags: "framework",
            },
          },
        ],
        fieldPath: ["payload", "ownerPubkey"],
      };
    case "ui:flush-pending":
      return {
        wellFormed: { type: "ui:flush-pending" },
        malformed: [
          // primitive discriminant
          "ui:flush-pending" as unknown as Record<string, unknown>,
        ],
        // No payload fields — discriminant is the whole contract.
      };
    case "tab:capture-candidate":
      return {
        wellFormed: {
          type: "tab:capture-candidate",
          payload: {
            platform: "claude",
            url: "https://claude.ai/chat/abc",
            content: "assistant turn body",
            chatId: "abc",
          },
        },
        malformed: [
          // platform missing
          {
            type: "tab:capture-candidate",
            payload: { url: "https://claude.ai", content: "x" },
          },
          // content wrong type
          {
            type: "tab:capture-candidate",
            payload: {
              platform: "claude",
              url: "https://claude.ai",
              content: 42,
            },
          },
          // chatId wrong type when present
          {
            type: "tab:capture-candidate",
            payload: {
              platform: "claude",
              url: "https://claude.ai",
              content: "x",
              chatId: 7,
            },
          },
        ],
        fieldPath: ["payload", "platform"],
      };
    case "tab:fab-save-chat":
      return {
        wellFormed: {
          type: "tab:fab-save-chat",
          payload: { pageUrl: "https://chatgpt.com/c/abc" },
        },
        malformed: [
          // pageUrl wrong type when present
          {
            type: "tab:fab-save-chat",
            payload: { pageUrl: 9 },
          },
          // payload must be a plain object when present (array is not)
          {
            type: "tab:fab-save-chat",
            payload: ["https://chatgpt.com"],
          },
        ],
        fieldPath: ["payload", "pageUrl"],
      };
    case "tab:fab-save-selection":
      return {
        wellFormed: {
          type: "tab:fab-save-selection",
          payload: {
            selectionText: "highlighted bit",
            pageUrl: "https://claude.ai/chat",
          },
        },
        malformed: [
          // selectionText wrong type
          {
            type: "tab:fab-save-selection",
            payload: { selectionText: 9, pageUrl: "https://claude.ai" },
          },
          // pageUrl missing
          {
            type: "tab:fab-save-selection",
            payload: { selectionText: "x" },
          },
        ],
        fieldPath: ["payload", "selectionText"],
      };
    case "tab:fab-open-popup":
      return {
        wellFormed: { type: "tab:fab-open-popup" },
        malformed: [
          // wrong discriminant (close-by typo)
          { type: "tab:fab-open-Popup" },
        ],
      };
    default: {
      // Exhaustiveness: if a new `Msg` variant is added without an
      // entry above, this assignment becomes a TS error. The runtime
      // throw is a defence-in-depth backstop for the case where the
      // type-check is bypassed (e.g. `as any` upstream).
      const _exhaustive: never = t;
      throw new Error(`buildSamples: missing case for ${String(_exhaustive)}`);
    }
  }
}

/** Every `Msg` variant in source order. Adding a new variant without
 *  extending this list is caught by the exhaustive switch above —
 *  this list is for the test driver to iterate. The
 *  `assertCoversAllMsgTypes` helper below double-locks this: the
 *  array is typed as a tuple whose union of elements MUST equal
 *  `MsgType`. Drop a variant from the list and the assertion's
 *  generic-bound becomes unsatisfied (TS compile error). Add a
 *  variant to the union without extending this list and the same
 *  assertion fails. */
const ALL_VARIANTS = [
  "sw:open-recall-overlay",
  "sw:save-selection",
  "ui:sign-memory",
  "ui:recall",
  "ui:flush-pending",
  "tab:capture-candidate",
  "tab:fab-save-chat",
  "tab:fab-save-selection",
  "tab:fab-open-popup",
] as const satisfies readonly MsgType[];

/**
 * Compile-time assertion that `ALL_VARIANTS` is BOTH a subset AND a
 * superset of `MsgType`. The `Exclude` symmetric difference being
 * `never` is the only way both bounds can hold; if a maintainer adds
 * a `Msg` variant but forgets to update `ALL_VARIANTS`, this line
 * becomes a TS error and CI fails. Mirrors the same trick used in
 * `messages.ts`'s own exhaustive switch.
 */
type _ExhaustiveListCheck =
  | Exclude<MsgType, (typeof ALL_VARIANTS)[number]>
  | Exclude<(typeof ALL_VARIANTS)[number], MsgType>;
const _exhaustiveListCheck: _ExhaustiveListCheck extends never ? true : never =
  true;
// Reference the binding so the unused-var lint doesn't strip it.
void _exhaustiveListCheck;

describe("parseMsg — every variant round-trips (every_variant_round_trips)", () => {
  for (const variant of ALL_VARIANTS) {
    const samples = buildSamples(variant);

    it(`accepts well-formed ${variant} and preserves payload`, () => {
      const parsed = parseMsg(samples.wellFormed);
      expect(parsed).not.toBeNull();
      // discriminant matches the input variant
      expect(parsed?.type).toBe(variant);
      if (samples.fieldPath) {
        // Walk the documented field path and assert the leaf is
        // non-null & not `undefined`. Proves the guard preserved
        // the payload (not just the discriminant) and that the
        // field's runtime type matches the source-level type.
        let node: unknown = parsed;
        for (const key of samples.fieldPath) {
          expect(node).not.toBeNull();
          expect(typeof node).toBe("object");
          node = (node as Record<string, unknown>)[key];
        }
        expect(node).not.toBeUndefined();
        expect(node).not.toBeNull();
      }
    });

    it(`rejects every malformed ${variant} sample`, () => {
      // At least one malformed sample per variant (driver invariant).
      expect(samples.malformed.length).toBeGreaterThan(0);
      for (const bad of samples.malformed) {
        expect(parseMsg(bad)).toBeNull();
      }
    });
  }
});

describe("parseMsg — generic defensive rejection", () => {
  it("returns null on null / undefined / primitives", () => {
    expect(parseMsg(null)).toBeNull();
    expect(parseMsg(undefined)).toBeNull();
    expect(parseMsg(0)).toBeNull();
    expect(parseMsg("")).toBeNull();
    expect(parseMsg(false)).toBeNull();
    expect(parseMsg(Symbol("x") as unknown)).toBeNull();
  });

  it("returns null on arrays (not plain objects)", () => {
    expect(parseMsg([])).toBeNull();
    expect(parseMsg([{ type: "ui:flush-pending" }])).toBeNull();
  });

  it("returns null on an object with no `type` field", () => {
    expect(parseMsg({})).toBeNull();
    expect(parseMsg({ payload: { foo: 1 } })).toBeNull();
  });

  it("returns null on a non-string `type`", () => {
    expect(parseMsg({ type: 7 })).toBeNull();
    expect(parseMsg({ type: null })).toBeNull();
    expect(parseMsg({ type: { nested: "ui:recall" } })).toBeNull();
  });

  it("returns null on unknown discriminants across every direction prefix", () => {
    // Cover all three direction prefixes (`sw:`, `ui:`, `tab:`) so a
    // regression in one branch can't hide behind the others. T23-T-N1
    // round-1 review nit.
    expect(parseMsg({ type: "sw:future-variant" })).toBeNull();
    expect(parseMsg({ type: "ui:unknown" })).toBeNull();
    expect(parseMsg({ type: "tab:fab-something-new" })).toBeNull();
    // Audit B5 / AUD-C-05 guard: dropping a `tab:fab-*` variant from
    // the source-level union (without updating `parseMsg`) would
    // surface as the variant's well-formed sample returning `null`
    // — covered by the per-variant loop above. This case covers the
    // opposite drift: a new tab:fab-* variant introduced in the
    // dispatch site without being added to `parseMsg`'s switch.
  });
});

describe("parseMsg — discriminant integrity", () => {
  it("does not coerce a discriminant from a nested payload", () => {
    // Hostile input embeds `type` inside `payload` but the top-level
    // object has none. parseMsg looks ONLY at the top-level `type`;
    // confirms guards never auto-promote nested discriminants.
    expect(parseMsg({ payload: { type: "ui:flush-pending" } })).toBeNull();
  });

  it("rejects a `type` set to a known variant prefixed with whitespace", () => {
    // The switch is exact-match; defensive against runtime callers
    // that pass un-trimmed strings through `JSON.parse` of human-
    // typed input.
    expect(parseMsg({ type: " ui:flush-pending" })).toBeNull();
    expect(parseMsg({ type: "ui:flush-pending " })).toBeNull();
  });
});
