/** Inventory, shopping list, spool locations (task 4.3). */
import { request } from "./client";
import type { Inventory } from "./types/Inventory";
import type { InventoryRow } from "./types/InventoryRow";
import type { LocationRow } from "./types/LocationRow";
import type { Locations } from "./types/Locations";
import type { RenameLocation } from "./types/RenameLocation";
import type { Reservation } from "./types/Reservation";
import type { StockStatus } from "./types/StockStatus";
import type { TrackFilament } from "./types/TrackFilament";

export type { Inventory, InventoryRow, LocationRow, Locations, Reservation, StockStatus, TrackFilament };

/** Highest target (matches the server). */
export const MAX_TARGET = 99;

export const inventoryApi = {
  get: () => request<Inventory>("GET", "/api/inventory"),
  track: (filamentId: number, t: TrackFilament) => request<Inventory>("PUT", `/api/inventory/${filamentId}`, t),
  untrack: (filamentId: number) => request<Inventory>("DELETE", `/api/inventory/${filamentId}`),
  locations: () => request<Locations>("GET", "/api/spool-locations"),
  renameLocation: (name: string, to: string) =>
    request<{ name: string; spools_moved: number }>(
      "PATCH",
      `/api/spool-locations/${encodeURIComponent(name)}`,
      { name: to } satisfies RenameLocation,
    ),
};

/** "598,00 kr" style money in the server's currency; falls back to "598.00 XYZ". */
export function money(amount: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(amount);
  } catch {
    return `${amount.toFixed(2)} ${currency}`.trim();
  }
}

export function kg(grams: number): string {
  return `${(grams / 1000).toFixed(1)} kg`;
}

/** "1 kg" / "750 g" for a filament's net weight. */
export function sizeLabel(weight: number | null): string {
  if (weight == null) return "—";
  return weight >= 1000 ? `${+(weight / 1000).toFixed(2)} kg` : `${Math.round(weight)} g`;
}

export function label(r: InventoryRow): string {
  return [r.filament.vendor, r.filament.name].filter(Boolean).join(" ");
}

/** New target / buy-below after a ± click: 0–MAX, buy-below never above target. */
export function stepTarget(t: TrackFilament, field: keyof TrackFilament, delta: number): TrackFilament {
  const next = { ...t, [field]: Math.max(0, Math.min(MAX_TARGET, t[field] + delta)) };
  if (field === "target") next.buy_below = Math.min(next.buy_below, next.target);
  if (field === "buy_below" && next.buy_below > next.target) next.target = next.buy_below;
  return next;
}

/** Store link: the filament's product URL, else its vendor's website. */
export function storeUrl(r: InventoryRow, vendorSite: (vendorId: number) => string | null | undefined): string | null {
  return r.filament.url || (r.filament.vendor_id != null ? vendorSite(r.filament.vendor_id) || null : null);
}

/** The shopping list as plain text for the clipboard. */
export function shoppingText(inv: Inventory, urlOf: (r: InventoryRow) => string | null = (r) => r.filament.url): string {
  const rows = inv.tracked.filter((r) => r.buy > 0);
  if (!rows.length) return "Shopping list: nothing to buy.";
  const lines = rows.map((r) => {
    const parts = [`${r.buy} × ${label(r)}`];
    const meta = [r.filament.material, sizeLabel(r.filament.weight)].filter((x) => x && x !== "—").join(", ");
    if (meta) parts[0] += ` (${meta})`;
    parts.push(r.line_cost != null ? money(r.line_cost, inv.currency) : "no price");
    const url = urlOf(r);
    if (url) parts.push(url);
    return `- ${parts.join(" — ")}`;
  });
  const total = `Estimated total: ${money(inv.est_cost, inv.currency)}${inv.unpriced ? ` (+${inv.unpriced} without a price)` : ""}`;
  return ["Shopping list", ...lines, total].join("\n");
}
