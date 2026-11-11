import { describe, expect, it } from "vitest";
import { hostLabel, safeHref } from "./ExternalLink";

describe("safeHref", () => {
  it("keeps http(s) links and adds https:// to bare hosts", () => {
    expect(safeHref("https://www.sunlu.com")).toBe("https://www.sunlu.com/");
    expect(safeHref("www.3djake.no/x")).toBe("https://www.3djake.no/x");
  });
  it("refuses other schemes and empty values", () => {
    expect(safeHref("javascript:alert(1)")).toBeNull();
    expect(safeHref("file:///etc/passwd")).toBeNull();
    expect(safeHref("  ")).toBeNull();
    expect(safeHref(null)).toBeNull();
  });
  it("labels by host", () => {
    expect(hostLabel("https://www.sunlu.com/a/b")).toBe("sunlu.com");
  });
});
