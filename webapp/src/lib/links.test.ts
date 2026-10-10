import { describe, expect, it } from "vitest";
import { arweaveDataUrl } from "./links";

describe("external transaction links", () => {
  it("routes production data item ids to the Arweave gateway", () => {
    expect(arweaveDataUrl("u6pIaLEoaJ8bOnTUCTj2LSiv0IAMp-QbMhcsOcrmBXA")).toBe(
      "https://arweave.net/u6pIaLEoaJ8bOnTUCTj2LSiv0IAMp-QbMhcsOcrmBXA",
    );
  });

  it("does not link synthetic local ids", () => {
    expect(arweaveDataUrl("local:memory")).toBeNull();
    expect(arweaveDataUrl("")).toBeNull();
  });
});
