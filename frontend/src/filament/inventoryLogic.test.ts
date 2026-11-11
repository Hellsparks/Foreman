import { describe, expect, it } from "vitest";
import { money, shoppingText, sizeLabel, stepTarget, storeUrl, type Inventory, type InventoryRow } from "../api/inventory";
import type { FilamentRow } from "../api/filaments";

function fil(over: Partial<FilamentRow> = {}): FilamentRow {
  return {
    id: 1, name: "Galaxy Black", vendor_id: 3, vendor: "Prusament", material: "PETG", modifiers: [], abrasive: false,
    colour_kind: "single", colours: ["1d1d22"], hue_td: null, extruder_temp: null, bed_temp: null, density: 1.27,
    diameter: 1.75, price: 299, url: null, article_number: null, comment: null, spool_weight: 200, spool_count: 1,
    grams_left: 800, weight: 1000, ...over,
  };
}

function row(over: Partial<InventoryRow> = {}, f: Partial<FilamentRow> = {}): InventoryRow {
  return { filament: fil(f), target: 3, buy_below: 2, stock: 1, spools: 1, grams: 800, grams_reserved: 0, status: "buy", buy: 2, line_cost: 598, ...over };
}

function inv(rows: InventoryRow[]): Inventory {
  const shop = rows.filter((r) => r.buy > 0);
  return {
    currency: "NOK", tracked: rows, spool_count: 1, grams_on_hand: 800, grams_reserved: 0, need_count: shop.length,
    est_cost: shop.reduce((a, r) => a + (r.line_cost ?? 0), 0), unpriced: shop.filter((r) => r.line_cost == null).length, reservations: [],
  };
}

describe("stepTarget", () => {
  it("keeps buy-below at or under the target", () => {
    expect(stepTarget({ target: 2, buy_below: 2 }, "target", -1)).toEqual({ target: 1, buy_below: 1 });
    expect(stepTarget({ target: 2, buy_below: 2 }, "buy_below", 1)).toEqual({ target: 3, buy_below: 3 });
    expect(stepTarget({ target: 0, buy_below: 0 }, "target", -1)).toEqual({ target: 0, buy_below: 0 });
    expect(stepTarget({ target: 99, buy_below: 1 }, "target", 1)).toEqual({ target: 99, buy_below: 1 });
  });
});

describe("shopping list", () => {
  it("lists what to buy with costs and a total in the server's currency", () => {
    const text = shoppingText(inv([row({}, { url: "https://example.com/p" }), row({ status: "ok", buy: 0, line_cost: null }, { id: 2, name: "Spare" })]));
    const lines = text.split("\n");
    expect(lines[0]).toBe("Shopping list");
    expect(lines).toHaveLength(3);
    expect(lines[1]).toContain("2 × Prusament Galaxy Black (PETG, 1 kg)");
    expect(lines[1]).toContain(money(598, "NOK"));
    expect(lines[1]).toContain("https://example.com/p");
    expect(lines[2]).toBe(`Estimated total: ${money(598, "NOK")}`);
  });
  it("says when lines have no price", () => {
    const text = shoppingText(inv([row({ line_cost: null }, { price: null })]));
    expect(text).toContain("no price");
    expect(text).toContain("(+1 without a price)");
  });
  it("store link falls back to the vendor website", () => {
    expect(storeUrl(row({}, { url: "https://a" }), () => "https://v")).toBe("https://a");
    expect(storeUrl(row(), () => "https://v")).toBe("https://v");
    expect(storeUrl(row({}, { vendor_id: null }), () => "https://v")).toBeNull();
  });
  it("formats sizes", () => {
    expect(sizeLabel(1000)).toBe("1 kg");
    expect(sizeLabel(750)).toBe("750 g");
    expect(sizeLabel(null)).toBe("—");
  });
});
