#!/usr/bin/env node
/**
 * Mine work/* feature specs into a typed roadmap artifact for the webapp.
 *
 * Input:  ../work/<feature>/ and ../work/completed/<feature>/
 *           ├─ tech-spec.md or user-spec.md (frontmatter + Markdown body)
 *           └─ tasks/*.md         (status: done | in_progress | pending | planned)
 *
 * Output: src/lib/roadmap.generated.ts — a `RoadmapItem[]` const consumed by
 *         the /roadmap page.
 *
 * Bucketing:
 *   - in `work/completed/`                         → "shipped"
 *   - all task statuses == "done"                  → "shipped"
 *   - any task in_progress OR mix done + pending   → "in_progress"
 *   - no tasks dir OR no done tasks                → "planned"
 *
 * This runs as the npm `prebuild` step. The output is committed so dev/test
 * doesn't need to re-run the script — CI regenerates on build to keep prod
 * in sync.
 */

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "../..");
const WORK_DIR = resolve(REPO_ROOT, "work");
const OUT_FILE = resolve(__dirname, "../src/lib/roadmap.generated.ts");

// ── Helpers ────────────────────────────────────────────────────────────────

function listFeatureDirs() {
  const dirs = [];
  for (const entry of readdirSync(WORK_DIR)) {
    const path = join(WORK_DIR, entry);
    if (!statSync(path).isDirectory()) continue;
    if (entry === "completed") {
      for (const sub of readdirSync(path)) {
        const subPath = join(path, sub);
        if (statSync(subPath).isDirectory()) {
          dirs.push({ name: sub, path: subPath, completed: true });
        }
      }
    } else {
      dirs.push({ name: entry, path, completed: false });
    }
  }
  return dirs;
}

function tryRead(path) {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return null;
  }
}

function parseFrontmatter(md) {
  if (!md.startsWith("---")) return { frontmatter: {}, body: md };
  const end = md.indexOf("\n---", 3);
  if (end === -1) return { frontmatter: {}, body: md };
  const fm = {};
  for (const line of md.slice(3, end).split("\n")) {
    const m = line.match(/^([a-zA-Z_]+):\s*(.*)$/);
    if (m) fm[m[1]] = m[2].trim();
  }
  return { frontmatter: fm, body: md.slice(end + 4) };
}

function extractTitle(body, fallback) {
  // Prefer the first `# Tech Spec: X` / `# Title` heading after frontmatter.
  const m = body.match(/^#\s+(?:Tech Spec:|User Spec:)?\s*(.+)$/m);
  if (m) return m[1].trim();
  return prettifyName(fallback);
}

function extractDescription(body) {
  // First paragraph after the first H2 (## Solution / ## Summary / etc).
  // Falls back to the first prose paragraph if no H2 exists.
  // Lookahead so the heading text itself isn't consumed by the split.
  const afterH2 = body.split(/\n##\s+(?=\w)/)[1];
  const block = afterH2 ?? body;
  // Drop the heading line we landed on, keep the body below it.
  const blockBody = block.replace(/^[^\n]*\n+/, "");
  const paragraphs = blockBody
    .split(/\n\n+/)
    .map((p) => p.trim())
    .filter((p) => p && !p.startsWith("#") && !p.startsWith("```"));
  const raw = paragraphs[0] ?? "";
  // Strip markdown emphasis / code / links, leading bullets, collapse ws.
  const cleaned = raw
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/[*_]{1,2}([^*_]+)[*_]{1,2}/g, "$1")
    .replace(/^\s*[-*]\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
  if (cleaned.length <= 220) return cleaned;
  return cleaned.slice(0, 217).replace(/\s+\S*$/, "") + "…";
}

function prettifyName(slug) {
  return slug
    .split("-")
    .map((w) => (w === "mnemonic" ? "Mnemonic" : w[0]?.toUpperCase() + w.slice(1)))
    .join(" ");
}

function classify(feature) {
  if (feature.completed) return classifyShipped(feature);
  const tasksDir = join(feature.path, "tasks");
  let total = 0;
  let done = 0;
  let inProgress = 0;
  try {
    for (const f of readdirSync(tasksDir)) {
      if (!f.endsWith(".md")) continue;
      total++;
      const head = readFileSync(join(tasksDir, f), "utf8").slice(0, 400);
      const m = head.match(/^status:\s*(\w+)/m);
      if (!m) continue;
      if (m[1] === "done") done++;
      else if (m[1] === "in_progress") inProgress++;
    }
  } catch {
    // no tasks dir
  }
  if (total === 0) return "planned";
  if (done === total) return "shipped";
  if (inProgress > 0 || done > 0) return "in_progress";
  return "planned";
}

function classifyShipped(_feature) {
  return "shipped";
}

// ── Mine ───────────────────────────────────────────────────────────────────

const features = listFeatureDirs();
const items = [];
for (const feature of features) {
  const spec =
    tryRead(join(feature.path, "tech-spec.md")) ??
    tryRead(join(feature.path, "user-spec.md"));
  if (!spec) {
    // No spec at all (e.g. seed-all-docs is just a logs dir). Skip silently.
    continue;
  }
  const { body } = parseFrontmatter(spec);
  const title = extractTitle(body, feature.name);
  const description = extractDescription(body);
  const status = classify(feature);
  const sourcePath = relative(REPO_ROOT, feature.path) + "/";
  items.push({ id: feature.name, title, description, status, sourcePath });
}

// Deterministic order: shipped → in_progress → planned, then alphabetical.
const STATUS_RANK = { shipped: 0, in_progress: 1, planned: 2 };
items.sort((a, b) => {
  const r = STATUS_RANK[a.status] - STATUS_RANK[b.status];
  return r !== 0 ? r : a.title.localeCompare(b.title);
});

// ── Emit ───────────────────────────────────────────────────────────────────

const banner = `// AUTO-GENERATED by webapp/scripts/generate-roadmap.mjs. Do not edit.
// Source: work/<feature>/{tech-spec.md,user-spec.md,tasks/*.md}
// Run \`npm run roadmap\` in webapp/ to regenerate.
`;

const tsLiteral =
  banner +
  `
export type RoadmapStatus = "shipped" | "in_progress" | "planned";

export interface RoadmapItem {
  id: string;
  title: string;
  description: string;
  status: RoadmapStatus;
  sourcePath: string;
}

export const ROADMAP_ITEMS: readonly RoadmapItem[] = ${JSON.stringify(
    items,
    null,
    2,
  )} as const;
`;

writeFileSync(OUT_FILE, tsLiteral);
console.log(
  `wrote ${items.length} roadmap items → ${relative(REPO_ROOT, OUT_FILE)}`,
);
