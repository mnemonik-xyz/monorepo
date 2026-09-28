// RFC 8785 JSON Canonicalization Scheme — restricted value space.
//
// Allowed types: strings, safe-range integers, booleans, objects, arrays.
// Rejected: floats, unsafe integers, null, undefined, NaN, Infinity, lone
// surrogates, non-plain objects.
//
// Key ordering: Object.keys().sort() — JavaScript's default sort is UTF-16
// code-unit lexicographic order, which is exactly what RFC 8785 requires.
//
// The shortcut from the tech spec: with the restricted value space above,
// JCS reduces to sort-keys-recursively + JSON.stringify with no spacing.
// Tests validate against published RFC 8785 vectors to prove it.

export function jcsStringify(value: unknown): string {
  return JSON.stringify(canonicalize(value));
}

export function jcsBytes(value: unknown): Uint8Array {
  return new TextEncoder().encode(jcsStringify(value));
}

function canonicalize(value: unknown): unknown {
  if (typeof value === "boolean") return value;

  if (typeof value === "string") {
    assertNoLoneSurrogates(value);
    return value;
  }

  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new JcsError(`non-finite number: ${value}`);
    }
    if (!Number.isInteger(value)) {
      throw new JcsError(`non-integer number: ${value}`);
    }
    if (!Number.isSafeInteger(value)) {
      throw new JcsError(`unsafe integer (outside ±2^53-1): ${value}`);
    }
    return value;
  }

  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }

  if (value !== null && typeof value === "object") {
    // Reject class instances and other non-plain objects.
    const proto = Object.getPrototypeOf(value) as unknown;
    if (proto !== Object.prototype && proto !== null) {
      throw new JcsError(`non-plain object (${(value as object).constructor?.name ?? "unknown"})`);
    }
    const obj = value as Record<string, unknown>;
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(obj).sort()) {
      sorted[key] = canonicalize(obj[key]);
    }
    return sorted;
  }

  // null, undefined, symbol, function, bigint
  const label =
    value === null
      ? "null"
      : value === undefined
        ? "undefined"
        : typeof value;
  throw new JcsError(`unsupported value type: ${label}`);
}

function assertNoLoneSurrogates(s: string): void {
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff) {
      // High surrogate — must be followed by a low surrogate.
      if (i + 1 >= s.length) {
        throw new JcsError(`lone high surrogate at end of string`);
      }
      const next = s.charCodeAt(i + 1);
      if (next < 0xdc00 || next > 0xdfff) {
        throw new JcsError(`lone high surrogate at position ${i}`);
      }
      i++;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      throw new JcsError(`lone low surrogate at position ${i}`);
    }
  }
}

export class JcsError extends Error {
  override name = "JcsError";
}
