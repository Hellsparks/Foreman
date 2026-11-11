import { describe, expect, it } from "vitest";
import { axesFrom, deg, EMPTY_AXES, EMPTY_PLANES, planesFrom, shrinkInput } from "./calLogic";

describe("calibration helpers", () => {
  it("needs a complete XY plane", () => {
    expect(planesFrom(EMPTY_PLANES, false)).toBeNull();
    const f = { ...EMPTY_PLANES, XY: ["140.4", "142.8", "99.8"] as [string, string, string] };
    expect(planesFrom(f, false)).toEqual([{ plane: "XY", ac: 140.4, bd: 142.8, ad: 99.8 }]);
    // Incomplete XZ is skipped, comma decimals accepted.
    const g = { ...f, XZ: ["141,6", "141.4", ""] as [string, string, string], YZ: ["142.4", "140.5", "99.5"] as [string, string, string] };
    expect(planesFrom(g, true)?.map((p) => p.plane)).toEqual(["XY", "YZ"]);
  });

  it("formats degrees with a sign", () => {
    expect(deg(-0.9711)).toBe("−0.97°");
    expect(deg(0.314)).toBe("+0.31°");
  });

  it("builds shrinkage input per axis and back", () => {
    expect(shrinkInput(EMPTY_AXES)).toBeNull();
    const a = { ...EMPTY_AXES, x: { nominal: "100", measured: "99.5" }, z: { nominal: "50", measured: "49.9" } };
    const i = shrinkInput(a);
    expect(i).toEqual({ x: [{ nominal: 100, readings: [99.5] }], y: [], z: [{ nominal: 50, readings: [49.9] }], cal_err: null });
    expect(axesFrom(i!)).toEqual({ x: a.x, y: { nominal: "100", measured: "" }, z: a.z });
  });
});
