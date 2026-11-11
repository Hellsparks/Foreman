/** Filaments, vendors, new spools (task 4.2). */
import { request } from "./client";
import type { CloneSpool } from "./types/CloneSpool";
import type { ColourKind } from "./types/ColourKind";
import type { CreatedSpools } from "./types/CreatedSpools";
import type { CreateSpools } from "./types/CreateSpools";
import type { FilamentForm } from "./types/FilamentForm";
import type { FilamentRow } from "./types/FilamentRow";
import type { MaterialCatalogue } from "./types/MaterialCatalogue";
import type { MaterialInfo } from "./types/MaterialInfo";
import type { ModifierGroup } from "./types/ModifierGroup";
import type { SpoolFacts } from "./types/SpoolFacts";
import type { VendorForm } from "./types/VendorForm";
import type { VendorRow } from "./types/VendorRow";

export type {
  CloneSpool,
  ColourKind,
  CreatedSpools,
  CreateSpools,
  FilamentForm,
  FilamentRow,
  MaterialCatalogue,
  MaterialInfo,
  ModifierGroup,
  VendorForm,
  VendorRow,
};

export const filamentApi = {
  catalogue: () => request<MaterialCatalogue>("GET", "/api/filament/catalogue"),
  vendors: () => request<VendorRow[]>("GET", "/api/vendors"),
  createVendor: (v: VendorForm) => request<VendorRow>("POST", "/api/vendors", v),
  updateVendor: (id: number, v: VendorForm) => request<VendorRow>("PATCH", `/api/vendors/${id}`, v),
  filaments: () => request<FilamentRow[]>("GET", "/api/filaments"),
  createFilament: (f: FilamentForm) => request<FilamentRow>("POST", "/api/filaments", f),
  updateFilament: (id: number, f: FilamentForm) => request<FilamentRow>("PATCH", `/api/filaments/${id}`, f),
  createSpools: (req: CreateSpools) => request<CreatedSpools>("POST", "/api/spools", req),
  cloneSpool: (id: number, quantity: number) =>
    request<SpoolFacts[]>("POST", `/api/spools/${id}/clone`, { quantity } satisfies CloneSpool),
  locations: () => request<string[]>("GET", "/api/spools/locations"),
};

// ── Colours ───────────────────────────────────────────────────────────────

/** Min / max colours per kind: single 1, coextruded 2–3, gradient 2–6. */
export const COLOUR_RANGE: Record<ColourKind, [number, number]> = {
  single: [1, 1],
  coaxial: [2, 3],
  longitudinal: [2, 6],
};

export const COLOUR_KINDS: [ColourKind, string][] = [
  ["single", "Single"],
  ["coaxial", "Coextruded"],
  ["longitudinal", "Gradient"],
];

/** Fill colours used when a kind needs more colours than are set. */
const FILL = ["#d4af37", "#c0c0c0", "#e8505b", "#3a6ea5", "#1f8a8a", "#ffffff"];

/** Trim or pad a colour list to fit a kind. */
export function fitColours(kind: ColourKind, cols: string[]): string[] {
  const [min, max] = COLOUR_RANGE[kind];
  const out = cols.slice(0, max);
  while (out.length < min) out.push(FILL[out.length] ?? "#ffffff");
  return out;
}

