/** Pure helpers for Tools → Swatches / Export (unit-tested). */
import type { ExportRow } from "../api/types/ExportRow";

/** Plate size and text geometry (crates/foreman-swatch), mm. */
export const PLATE = { w: 75, h: 40 } as const;
export const TEXT_X = 8.5;
export const BANDS = { upper: 34, lower: 25 } as const;

/** % of plate width / height for CSS placement. */
export const pctX = (mm: number) => (mm / PLATE.w) * 100;
export const pctY = (mm: number) => (mm / PLATE.h) * 100;

/** Relative luminance 0–1 of `RRGGBB` (no #). */
export function luminance(hex: string): number {
  const h = hex.replace(/^#/, "");
  if (!/^[0-9a-f]{6}$/i.test(h)) return 0.5;
  const v = parseInt(h, 16);
  return (0.299 * ((v >> 16) & 255) + 0.587 * ((v >> 8) & 255) + 0.114 * (v & 255)) / 255;
}

/** Filament matches a search: vendor, name, material, id. */
export function matches(r: ExportRow, q: string): boolean {
  const s = q.trim().toLowerCase();
  if (!s) return true;
  return [r.vendor ?? "", r.name, r.material ?? "", `#${r.id}`].join(" ").toLowerCase().includes(s);
}

export function label(r: ExportRow): string {
  return [r.vendor, r.material, r.name].filter(Boolean).join(" ");
}

/** What the Export tab's cards say. */
export function exportSummary(rows: ExportRow[], selected: Set<number>) {
  const scope = selected.size > 0 ? rows.filter((r) => selected.has(r.id)) : rows;
  const multiOrca = scope.filter((r) => r.orca.multi_colour).length;
  const withShrink = scope.filter((r) => r.orca.shrinkage_xy != null || r.orca.shrinkage_z != null).length;
  const hue = scope.filter((r) => r.hueforge_skip === null).length;
  const hueMulti = scope.filter((r) => r.hueforge_skip === "multi_colour").length;
  const missingTd = rows.filter((r) => r.hueforge_skip === "missing_td");
  return { scope: scope.length, multiOrca, withShrink, hue, hueMulti, missingTd };
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
