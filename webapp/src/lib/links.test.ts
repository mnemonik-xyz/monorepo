import { describe, expect, it } from "vitest";
import { irysDataUrl, solanaTxUrl } from "./links";

describe("external transaction links", () => {
  it("routes production data item ids to the Irys gateway", () => {
    expect(irysDataUrl("CubJzDPLBaLF7fo67KB9RWXgex1QV8RsaWPWWkEaLoT3")).toBe(
      "https://gateway.irys.xyz/CubJzDPLBaLF7fo67KB9RWXgex1QV8RsaWPWWkEaLoT3",
    );
  });

  it("does not link synthetic local ids", () => {
    expect(irysDataUrl("local:memory")).toBeNull();
    expect(solanaTxUrl("local:memory")).toBeNull();
  });
});
