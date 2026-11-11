import { describe, expect, it } from "vitest";
import { autoKey, choicesFor, groupMaterials, groupSlots, memberId, type PickedFile } from "./grouping";

const slot = (index: number, material: string, colour: string, grams: number, label = `T${index}`) => ({
  index, tool: index, label, material, colour, grams, mm: null, preset: null, spool_id: null, tray: null,
});

const files = (): PickedFile[] => [
  { file_id: 1, name: "housing_top.gcode", slot_kind: "tool", copies: 2, slots: [slot(0, "PLA", "#F4F4EF", 64)] },
  { file_id: 2, name: "lid.3mf", slot_kind: "ams", copies: 1, slots: [slot(0, "PLA", "#1f8a8a", 30, "AMS 1"), slot(1, "PLA", "#F4F4EF", 18, "AMS 2")] },
  { file_id: 3, name: "clip.gcode", slot_kind: "tool", copies: 1, slots: [slot(0, "PETG", "#000000", 22)] },
];

describe("loose-file colour slots", () => {
  it("auto-groups the same material + colour across files", () => {
    const g = groupSlots(files(), {});
    expect(g.map((x) => x.key)).toEqual(["PLA #F4F4EF", "PLA #1F8A8A", "PETG #000000"]);
    expect(g[0].members.map((m) => [m.file_id, m.slot.index])).toEqual([[1, 0], [2, 1]]);
    expect(g[0].perSet).toBe(64 * 2 + 18);
    expect(groupMaterials(g[2])).toEqual(["PETG"]);
  });

  it("splits a filament into its own slot", () => {
    const f = files();
    const own = choicesFor(groupSlots(f, {}), f[1], f[1].slots[1]).find((c) => c.own);
    expect(own?.value).toBe("LID AMS 2");
    const g = groupSlots(f, { [memberId(2, 1)]: own!.value });
    expect(g.map((x) => x.key)).toEqual(["PLA #F4F4EF", "PLA #1F8A8A", "LID AMS 2", "PETG #000000"]);
    expect(g[0].members).toHaveLength(1);
  });

  it("merges a filament into another slot", () => {
    const g = groupSlots(files(), { [memberId(3, 0)]: "PLA #1F8A8A" });
    expect(g.map((x) => x.key)).toEqual(["PLA #F4F4EF", "PLA #1F8A8A"]);
    expect(groupMaterials(g[1])).toEqual(["PLA", "PETG"]);
  });

  it("colour-change files give one slot per colour", () => {
    const sign: PickedFile = {
      file_id: 9, name: "sign.gcode", slot_kind: "change", copies: 1,
      slots: [slot(0, "PLA", "#3B3B3B", 1.5, "Colour 1"), slot(1, "PLA", "#FFFFFF", 1.5, "Colour 2")],
    };
    expect(groupSlots([sign], {}).map((g) => g.key)).toEqual(["PLA #3B3B3B", "PLA #FFFFFF"]);
    expect(autoKey({ ...sign.slots[0], colour: null })).toBe("PLA");
  });
});