/** `rrggbb` / `#RRGGBB` → `#rrggbb`, or null when it is not a colour. */
export function normHex(v: string): string | null {
  const s = v.trim().replace(/^#/, "");
  return /^[0-9a-fA-F]{6}$/.test(s) ? `#${s.toLowerCase()}` : null;
}

export function colourHelp(kind: ColourKind): string {
  if (kind === "single") return "One colour, stored as color_hex.";
  if (kind === "coaxial")
    return "Colours run side by side the whole length (dual/tri-colour silk). Stored as multi_color_hexes + direction “coaxial”.";
  return "Colour changes along the spool (rainbow / gradient). Stored as multi_color_hexes + direction “longitudinal”.";
}

// ── Modifiers + abrasive ──────────────────────────────────────────────────

/** Why each abrasive modifier is abrasive ("CF" → "carbon fibre"). */
export function abrasiveReasons(cat: MaterialCatalogue | null): Map<string, string> {
  const m = new Map<string, string>();
  for (const g of cat?.modifier_groups ?? []) for (const it of g.items) if (it.abrasive) m.set(it.name, it.why ?? "abrasive");
  return m;
}

/**
 * Toggle a modifier. The abrasive box follows the modifiers only when the set
 * changes between "has an abrasive modifier" and "has none"; otherwise the
 * user's own tick (or untick) stays.
 */
export function toggleModifier(
  mods: string[],
  mod: string,
  abrasive: boolean,
  reasons: Map<string, string>,
): { mods: string[]; abrasive: boolean } {
  const next = mods.includes(mod) ? mods.filter((m) => m !== mod) : [...mods, mod];
  const was = mods.some((m) => reasons.has(m));
  const now = next.some((m) => reasons.has(m));
  return { mods: next, abrasive: was === now ? abrasive : now };
}

/** "PETG CF HF". */
export function materialFull(material: string, mods: string[]): string {
  return [material, ...mods].filter(Boolean).join(" ");
}

// ── Form state ────────────────────────────────────────────────────────────

/** What the filament form edits (strings for inputs). */
export interface FilamentDraft {
  vendor: string;
  name: string;
  material: string;
  article: string;
  mods: string[];
  abrasive: boolean;
  kind: ColourKind;
  colours: string[];
  td: string;
  diameter: string;
  density: string;
  nozzle: string;
  bed: string;
  price: string;
  url: string;
  comment: string;
}

export function emptyDraft(mat?: MaterialInfo): FilamentDraft {
  return {
    vendor: "",
    name: "",
    material: mat?.name ?? "PLA",
    article: "",
    mods: [],
    abrasive: false,
    kind: "single",
    colours: ["#3a6ea5"],
    td: "",
    diameter: "1.75",
    density: String(mat?.density ?? 1.24),
    nozzle: String(mat?.nozzle_temp ?? 215),
    bed: String(mat?.bed_temp ?? 60),
    price: "",
    url: "",
    comment: "",
  };
}

export function draftFrom(f: FilamentRow): FilamentDraft {
  return {
    vendor: f.vendor ?? "",
    name: f.name,
    material: f.material ?? "",
    article: f.article_number ?? "",
    mods: f.modifiers,
    abrasive: f.abrasive,
    kind: f.colour_kind,
    colours: f.colours.length ? f.colours.map((c) => `#${c.toLowerCase()}`) : fitColours(f.colour_kind, []),
    td: f.hue_td != null ? String(f.hue_td) : "",
    diameter: String(f.diameter),
    density: String(f.density),
    nozzle: f.extruder_temp != null ? String(f.extruder_temp) : "",
    bed: f.bed_temp != null ? String(f.bed_temp) : "",
    price: f.price != null ? String(f.price) : "",
    url: f.url ?? "",
    comment: f.comment ?? "",
  };
}

const num = (s: string): number | null => {
  const t = s.trim().replace(",", ".");
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
};
const int = (s: string): number | null => {
  const n = num(s);
  return n == null ? null : Math.round(n);
};
const str = (s: string): string | null => (s.trim() ? s.trim() : null);

/** Draft → API body. Vendor is matched by name (case-insensitive), else created. */
export function draftToForm(d: FilamentDraft, vendors: VendorRow[]): FilamentForm {
  const v = vendors.find((x) => x.name.toLowerCase() === d.vendor.trim().toLowerCase());
  return {
    vendor_id: v?.id ?? null,
    vendor_name: v ? null : str(d.vendor),
    name: d.name.trim(),
    material: d.material,
    modifiers: d.mods,
    abrasive: d.abrasive,
    colour_kind: d.kind,
    colours: d.colours,
    hue_td: d.kind === "single" ? num(d.td) : null,
    extruder_temp: int(d.nozzle),
    bed_temp: int(d.bed),
    density: num(d.density),
    diameter: num(d.diameter),
    price: num(d.price),
    url: str(d.url),
    article_number: str(d.article),
    comment: str(d.comment),
  };
}

export { num as parseNumber };
