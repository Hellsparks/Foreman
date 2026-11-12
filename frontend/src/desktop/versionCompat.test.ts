import { describe, expect, it } from "vitest";
import { versionMismatch } from "./versionCompat";

describe("versionMismatch", () => {
  it("is quiet for equal versions and patch differences", () => {
    expect(versionMismatch("0.1.0", "0.1.0")).toBeNull();
    expect(versionMismatch("0.1.0", "0.1.9")).toBeNull();
    expect(versionMismatch("1.4.7", "1.4.2")).toBeNull();
    expect(versionMismatch("v2.0.0", "2.0.3-beta")).toBeNull();
  });

  it("warns on a minor difference and names the older side", () => {
    const a = versionMismatch("0.2.0", "0.1.5");
    expect(a?.update).toBe("server");
    expect(a?.message).toContain("Update the server to 0.2");
    const b = versionMismatch("0.1.5", "0.2.0");
    expect(b?.update).toBe("app");
    expect(b?.message).toContain("Update this app");
  });

  it("warns on a major difference", () => {
    expect(versionMismatch("2.0.0", "1.9.9")?.update).toBe("server");
    expect(versionMismatch("1.9.9", "2.0.0")?.update).toBe("app");
  });

  it("ignores versions it cannot read", () => {
    expect(versionMismatch("0.1.0", "dev")).toBeNull();
    expect(versionMismatch("", "0.2.0")).toBeNull();
  });
});
