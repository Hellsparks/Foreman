import { describe, expect, it } from "vitest";
import { draftToForm, emptyDraft, fitColours, normHex, toggleModifier } from "../api/filaments";

const reasons = new Map([
  ["CF", "carbon fibre"],
  ["GF", "glass fibre"],
  ["AF", "aramid fibre"],
  ["Metal", "metal-filled"],
  ["Glow", "glow pigment"],
]);

describe("toggleModifier", () => {
  it("ticks abrasive for CF / GF / AF / Metal / Glow", () => {
    for (const m of ["CF", "GF", "AF", "Metal", "Glow"]) {
      expect(toggleModifier([], m, false, reasons)).toEqual({ mods: [m], abrasive: true });
    }
  });
  it("leaves it for other modifiers", () => {
    expect(toggleModifier([], "Silk", false, reasons).abrasive).toBe(false);
  });
  it("unticks when the last abrasive modifier goes", () => {
    expect(toggleModifier(["CF"], "CF", true, reasons)).toEqual({ mods: [], abrasive: false });
  });
  it("keeps the user's override while the abrasive-ness does not change", () => {
    // CF picked, user unticked abrasive, then adds HF: stays unticked.
    expect(toggleModifier(["CF"], "HF", false, reasons)).toEqual({ mods: ["CF", "HF"], abrasive: false });
    // User ticked it on a plain Silk: adding Matte keeps it.
    expect(toggleModifier(["Silk"], "Matte", true, reasons).abrasive).toBe(true);
  });
});

describe("colours", () => {
  it("fits the colour count to the kind", () => {
    expect(fitColours("single", ["#111111", "#222222"])).toEqual(["#111111"]);
    expect(fitColours("coaxial", ["#111111"])).toHaveLength(2);
    expect(fitColours("coaxial", Array(5).fill("#111111"))).toHaveLength(3);
    expect(fitColours("longitudinal", Array(8).fill("#111111"))).toHaveLength(6);
  });
  it("normalises hex", () => {
    expect(normHex("1F8A8A")).toBe("#1f8a8a");
    expect(normHex("#abc")).toBeNull();
  });
});

describe("draftToForm", () => {
  it("matches an existing vendor by name or names a new one", () => {
    const d = { ...emptyDraft(), vendor: "polymaker", name: " PLA Teal ", td: "4.1" };
    const vendors = [{ id: 3, name: "Polymaker", empty_spool_weight: 230, website: null, comment: null, filament_count: 1 }];
    const f = draftToForm(d, vendors);
    expect(f.vendor_id).toBe(3);
    expect(f.vendor_name).toBeNull();
    expect(f.name).toBe("PLA Teal");
    expect(f.hue_td).toBe(4.1);
    const g = draftToForm({ ...d, vendor: "Sunlu", kind: "longitudinal", colours: ["#111111", "#222222"] }, vendors);
    expect(g.vendor_id).toBeNull();
    expect(g.vendor_name).toBe("Sunlu");
    expect(g.hue_td).toBeNull(); // TD is single-colour only
  });
});
