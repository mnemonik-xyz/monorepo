import { describe, expect, it } from "vitest";
import { sampleArtifacts, sampleTimeline, type TimeRange } from "./ledger";

describe("sampleTimeline", () => {
  const cases: Array<[TimeRange, number]> = [
    ["30d", 30],
    ["90d", 90],
    ["12m", 12],
  ];

  it.each(cases)("has the right bucket count for %s", (range, count) => {
    expect(sampleTimeline(range).buckets).toHaveLength(count);
  });

  it.each(cases)(
    "reports totals matching the sum of buckets for %s",
    (range) => {
      const t = sampleTimeline(range);
      const node = t.buckets.reduce((s, b) => s + b.on_node, 0);
      const chain = t.buckets.reduce((s, b) => s + b.on_chain, 0);
      expect(t.total_on_node).toBe(node);
      expect(t.total_on_chain).toBe(chain);
    },
  );

  it("is flagged as sample data", () => {
    expect(sampleTimeline("30d").sample).toBe(true);
  });

  it("emits ascending ISO day-granularity dates", () => {
    const buckets = sampleTimeline("90d").buckets;
    for (const b of buckets) {
      expect(b.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    }
    const dates = buckets.map((b) => b.date);
    expect([...dates].sort()).toEqual(dates);
  });

  it("is deterministic across calls (no wall-clock dependence)", () => {
    expect(sampleTimeline("12m")).toEqual(sampleTimeline("12m"));
  });
});

describe("sampleArtifacts", () => {
  it("mixes both write modes", () => {
    const modes = new Set(sampleArtifacts().map((a) => a.write_mode));
    expect(modes).toContain("local");
    expect(modes).toContain("participate");
  });

  it("includes both real and local: / null anchors", () => {
    const rows = sampleArtifacts();
    const hasReal = rows.some(
      (a) => a.solana_tx !== null && !a.solana_tx.startsWith("local:"),
    );
    const hasUnanchored = rows.some(
      (a) => a.solana_tx === null || a.solana_tx.startsWith("local:"),
    );
    expect(hasReal).toBe(true);
    expect(hasUnanchored).toBe(true);
  });

  it("uses blake3-style 64-hex hashes and ISO timestamps with varied tags", () => {
    const rows = sampleArtifacts();
    for (const a of rows) {
      expect(a.content_hash).toMatch(/^[0-9a-f]{64}$/);
      expect(a.created_at).toMatch(
        /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/,
      );
      expect(a.tags.length).toBeGreaterThan(0);
    }
    const allTags = new Set(rows.flatMap((a) => a.tags));
    expect(allTags.size).toBeGreaterThan(1);
  });

  it("is deterministic across calls", () => {
    expect(sampleArtifacts()).toEqual(sampleArtifacts());
  });
});
