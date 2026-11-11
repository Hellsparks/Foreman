import { describe, expect, it } from "vitest";
import { nextSlotKey, plateStatus, progressText, renameSlot, setPlateSlot, type TemplateForm } from "./projects";

const form = (): TemplateForm => ({
  name: "Chess",
  category: null,
  notes: null,
  thumb_file_id: null,
  slots: [
    { key: "PRIMARY", hex: null, label: null, allow_swap: true },
    { key: "ACCENT", hex: null, label: null, allow_swap: true },
  ],
  plates: [{ name: "Pawns", file_id: 1, copies: 2, tools: [{ tool: 0, slot: "PRIMARY" }, { tool: 1, slot: "ACCENT" }] }],
});

describe("template editing", () => {
  it("maps and unmaps a tool", () => {
    const p = form().plates[0];
    expect(setPlateSlot(p, 1, null).tools).toEqual([{ tool: 0, slot: "PRIMARY" }]);
    expect(setPlateSlot(p, 1, "primary").tools).toEqual([{ tool: 0, slot: "PRIMARY" }, { tool: 1, slot: "PRIMARY" }]);
  });
  it("renames a slot on every plate", () => {
    const f = renameSlot(form(), "ACCENT", " dark  pieces ");
    expect(f.slots[1].key).toBe("DARK PIECES");
    expect(f.plates[0].tools[1].slot).toBe("DARK PIECES");
  });
  it("picks free slot names", () => {
    expect(nextSlotKey(form())).toBe("DETAIL");
  });
});

describe("progress counts prints", () => {
  it("summarises", () => {
    expect(progressText({ prints: 8, printed: 3, printing: 1, queued: 2, remaining: 2, secs_used: 0, grams_used: 0 })).toBe(
      "3 of 8 prints done · 1 printing · 2 queued · 2 not started",
    );
  });
  it("plate status", () => {
    const c = { copies: 3, printed: 1, printing: 1, queued: 1, remaining: 0 };
    expect(plateStatus(c, [{ queue_item_id: 1, status: "printing", printer_id: 1, where_: "HellV0" }]).text).toBe("▶ Printing · copy 2/3 · HellV0");
    expect(plateStatus({ copies: 2, printed: 2, printing: 0, queued: 0, remaining: 0 }, []).text).toBe("✓ Done ×2");
    expect(plateStatus({ copies: 2, printed: 0, printing: 0, queued: 2, remaining: 0 }, [
      { queue_item_id: 1, status: "waiting", printer_id: null, where_: "fleet" },
    ]).text).toBe("◷ Queued ×2 · fleet");
    expect(plateStatus({ copies: 2, printed: 0, printing: 0, queued: 0, remaining: 2 }, []).text).toBe("○ Not started");
  });
});
