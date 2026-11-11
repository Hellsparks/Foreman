import { describe, expect, it } from "vitest";
import type { ExportRow } from "../api/types/ExportRow";
import { exportSummary, luminance, matches, pctX, pctY, plural } from "./exportLogic";

function row(id: number, over: Partial<ExportRow> = {}): ExportRow {
  return {
    id,
    name: "Black",
    vendor: "Frem3D",
    material: "ASA",
    colours: ["000000"],
    hue_td: null,
    owned: true,
    orca: {
      filament_id: id,
      name: "x",
      file: "x.json",
      inherits: "Generic ASA @System",
      orca_filament_id: "OFLPAxz3",
      bambu_filament_id: "GFB98",
      shrinkage_xy: null,
      shrinkage_z: null,
      flow_ratio: null,
      multi_colour: false,
      notes: [],
    },
    hueforge_skip: "missing_td",
    swatch_line1: "Frem3D ASA",
    swatch_line2: "Black",
    ...over,
  };
}

describe("exportLogic", () => {
  it("places text in plate percentages", () => {
    expect(pctX(8.5)).toBeCloseTo(11.33, 2);
    expect(100 - pctY(34)).toBeCloseTo(15, 5);
  });
  it("tells dark from light filaments", () => {
    expect(luminance("000000")).toBe(0);
    expect(luminance("FFFFFF")).toBe(1);
    expect(luminance("nope")).toBe(0.5);
  });
  it("searches vendor, name, material and id", () => {
    expect(matches(row(11), "frem")).toBe(true);
    expect(matches(row(11), "#11")).toBe(true);
    expect(matches(row(11), "petg")).toBe(false);
  });
  it("summarises the selection or everything", () => {
    const rows = [
      row(1, { hueforge_skip: null, hue_td: 4.5, orca: { ...row(1).orca, shrinkage_xy: 98.85 } }),
      row(2, { hueforge_skip: "multi_colour", colours: ["FFFFFF", "8B4513"], orca: { ...row(2).orca, multi_colour: true } }),
      row(3),
    ];
    const all = exportSummary(rows, new Set());
    expect(all).toMatchObject({ scope: 3, hue: 1, hueMulti: 1, withShrink: 1, multiOrca: 1 });
    expect(all.missingTd.map((r) => r.id)).toEqual([3]);
    expect(exportSummary(rows, new Set([3])).scope).toBe(1);
  });
  it("pluralises", () => {
    expect(plural(1, "profile")).toBe("1 profile");
    expect(plural(2, "swatch", "swatches")).toBe("2 swatches");
  });
});
