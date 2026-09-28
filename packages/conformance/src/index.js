/**
 * @mnemonik-xyz/conformance — compiled ESM entry point.
 *
 * Generated from src/index.ts. Re-exports vectors and helpers for
 * third-party conformance harnesses.
 */

import { createRequire } from "module";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Load vectors relative to this file (works from src/ or dist/).
const vectorsPath = join(__dirname, "..", "vectors-a2a-v1.0.0.json");
const require = createRequire(import.meta.url);
const vectors = require(vectorsPath);

export { vectors };

/**
 * Load all vectors typed.
 * @returns {import("./index.js").A2aVector[]}
 */
export function loadVectors() {
  return vectors;
}

/**
 * Return vectors filtered by kind prefix.
 * @param {"message" | "task" | "artifact"} kind
 * @returns {import("./index.js").A2aVector[]}
 */
export function vectorsForKind(kind) {
  return loadVectors().filter((v) => v.name.startsWith(kind));
}
