import { describe, expect, it } from "vitest";
import { place } from "./Popover";

const r = (left: number, top: number, w = 40, h = 40) => ({ left, top, right: left + w, bottom: top + h });

describe("place (popover positioning)", () => {
  it("opens below, end-aligned, when it fits", () => {
    const p = place(r(500, 100), { width: 200, height: 150 }, 1440, 900);
    expect(p).toMatchObject({ side: "bottom", align: "end", top: 144, left: 340 });
  });
  it("flips above near the bottom edge", () => {
    const p = place(r(500, 820), { width: 200, height: 150 }, 1440, 900);
    expect(p.side).toBe("top");
    expect(p.top).toBe(820 - 4 - 150);
  });
  it("flips to start alignment near the left edge", () => {
    const p = place(r(10, 100), { width: 200, height: 100 }, 390, 844);
    expect(p.align).toBe("start");
    expect(p.left).toBe(10);
  });
  it("stays inside the viewport near the right edge", () => {
    const p = place(r(360, 100, 30, 30), { width: 220, height: 100 }, 390, 844, "bottom", "start");
    expect(p.align).toBe("end");
    expect(p.left + 220).toBeLessThanOrEqual(390 - 8);
  });
  it("limits height to the room on the chosen side", () => {
    const p = place(r(100, 400), { width: 200, height: 2000 }, 1440, 900);
    expect(p.maxHeight).toBeLessThan(900);
    expect(p.top + p.maxHeight).toBeLessThanOrEqual(900);
  });
});
